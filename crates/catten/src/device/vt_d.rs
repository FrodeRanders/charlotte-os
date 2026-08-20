//! Intel VT-d DMA remapping.
//!
//! One kernel-owned second-level translation domain is created per delegated
//! PCI requester. Drivers receive only a `DmaDomain` capability and IOVAs;
//! root/context tables, page tables, and physical addresses remain
//! kernel-private. This is the x86_64 counterpart of the Arm SMMUv3 driver and
//! replaces the temporary direct (identity) DMA model, whose assumption of
//! physically-contiguous DMA buffers fragments under concurrent boot load.
//!
//! The driver programs a single DMA remapping unit (the first DRHD the DMAR
//! table describes): it installs a root table whose entries all fault, enables
//! translation, and then carves out one context entry and page-table hierarchy
//! per domain. IOVAs are allocated from a per-domain monotonic cursor, so
//! non-contiguous physical frames are transparently re-mapped into a
//! contiguous IOVA range.

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};
use core::{
    ptr,
    sync::atomic::{
        AtomicU32,
        AtomicU64,
        AtomicUsize,
        Ordering,
    },
};

use spin::LazyLock;

use crate::{
    cpu::{
        isa::interface::memory::AddressSpaceInterface,
        multiprocessor::spin::mutex::Mutex,
    },
    memory::{
        AddressSpace,
        PHYSICAL_FRAME_ALLOCATOR,
        object::{
            self,
            DmaPin,
        },
        physical::{
            PAddr,
            PhysicalAddress,
        },
    },
};

const PAGE_SIZE: usize = 4096;
// Keep the default aperture usable by devices whose DMA descriptors are
// nominally 64-bit but whose queue transport still has an effective 32-bit
// address limit. This is an I/O virtual address, not exposed physical memory.
const IOVA_START: u64 = 0x1000_0000;
// Physical address field mask (bits 51:12). All remapping structure pointers
// and page-table entries share it.
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

// Register offsets (bytes) from the remapping unit's register base.
const VER: usize = 0x00;
const CAP: usize = 0x08;
const GCMD: usize = 0x18;
const GSTS: usize = 0x1c;
const RTADDR: usize = 0x20;
const CCMD: usize = 0x28;
const FSTS: usize = 0x34;
const FECTL: usize = 0x38;
const FEDATA: usize = 0x3c;
const FEADDR: usize = 0x40;
const FEUADDR: usize = 0x44;

// Global command register bits.
const GCMD_SRTP: u32 = 1 << 30;
const GCMD_TE: u32 = 1 << 31;
// Global status register bits.
const GSTS_RTPS: u32 = 1 << 30;
const GSTS_TES: u32 = 1 << 31;
// Context command register bits.
const CCMD_ICC: u64 = 1 << 63;
const CCMD_CIRG: u64 = 1 << 62;
// IOTLB invalidation register bits (offset 0x80).
const IOTLB_INV: usize = 0x80;
const IOTLB_IVA: u64 = 1 << 63;
const IOTLB_IIRG: u64 = 1 << 60;

// Fault status register bits (bits 0..7 are the latched fault/error sources).
const FSTS_FAULT_MASK: u32 = 0xff;

// Context entry adjusted guest address width (AW) encodings.
const CTX_AW_39BIT: u64 = 1;
const CTX_AW_48BIT: u64 = 2;
const CTX_AW_57BIT: u64 = 3;

#[derive(Debug)]
pub enum Error {
    Unsupported,
    InvalidStream,
    StreamInUse,
    InvalidDirection,
    Memory,
    OutOfIova,
    MapFailed,
    UnknownDomain,
    UnknownMapping,
    HardwareTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Direction(u32);

impl Direction {
    pub const DEVICE_READ: Self = Self(1);
    pub const DEVICE_WRITE: Self = Self(2);

    pub fn from_bits(bits: u32) -> Result<Self, Error> {
        if bits != 0 && bits & !3 == 0 {
            Ok(Self(bits))
        } else {
            Err(Error::InvalidDirection)
        }
    }

    fn device_writes(self) -> bool {
        self.0 & Self::DEVICE_WRITE.0 != 0
    }
}

struct Mapping {
    pin: DmaPin,
    pages: usize,
}

struct Domain {
    source_id: u16,
    agaw: u8,
    levels: usize,
    root: PAddr,
    table_frames: Vec<PAddr>,
    next_iova: u64,
    mappings: BTreeMap<u64, Mapping>,
}

impl Domain {
    fn new(source_id: u16, agaw: u8, msi_address: Option<u64>) -> Result<Self, Error> {
        let root = alloc_zeroed_frame()?;
        let levels = ((agaw - 12) / 9) as usize;
        let mut domain = Self {
            source_id,
            agaw,
            levels,
            root,
            table_frames: alloc::vec![root],
            next_iova: IOVA_START,
            mappings: BTreeMap::new(),
        };
        // The device raises MSI/MSI-X by writing the LAPIC message address; the
        // remapping unit must identity-map that page so the transaction reaches
        // the interrupt controller rather than faulting.
        if let Some(address) = msi_address {
            let page = address & !(PAGE_SIZE as u64 - 1);
            domain.map_page(page, PAddr::from(page), true)?;
        }
        Ok(domain)
    }

    fn map_page(&mut self, iova: u64, frame: PAddr, writable: bool) -> Result<(), Error> {
        let mut parent = self.root;
        for level in 0..self.levels - 1 {
            let shift = self.agaw as u64 - 9 * (level as u64 + 1);
            let index = ((iova >> shift) & 0x1ff) as usize;
            let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
            let value = unsafe { entry.read_volatile() };
            if value & 1 == 0 {
                let next = alloc_zeroed_frame()?;
                self.table_frames.push(next);
                unsafe { entry.write_volatile((u64::from(next) & ADDR_MASK) | 3) };
            }
            parent = PAddr::from(unsafe { entry.read_volatile() } & ADDR_MASK);
        }
        let index = ((iova >> 12) & 0x1ff) as usize;
        let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
        if unsafe { entry.read_volatile() } & 1 != 0 {
            return Err(Error::MapFailed);
        }
        unsafe { entry.write_volatile((u64::from(frame) & ADDR_MASK) | if writable { 3 } else { 1 }) };
        Ok(())
    }

    fn clear_page(&mut self, iova: u64) {
        let mut parent = self.root;
        for level in 0..self.levels - 1 {
            let shift = self.agaw as u64 - 9 * (level as u64 + 1);
            let index = ((iova >> shift) & 0x1ff) as usize;
            let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
            let value = unsafe { entry.read_volatile() };
            if value & 1 == 0 {
                return;
            }
            parent = PAddr::from(value & ADDR_MASK);
        }
        let index = ((iova >> 12) & 0x1ff) as usize;
        let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
        unsafe { entry.write_volatile(0) };
    }

    fn map(&mut self, pin: DmaPin, direction: Direction) -> Result<u64, (Error, DmaPin)> {
        let pages = pin.frames().len();
        let Some(bytes) = (pages as u64).checked_mul(PAGE_SIZE as u64) else {
            return Err((Error::OutOfIova, pin));
        };
        let iova = self.next_iova;
        let Some(next_iova) = self
            .next_iova
            .checked_add(bytes)
            .and_then(|next| next.checked_add(PAGE_SIZE as u64 - 1))
            .map(|next| next & !(PAGE_SIZE as u64 - 1))
        else {
            return Err((Error::OutOfIova, pin));
        };
        let writable = direction.device_writes();
        for (index, frame) in pin.frames().iter().copied().enumerate() {
            let address = iova + (index * PAGE_SIZE) as u64;
            if let Err(error) = self.map_page(address, frame, writable) {
                for rollback_index in 0..index {
                    self.clear_page(iova + (rollback_index * PAGE_SIZE) as u64);
                }
                return Err((error, pin));
            }
        }
        self.next_iova = next_iova;
        self.mappings.insert(
            iova,
            Mapping {
                pin,
                pages,
            },
        );
        Ok(iova)
    }

    fn clear_mapping(&mut self, iova: u64) -> Result<Mapping, Error> {
        let mapping = self.mappings.remove(&iova).ok_or(Error::UnknownMapping)?;
        for index in 0..mapping.pages {
            self.clear_page(iova + (index * PAGE_SIZE) as u64);
        }
        Ok(mapping)
    }
}

struct Unit {
    base: usize,
    agaw: u8,
    root_table: PAddr,
    context_tables: BTreeMap<u16, PAddr>,
    next_domain: u64,
    domains: BTreeMap<u64, Domain>,
    sources: BTreeMap<u16, u64>,
}

impl Unit {
    fn set_root_entry(&self, bus: u16, context_table: PAddr) {
        let root = unsafe { self.root_table.into_hhdm_mut::<u64>() };
        let entry = unsafe { root.add(bus as usize * 2) };
        unsafe {
            entry.write_volatile((u64::from(context_table) & ADDR_MASK) | 1);
            entry.add(1).write_volatile(0);
        }
    }

    fn write_context_entry(&self, bus: u16, devfunc: u8, root: PAddr, aw: u64) {
        let context_table = self.context_tables[&bus];
        let entry = unsafe { context_table.into_hhdm_mut::<u64>().add(devfunc as usize * 2) };
        // 128-bit context entry: lo = present + second-stage page table
        // pointer, hi = adjusted guest address width (AW) in bits 2:0.
        unsafe {
            entry.write_volatile((u64::from(root) & ADDR_MASK) | 1);
            entry.add(1).write_volatile(aw);
        }
    }

    fn flush_context_cache(&self) -> Result<(), Error> {
        write64(self.base, CCMD, CCMD_ICC | CCMD_CIRG);
        for _ in 0..1_000_000 {
            if read64(self.base, CCMD) & CCMD_ICC == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::HardwareTimeout)
    }

    fn flush_iotlb(&self) -> Result<(), Error> {
        write64(self.base, IOTLB_INV, IOTLB_IVA | IOTLB_IIRG);
        for _ in 0..1_000_000 {
            if read64(self.base, IOTLB_INV) & IOTLB_IVA == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::HardwareTimeout)
    }
}

static UNIT: LazyLock<Mutex<Option<Unit>>> = LazyLock::new(|| Mutex::new(None));
static IRQ_MMIO: AtomicUsize = AtomicUsize::new(0);
static FAULT_COUNT: AtomicU64 = AtomicU64::new(0);
static FAULT_INTID: AtomicU32 = AtomicU32::new(u32::MAX);
static FRCD_OFFSET: AtomicUsize = AtomicUsize::new(0);

fn read32(base: usize, offset: usize) -> u32 {
    unsafe { ptr::read_volatile((base + offset) as *const u32) }
}

fn write32(base: usize, offset: usize, value: u32) {
    unsafe { ptr::write_volatile((base + offset) as *mut u32, value) }
}

fn read64(base: usize, offset: usize) -> u64 {
    unsafe { ptr::read_volatile((base + offset) as *const u64) }
}

fn write64(base: usize, offset: usize, value: u64) {
    unsafe { ptr::write_volatile((base + offset) as *mut u64, value) }
}

fn wait_gsts(base: usize, mask: u32) -> Result<(), Error> {
    for _ in 0..1_000_000 {
        if read32(base, GSTS) & mask == mask {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(Error::HardwareTimeout)
}

fn wait_gsts_clear(base: usize, mask: u32) -> Result<(), Error> {
    for _ in 0..1_000_000 {
        if read32(base, GSTS) & mask == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(Error::HardwareTimeout)
}

fn alloc_zeroed_frame() -> Result<PAddr, Error> {
    let frame = PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().map_err(|_| Error::MapFailed)?;
    unsafe { ptr::write_bytes(frame.into_hhdm_mut::<u8>(), 0, PAGE_SIZE) };
    Ok(frame)
}

fn supported_agaw(cap: u64) -> Option<u8> {
    let sagaw = (cap >> 10) & 0x7;
    if sagaw & (1 << 1) != 0 {
        Some(48)
    } else if sagaw & 1 != 0 {
        Some(39)
    } else if sagaw & (1 << 2) != 0 {
        Some(57)
    } else {
        None
    }
}

fn agaw_aw(agaw: u8) -> u64 {
    match agaw {
        39 => CTX_AW_39BIT,
        48 => CTX_AW_48BIT,
        _ => CTX_AW_57BIT,
    }
}

fn initialize(config: crate::environment::acpi::sdt::dmar::DmarConfig) -> Result<Unit, Error> {
    let mut current = AddressSpace::get_current();
    current.map_mmio_region(config.base, 0x1000).map_err(|_| Error::MapFailed)?;
    let base = unsafe { PAddr::from(config.base as u64).into_hhdm_ptr::<u8>() } as usize;

    let version = read32(base, VER);
    if version & 0xff < 0x10 {
        return Err(Error::Unsupported);
    }
    let cap = read64(base, CAP);
    let agaw = supported_agaw(cap).ok_or(Error::Unsupported)?;
    // The fault recording register (FRCD) sits at CAP.FRO * 16 bytes; extend
    // the MMIO mapping past the first page if it lands beyond it.
    let frcd_offset = (((cap >> 24) & 0xfff) as usize) * 16;
    FRCD_OFFSET.store(frcd_offset, Ordering::Release);
    if frcd_offset + 0x10 > 0x1000 {
        current
            .map_mmio_region(config.base, frcd_offset + 0x10)
            .map_err(|_| Error::MapFailed)?;
    }

    let root_table = alloc_zeroed_frame()?;

    // Disable translation while installing the root table, then publish it and
    // re-enable translation. The firmware may leave the unit in an arbitrary
    // state, so write the control register rather than read-modify-writing it.
    write32(base, GCMD, 0);
    wait_gsts_clear(base, GSTS_TES)?;
    write64(base, RTADDR, u64::from(root_table) & ADDR_MASK);
    write32(base, GCMD, GCMD_SRTP);
    wait_gsts(base, GSTS_RTPS)?;
    write32(base, GCMD, GCMD_SRTP | GCMD_TE);
    wait_gsts(base, GSTS_TES)?;

    IRQ_MMIO.store(base, Ordering::Release);
    // Route fault events through an MSI. Program the fault-event message
    // address/data registers, then unmask the fault interrupt.
    match crate::device::allocate_msi(0) {
        Some(msi) => {
            write32(base, FEADDR, msi.address as u32);
            write32(base, FEUADDR, (msi.address >> 32) as u32);
            write32(base, FEDATA, msi.data);
            write32(base, FECTL, 0);
            FAULT_INTID.store(msi.intid, Ordering::Release);
            crate::logln!("[vtd] fault MSI enabled (intid={})", msi.intid);
        }
        None => {
            crate::logln!("[vtd] fault MSI unavailable; faults detected via polling");
        }
    }

    crate::logln!(
        "[vtd] enabled VT-d at {:#x}: {} bit AGAW, {} translation levels",
        config.base,
        agaw,
        ((agaw - 12) / 9)
    );

    Ok(Unit {
        base,
        agaw,
        root_table,
        context_tables: BTreeMap::new(),
        next_domain: 1,
        domains: BTreeMap::new(),
        sources: BTreeMap::new(),
    })
}

fn with_unit<R>(f: impl FnOnce(&mut Unit) -> Result<R, Error>) -> Result<R, Error> {
    let mut guard = UNIT.lock();
    if guard.is_none() {
        let config =
            crate::environment::acpi::sdt::dmar::discover_vtd().ok_or(Error::Unsupported)?;
        *guard = Some(initialize(config)?);
    }
    f(guard.as_mut().expect("VT-d unit initialized"))
}

/// Initialize the platform DMA remapping unit before driver domains begin
/// competing for physical memory. Mirroring the SMMU driver, deferring this to
/// the first DMA-domain request would make the large root-table allocation
/// depend on how earlier boot allocations fragmented physical memory.
pub fn initialize_early() -> Result<(), Error> {
    with_unit(|_| Ok(()))
}

pub fn stream_id(requester_id: u32) -> Result<u32, Error> {
    crate::environment::acpi::sdt::dmar::discover_vtd().ok_or(Error::Unsupported)?;
    Ok(requester_id & 0xffff)
}

pub fn create_domain(sid: u32, msi_address: Option<u64>) -> Result<u64, Error> {
    with_unit(|unit| {
        let source_id = sid as u16;
        if unit.sources.contains_key(&source_id) {
            return Err(Error::StreamInUse);
        }
        let id = unit.next_domain;
        unit.next_domain = unit.next_domain.checked_add(1).ok_or(Error::MapFailed)?;
        let domain = Domain::new(source_id, unit.agaw, msi_address)?;

        let bus = source_id >> 8;
        if !unit.context_tables.contains_key(&bus) {
            let context_table = alloc_zeroed_frame()?;
            unit.set_root_entry(bus, context_table);
            unit.context_tables.insert(bus, context_table);
        }
        let devfunc = (source_id & 0xff) as u8;
        let root = domain.root;
        unit.domains.insert(id, domain);
        unit.sources.insert(source_id, id);
        unit.write_context_entry(bus, devfunc, root, agaw_aw(unit.agaw));
        unit.flush_context_cache()?;
        Ok(id)
    })
}

pub fn map(
    domain_id: u64,
    caller: crate::memory::AddressSpaceId,
    memory_cap: u64,
    direction: Direction,
    exclusive: bool,
) -> Result<u64, Error> {
    let pin = object::pin_for_dma(
        caller,
        memory_cap,
        direction.0 & Direction::DEVICE_READ.0 != 0,
        direction.device_writes(),
        exclusive,
    )
    .map_err(|_| Error::Memory)?;
    let mut pending_pin = Some(pin);
    let result = with_unit(|unit| {
        let iova = {
            let domain = unit.domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?;
            let pin = pending_pin.take().expect("DMA pin consumed twice");
            match domain.map(pin, direction) {
                Ok(iova) => iova,
                Err((error, pin)) => {
                    pending_pin = Some(pin);
                    return Err(error);
                }
            }
        };
        // If the hardware never acknowledges the invalidation the mapping stays
        // installed until domain destruction, but returning no IOVA keeps the
        // driver from touching the possibly-stale translation.
        unit.flush_iotlb()?;
        Ok(iova)
    });
    if let Some(pin) = pending_pin {
        object::unpin_dma(pin);
    }
    result
}

pub fn unmap(domain_id: u64, iova: u64) -> Result<(), Error> {
    let mapping = with_unit(|unit| {
        let domain = unit.domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?;
        let mapping = domain.clear_mapping(iova)?;
        unit.flush_iotlb()?;
        Ok(mapping)
    })?;
    object::unpin_dma(mapping.pin);
    Ok(())
}

pub fn destroy_domain(domain_id: u64) -> Result<(), Error> {
    let mappings = with_unit(|unit| {
        let Some(domain) = unit.domains.get(&domain_id) else {
            return Ok(Vec::new());
        };
        let source_id = domain.source_id;
        // Quarantine the source id before releasing mappings or their pins:
        // write a non-present context entry and wait for the hardware to
        // acknowledge before freeing frames a device may still translate.
        let bus = source_id >> 8;
        let devfunc = (source_id & 0xff) as u8;
        unit.write_context_entry(bus, devfunc, PAddr::from(0u64), 0);
        unit.flush_context_cache()?;
        let mut domain = unit.domains.remove(&domain_id).expect("VT-d domain disappeared");
        let mappings = core::mem::take(&mut domain.mappings).into_values().collect::<Vec<_>>();
        unit.sources.remove(&source_id);
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        for frame in domain.table_frames {
            let _ = allocator.deallocate_frame(frame);
        }
        Ok(mappings)
    })?;
    for mapping in mappings {
        object::unpin_dma(mapping.pin);
    }
    Ok(())
}

/// Handle a VT-d fault interrupt. The fault event is delivered as an MSI whose
/// synthetic intid was captured during initialization; any other intid is left
/// to the device-capability layer.
pub fn handle_interrupt(intid: u32) -> bool {
    if intid != FAULT_INTID.load(Ordering::Acquire) {
        return false;
    }
    let base = IRQ_MMIO.load(Ordering::Acquire);
    if base == 0 {
        return true;
    }
    let fsts = read32(base, FSTS);
    if fsts & FSTS_FAULT_MASK != 0 {
        FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
        let frcd = FRCD_OFFSET.load(Ordering::Acquire);
        let fault_info = read64(base, frcd);
        let fault_meta = read64(base, frcd + 8);
        let fault_address = fault_info & 0xffff_ffff_ffff_f000;
        crate::early_logln!(
            "[vtd] DMA fault: fsts={:#x} sid={:#x} reason={:#x} address={:#x}",
            fsts,
            fault_meta & 0xffff,
            (fault_meta >> 32) & 0xff,
            fault_address
        );
        // The fault status bits are write-1-to-clear.
        write32(base, FSTS, fsts & FSTS_FAULT_MASK);
    }
    true
}

/// Number of DMA translation faults observed since boot. When a fault MSI
/// route is installed the counter is maintained by [`handle_interrupt`];
/// otherwise latched faults are drained here.
pub fn fault_count() -> u64 {
    if FAULT_INTID.load(Ordering::Acquire) != u32::MAX {
        return FAULT_COUNT.load(Ordering::Acquire);
    }
    let base = IRQ_MMIO.load(Ordering::Acquire);
    if base == 0 {
        return FAULT_COUNT.load(Ordering::Acquire);
    }
    let fsts = read32(base, FSTS);
    if fsts & FSTS_FAULT_MASK != 0 {
        write32(base, FSTS, fsts & FSTS_FAULT_MASK);
        return FAULT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    }
    FAULT_COUNT.load(Ordering::Acquire)
}

/// Number of fault events the hardware has latched but not yet consumed.
pub fn pending_fault_events() -> u32 {
    let base = IRQ_MMIO.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    ((read32(base, FSTS) >> 1) & 1) as u32
}
