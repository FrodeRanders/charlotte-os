//! Arm SMMUv3 DMA isolation.
//!
//! One kernel-owned stage-1 translation context is created per delegated PCI
//! requester stream. Drivers receive only a `DmaDomain` capability and IOVAs;
//! stream tables, context descriptors, page tables, invalidation queues, and
//! physical addresses remain kernel-private.

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};
use core::{
    arch::asm,
    ptr,
    sync::atomic::{
        AtomicU32,
        AtomicU64,
        AtomicUsize,
        Ordering,
    },
};

use spin::LazyLock;

use super::dma_tables::{
    Scope,
    Tables,
};
use crate::{
    cpu::{
        isa::{
            interface::memory::AddressSpaceInterface,
            memory::paging::{
                PageTable,
                descriptor::{
                    Descriptor,
                    MAIR_IDX_NORMAL,
                },
            },
        },
        multiprocessor::spin::mutex::Mutex,
    },
    environment::acpi::sdt::iort::SmmuV3Config,
    memory::{
        AddressSpace,
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
const STE_SIZE: usize = 64;
const QUEUE_ENTRIES: u32 = 256;
const EVENT_ENTRIES: u32 = 128;
// Keep the default aperture usable by devices whose DMA descriptors are
// nominally 64-bit but whose queue transport still has an effective 32-bit
// address limit. This is an I/O virtual address, not exposed physical memory;
// each requester retains its own isolated stage-1 domain.
const IOVA_START: u64 = 0x1000_0000;

const IDR0: usize = 0x000;
const IDR1: usize = 0x004;
const IDR5: usize = 0x014;
const CR0: usize = 0x020;
const CR0_ACK: usize = 0x024;
const CR1: usize = 0x028;
const CR2: usize = 0x02c;
const IRQ_CTRL: usize = 0x050;
const IRQ_CTRL_ACK: usize = 0x054;
const GERROR: usize = 0x060;
const GERRORN: usize = 0x064;
const STRTAB_BASE: usize = 0x080;
const STRTAB_BASE_CFG: usize = 0x088;
const CMDQ_BASE: usize = 0x090;
const CMDQ_PROD: usize = 0x098;
const CMDQ_CONS: usize = 0x09c;
const EVTQ_BASE: usize = 0x0a0;
const EVTQ_PROD: usize = 0x0a8;
const EVTQ_CONS: usize = 0x0ac;

const CR0_SMMUEN: u32 = 1 << 0;
const CR0_EVTQEN: u32 = 1 << 2;
const CR0_CMDQEN: u32 = 1 << 3;
const IRQ_EVTQ: u32 = 1 << 2;
const IRQ_GERROR: u32 = 1 << 0;

const STE_VALID: u64 = 1;
const STE_CFG_ABORT: u64 = 0;
const STE_CFG_S1: u64 = 5;
const CD_VALID: u64 = 1 << 31;
const CD_AA64: u64 = 1 << 41;

static IRQ_MMIO: AtomicUsize = AtomicUsize::new(0);
static IRQ_EVENTQ: AtomicU64 = AtomicU64::new(0);
static IRQ_EVENT_CONS: AtomicU32 = AtomicU32::new(0);
static IRQ_EVENT_INTID: AtomicU32 = AtomicU32::new(0);
static IRQ_GERROR_INTID: AtomicU32 = AtomicU32::new(0);
static FAULT_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    InvalidStream,
    StreamInUse,
    InvalidDirection,
    Memory,
    OutOfIova,
    MapFailed,
    AlreadyMapped,
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
    retiring: bool,
    sid: u32,
    asid: u16,
    root: PAddr,
    tables: Tables,
    l3_tables: BTreeMap<u64, PAddr>,
    next_iova: u64,
    mappings: BTreeMap<u64, Mapping>,
    cd: PAddr,
    quarantined_pins: Vec<DmaPin>,
}

struct Smmu {
    config: SmmuV3Config,
    sid_bits: u8,
    oas: u8,
    strtab: PAddr,
    _tables: Tables,
    cmdq: PAddr,
    cmd_prod: u32,
    next_domain: u64,
    domains: BTreeMap<u64, Domain>,
    streams: BTreeMap<u32, u64>,
}

static SMMU: LazyLock<Mutex<Option<Smmu>>> = LazyLock::new(|| Mutex::new(None));

fn read32(base: usize, offset: usize) -> u32 {
    unsafe { ptr::read_volatile((base + offset) as *const u32) }
}

fn write32(base: usize, offset: usize, value: u32) {
    unsafe { ptr::write_volatile((base + offset) as *mut u32, value) }
}

fn write64(base: usize, offset: usize, value: u64) {
    unsafe { ptr::write_volatile((base + offset) as *mut u64, value) }
}

fn barrier() {
    unsafe { asm!("dsb oshst", options(nostack, preserves_flags)) }
}

fn wait_ack(base: usize, register: usize, value: u32) -> Result<(), Error> {
    for _ in 0..1_000_000 {
        if read32(base, register) == value {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(Error::HardwareTimeout)
}

fn set_descriptor(table: PAddr, index: usize, descriptor: Descriptor) {
    let table = unsafe { table.into_hhdm_mut::<PageTable>() };
    unsafe { (*table)[index] = descriptor };
}

impl Domain {
    fn new(asid: u16, sid: u32, oas: u8, msi_address: Option<u64>) -> Result<Self, Error> {
        let (tables, (root, cd)) = Tables::prepare_unpublished(Scope::Domain, |tables| {
            Ok((tables.allocate_frame()?, tables.allocate_frame()?))
        })?;
        let cd_words = unsafe { cd.into_hhdm_mut::<u64>() };
        // 48-bit IOVA, 4 KiB granule, WB/WA walks, inner-shareable, TTBR1
        // disabled, implementation output-address size inherited from IDR5.
        let tcr = 16u64
            | (1 << 8)
            | (1 << 10)
            | (3 << 12)
            | (1 << 30)
            | ((oas as u64) << 32)
            | CD_VALID
            | CD_AA64
            | (1 << 45)
            | (1 << 46)
            | (1 << 47)
            | ((asid as u64) << 48);
        unsafe {
            ptr::write_volatile(cd_words, tcr);
            ptr::write_volatile(cd_words.add(1), u64::from(root));
            ptr::write_volatile(cd_words.add(3), 0xff);
        }
        let mut domain = Self {
            retiring: false,
            sid,
            asid,
            root,
            tables,
            l3_tables: BTreeMap::new(),
            next_iova: IOVA_START,
            mappings: BTreeMap::new(),
            cd,
            quarantined_pins: Vec::new(),
        };
        if let Some(address) = msi_address {
            let page = address & !(PAGE_SIZE as u64 - 1);
            if let Err(error) = domain.map_page(page, PAddr::from(page), true) {
                // Neither the private CD nor its MSI prefix entered an STE.
                let _ = domain.tables.cancel_unpublished();
                return Err(error);
            }
        }
        Ok(domain)
    }

    fn ensure_l3(&mut self, iova: u64) -> Result<PAddr, Error> {
        let key = iova >> 21;
        if let Some(table) = self.l3_tables.get(&key) {
            return Ok(*table);
        }
        let indices = [
            ((iova >> 39) & 0x1ff) as usize,
            ((iova >> 30) & 0x1ff) as usize,
            ((iova >> 21) & 0x1ff) as usize,
        ];
        let mut parent = self.root;
        for index in indices {
            let descriptor = unsafe { (*parent.into_hhdm_ptr::<PageTable>())[index] };
            parent = if descriptor.is_valid() {
                descriptor.frame()
            } else {
                let next = self.tables.allocate_frame()?;
                barrier();
                set_descriptor(parent, index, Descriptor::new_table(next));
                next
            };
        }
        self.l3_tables.insert(key, parent);
        Ok(parent)
    }

    fn map(&mut self, pin: DmaPin, direction: Direction) -> Result<u64, (Error, DmaPin, bool)> {
        if self.retiring {
            return Err((Error::UnknownDomain, pin, false));
        }
        if self.mappings.values().any(|mapping| mapping.pin.object_id() == pin.object_id())
            || self.quarantined_pins.iter().any(|held| held.object_id() == pin.object_id())
        {
            return Err((Error::AlreadyMapped, pin, false));
        }
        // Prepare enough quarantine capacity for every live mapping, before leaves.
        if self.quarantined_pins.try_reserve(self.mappings.len() + 1).is_err() {
            return Err((Error::Memory, pin, false));
        }
        let pages = pin.frames().len();
        let Some(bytes) = (pages as u64).checked_mul(PAGE_SIZE as u64) else {
            return Err((Error::OutOfIova, pin, false));
        };
        let iova = self.next_iova;
        let Some(next_iova) = self
            .next_iova
            .checked_add(bytes)
            .and_then(|next| next.checked_add(PAGE_SIZE as u64 - 1))
            .map(|next| next & !(PAGE_SIZE as u64 - 1))
        else {
            return Err((Error::OutOfIova, pin, false));
        };
        let writable = direction.device_writes();
        for (index, frame) in pin.frames().iter().copied().enumerate() {
            let address = iova + (index * PAGE_SIZE) as u64;
            if let Err(error) = self.map_page(address, frame, writable) {
                for rollback_index in 0..index {
                    let rollback_address = iova + (rollback_index * PAGE_SIZE) as u64;
                    let l3 = self.l3_tables[&(rollback_address >> 21)];
                    let slot = ((rollback_address >> 12) & 0x1ff) as usize;
                    unsafe { (*l3.into_hhdm_mut::<PageTable>())[slot].clear() };
                }
                barrier();
                return Err((error, pin, index != 0));
            }
        }
        barrier();
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

    fn map_page(&mut self, iova: u64, frame: PAddr, writable: bool) -> Result<(), Error> {
        let l3 = self.ensure_l3(iova)?;
        let slot = ((iova >> 12) & 0x1ff) as usize;
        let current = unsafe { (*l3.into_hhdm_ptr::<PageTable>())[slot] };
        if current.is_valid() {
            return Err(Error::MapFailed);
        }
        set_descriptor(
            l3,
            slot,
            Descriptor::new_leaf(frame, writable, false, true, MAIR_IDX_NORMAL, true),
        );
        Ok(())
    }

    fn clear_mapping(&mut self, iova: u64) -> Result<Mapping, Error> {
        if self.retiring {
            return Err(Error::UnknownDomain);
        }
        let mapping = self.mappings.remove(&iova).ok_or(Error::UnknownMapping)?;
        for index in 0..mapping.pages {
            let address = iova + (index * PAGE_SIZE) as u64;
            let l3 = *self
                .l3_tables
                .get(&(address >> 21))
                .expect("tracked SMMU mapping lost its page table");
            let slot = ((address >> 12) & 0x1ff) as usize;
            unsafe { (*l3.into_hhdm_mut::<PageTable>())[slot].clear() };
        }
        barrier();
        Ok(mapping)
    }
}

impl Smmu {
    fn issue(&mut self, command: [u64; 2]) -> Result<(), Error> {
        if !charlotte_lifecycle::iommu::smmu_queue_has_space(
            self.cmd_prod,
            read32(self.config.base, CMDQ_CONS),
            QUEUE_ENTRIES,
        ) {
            // Preserve timed-out/unconsumed commands and backing on retry.
            return Err(Error::HardwareTimeout);
        }
        let slot = (self.cmd_prod & (QUEUE_ENTRIES - 1)) as usize;
        let entry = unsafe { self.cmdq.into_hhdm_mut::<u64>().add(slot * 2) };
        unsafe {
            ptr::write_volatile(entry, command[0]);
            ptr::write_volatile(entry.add(1), command[1]);
        }
        barrier();
        self.cmd_prod = (self.cmd_prod + 1) & (QUEUE_ENTRIES * 2 - 1);
        write32(self.config.base, CMDQ_PROD, self.cmd_prod);
        Ok(())
    }

    fn sync(&mut self) -> Result<(), Error> {
        self.issue([0x46, 0])?;
        for _ in 0..1_000_000 {
            if read32(self.config.base, CMDQ_CONS) == self.cmd_prod {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::HardwareTimeout)
    }

    fn invalidate_ste(&mut self, sid: u32) -> Result<(), Error> {
        self.issue([0x03 | ((sid as u64) << 32), 1])?;
        self.sync()
    }

    fn invalidate_asid(&mut self, asid: u16) -> Result<(), Error> {
        self.issue([0x11 | ((asid as u64) << 48), 0])?;
        self.sync()
    }

    fn write_ste(&mut self, sid: u32, cd: Option<PAddr>) -> Result<(), Error> {
        if sid >= (1u32 << self.sid_bits) {
            return Err(Error::InvalidStream);
        }
        let ste = unsafe { self.strtab.into_hhdm_mut::<u64>().add(sid as usize * 8) };
        let first = match cd {
            Some(cd) => STE_VALID | (STE_CFG_S1 << 1) | (u64::from(cd) & 0x000f_ffff_ffff_ffc0),
            None => STE_VALID | (STE_CFG_ABORT << 1),
        };
        if cd.is_none() {
            // Publish abort before changing any formerly live secondary word.
            // Leave the old metadata intact until configuration maintenance
            // completes; teardown separately invalidates the original ASID.
            unsafe { ptr::write_volatile(ste, first) };
            barrier();
            return self.invalidate_ste(sid);
        }
        unsafe {
            for index in 1..8 {
                ptr::write_volatile(ste.add(index), 0);
            }
            if cd.is_some() {
                // SSID 0, WB/WA table walks, inner-shareable.
                ptr::write_volatile(ste.add(1), 2 | (1 << 2) | (1 << 4) | (3 << 6));
            }
            barrier();
            ptr::write_volatile(ste, first);
        }
        barrier();
        self.invalidate_ste(sid)
    }
}

fn initialize(mut config: SmmuV3Config) -> Result<Smmu, Error> {
    if !config.coherent {
        // Non-coherent table walks require explicit cache maintenance, which
        // this first implementation intentionally does not pretend to offer.
        return Err(Error::Unsupported);
    }
    let mut current = AddressSpace::get_current();
    current.map_mmio_region(config.base, 0x2_0000).map_err(|_| Error::MapFailed)?;
    let physical_base = config.base;
    config.base = unsafe { PAddr::from(physical_base as u64).into_hhdm_ptr::<u8>() } as usize;
    let idr0 = read32(config.base, IDR0);
    let idr1 = read32(config.base, IDR1);
    let idr5 = read32(config.base, IDR5);
    if idr0 & (1 << 1) == 0 || idr5 & (1 << 4) == 0 {
        return Err(Error::Unsupported);
    }
    let sid_bits = (idr1 & 0x3f) as u8;
    if sid_bits == 0 || sid_bits > 16 || idr1 & ((1 << 30) | (1 << 29)) != 0 {
        return Err(Error::Unsupported);
    }
    let entries = 1usize << sid_bits;
    let strtab_bytes = entries.checked_mul(STE_SIZE).ok_or(Error::MapFailed)?;
    let strtab_frames = strtab_bytes.div_ceil(PAGE_SIZE);
    let (mut tables, (strtab, cmdq, eventq)) =
        Tables::prepare_unpublished(Scope::Unit, |tables| {
            let strtab =
                tables.allocate(strtab_frames, strtab_bytes.next_power_of_two().max(PAGE_SIZE))?;
            for sid in 0..entries {
                let ste = unsafe { strtab.into_hhdm_mut::<u64>().add(sid * 8) };
                unsafe { ptr::write_volatile(ste, STE_VALID) };
            }
            let cmdq = tables.allocate_frame()?;
            let eventq = tables.allocate_frame()?;
            write32(config.base, CR0, 0);
            wait_ack(config.base, CR0_ACK, 0)?;
            Ok((strtab, cmdq, eventq))
        })?;
    // Inner-shareable WB table and queue walks.
    write32(config.base, CR1, (3 << 10) | (1 << 8) | (1 << 6) | (3 << 4) | (1 << 2) | 1);
    write32(config.base, CR2, (1 << 2) | (1 << 1));
    tables.publish();
    write64(config.base, STRTAB_BASE, u64::from(strtab) | (1 << 62));
    write32(config.base, STRTAB_BASE_CFG, sid_bits as u32);
    write64(config.base, CMDQ_BASE, u64::from(cmdq) | 8);
    write32(config.base, CMDQ_PROD, 0);
    write32(config.base, CMDQ_CONS, 0);
    write64(config.base, EVTQ_BASE, u64::from(eventq) | 7);
    write32(config.base, EVTQ_PROD, 0);
    write32(config.base, EVTQ_CONS, 0);
    barrier();
    write32(config.base, CR0, CR0_CMDQEN);
    wait_ack(config.base, CR0_ACK, CR0_CMDQEN)?;

    let mut smmu = Smmu {
        config,
        sid_bits,
        oas: (idr5 & 7) as u8,
        strtab,
        _tables: tables,
        cmdq,
        cmd_prod: 0,
        next_domain: 1,
        domains: BTreeMap::new(),
        streams: BTreeMap::new(),
    };
    smmu.issue([0x04, 0])?;
    smmu.issue([0x30, 0])?;
    smmu.sync()?;
    write32(config.base, CR0, CR0_CMDQEN | CR0_EVTQEN | CR0_SMMUEN);
    wait_ack(config.base, CR0_ACK, CR0_CMDQEN | CR0_EVTQEN | CR0_SMMUEN)?;
    write32(config.base, IRQ_CTRL, IRQ_EVTQ | IRQ_GERROR);
    wait_ack(config.base, IRQ_CTRL_ACK, IRQ_EVTQ | IRQ_GERROR)?;

    IRQ_MMIO.store(config.base, Ordering::Release);
    IRQ_EVENTQ.store(u64::from(eventq), Ordering::Release);
    IRQ_EVENT_INTID.store(config.event_intid, Ordering::Release);
    IRQ_GERROR_INTID.store(config.gerror_intid, Ordering::Release);
    crate::cpu::isa::interrupts::gic::enable_spi(config.event_intid, 0);
    crate::cpu::isa::interrupts::gic::enable_spi(config.gerror_intid, 0);
    crate::logln!(
        "[smmu] enabled SMMUv3 at {:#x}: {} StreamID bits, 4 KiB stage-1 translation",
        physical_base,
        sid_bits
    );
    Ok(smmu)
}

fn with_smmu<R>(f: impl FnOnce(&mut Smmu) -> Result<R, Error>) -> Result<R, Error> {
    let mut guard = SMMU.lock();
    if guard.is_none() {
        let config =
            crate::environment::acpi::sdt::iort::discover_smmuv3().ok_or(Error::Unsupported)?;
        *guard = Some(initialize(config)?);
    }
    f(guard.as_mut().expect("SMMU initialized"))
}

/// Initialize the platform SMMU before driver domains begin competing for
/// physical memory.
///
/// A linear stream table can require a large aligned contiguous allocation
/// (4 MiB on QEMU's 16-bit StreamID implementation). Deferring this until an
/// EL0 driver happens to request its first DMA domain made success depend on
/// how earlier boot allocations fragmented physical memory.
pub fn initialize_early() -> Result<(), Error> {
    with_smmu(|_| Ok(()))
}

pub fn stream_id(requester_id: u32) -> Result<u32, Error> {
    let config =
        crate::environment::acpi::sdt::iort::discover_smmuv3().ok_or(Error::Unsupported)?;
    config.stream_id(requester_id).ok_or(Error::InvalidStream)
}

pub(crate) fn create_domain_with_reset(
    sid: u32,
    msi_address: Option<u64>,
    reset: impl FnOnce(bool) -> Result<(), Error>,
) -> Result<u64, Error> {
    with_smmu(|smmu| {
        match smmu.streams.get(&sid) {
            Some(0) => reset(true)?,
            Some(_) => return Err(Error::StreamInUse),
            None => reset(false)?,
        }
        let id = smmu.next_domain;
        smmu.next_domain += 1;
        let asid = u16::try_from(id).map_err(|_| Error::MapFailed)?;
        let domain = Domain::new(asid, sid, smmu.oas, msi_address)?;
        let cd = domain.cd;
        smmu.domains.insert(id, domain);
        smmu.streams.insert(sid, id);
        smmu.domains.get_mut(&id).unwrap().tables.publish();
        if let Err(error) = smmu.write_ste(sid, Some(cd)) {
            smmu.domains.get_mut(&id).unwrap().retiring = true;
            // Retain the exact owner unless abort, original-ASID maintenance
            // and complete physical release all succeed.
            if smmu.write_ste(sid, None).is_ok()
                && smmu.invalidate_asid(asid).is_ok()
                && smmu.domains.get_mut(&id).unwrap().tables.release().is_ok()
            {
                *smmu.streams.get_mut(&sid).unwrap() = 0;
                smmu.domains.remove(&id).expect("new IOMMU domain disappeared");
            } else {
                crate::logln!("[smmu] quarantining failed domain {} for sid {:#x}", id, sid);
            }
            return Err(error);
        }
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
    let result = with_smmu(|smmu| {
        let (mapped, asid) = {
            let domain = smmu.domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?;
            let pin = pending_pin.take().expect("DMA pin consumed twice");
            (domain.map(pin, direction), domain.asid)
        };
        let iova = match mapped {
            Ok(iova) => iova,
            Err((error, pin, prefix)) => {
                if !prefix
                    || (!super::test_reject_map_rollback() && smmu.invalidate_asid(asid).is_ok())
                {
                    pending_pin = Some(pin);
                } else {
                    smmu.domains.get_mut(&domain_id).unwrap().quarantined_pins.push(pin);
                }
                return Err(error);
            }
        };
        // If hardware does not acknowledge this invalidation, keep the
        // internal mapping and its pin until domain destruction successfully
        // installs an aborting STE. Returning no IOVA makes the failed mapping
        // unreachable to the driver.
        smmu.invalidate_asid(asid)?;
        Ok(iova)
    });
    if let Some(pin) = pending_pin {
        object::unpin_dma(pin);
    }
    result
}

pub fn unmap(domain_id: u64, iova: u64) -> Result<(), Error> {
    let mapping = with_smmu(|smmu| {
        let (mapping, asid) = {
            let domain = smmu.domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?;
            (domain.clear_mapping(iova)?, domain.asid)
        };
        if let Err(error) = smmu.invalidate_asid(asid) {
            // Hardware may still translate the removed entry. Retain the
            // mapping record and its pin so an acknowledged domain destroy
            // releases them once the aborting STE is installed.
            if let Some(domain) = smmu.domains.get_mut(&domain_id) {
                domain.mappings.insert(iova, mapping);
            }
            return Err(error);
        }
        Ok(mapping)
    })?;
    object::unpin_dma(mapping.pin);
    Ok(())
}

pub fn destroy_domain(domain_id: u64) -> Result<(), Error> {
    let retired = with_smmu(|smmu| {
        let Some(domain) = smmu.domains.get_mut(&domain_id) else {
            return Ok(None);
        };
        domain.retiring = true;
        let sid = domain.sid;
        let asid = domain.asid;
        // Do not release mappings or their memory pins until the aborting STE
        // has been acknowledged. On timeout the domain remains quarantined:
        // leaking authority is preferable to freeing frames a device may
        // still be able to translate.
        smmu.write_ste(sid, None)?;
        if super::test_reject_retirement() {
            return Err(Error::HardwareTimeout);
        }
        // CFGI/SYNC retires structure fetches; TLBI/SYNC additionally completes
        // client transactions translated by this ASID before data-pin release.
        smmu.invalidate_asid(asid)?;
        smmu.domains.get_mut(&domain_id).unwrap().tables.release()?;
        // Hardware translation/drain completion does not reset queued device
        // work. Keep the requester fenced until a confirmed device reset.
        *smmu.streams.get_mut(&sid).expect("registered source") = 0;
        Ok(smmu.domains.remove(&domain_id))
    })?;
    if let Some(domain) = retired {
        for mapping in domain.mappings.into_values() {
            object::unpin_dma(mapping.pin);
        }
        for pin in domain.quarantined_pins {
            object::unpin_dma(pin);
        }
    }
    Ok(())
}

/// Handle an SMMU event or global-error interrupt without taking the SMMU
/// management lock. The queue and MMIO locations become immutable before the
/// interrupt sources are enabled.
pub fn handle_interrupt(intid: u32) -> bool {
    let event_intid = IRQ_EVENT_INTID.load(Ordering::Acquire);
    let gerror_intid = IRQ_GERROR_INTID.load(Ordering::Acquire);
    if intid != event_intid && intid != gerror_intid {
        return false;
    }
    let mmio = IRQ_MMIO.load(Ordering::Acquire);
    if intid == gerror_intid {
        let error = read32(mmio, GERROR) ^ read32(mmio, GERRORN);
        if error != 0 {
            FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
            crate::early_logln!("[smmu] global error {:#x}", error);
            write32(mmio, GERRORN, read32(mmio, GERROR));
        }
    } else {
        let raw_producer = read32(mmio, EVTQ_PROD);
        let producer = raw_producer & (EVENT_ENTRIES * 2 - 1);
        if raw_producer & (1 << 31) != 0 {
            FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
            crate::early_logln!("[smmu] event queue overflow");
        }
        let mut consumer = IRQ_EVENT_CONS.load(Ordering::Relaxed);
        let eventq = PAddr::from(IRQ_EVENTQ.load(Ordering::Acquire));
        while consumer != producer {
            let slot = (consumer & (EVENT_ENTRIES - 1)) as usize;
            let event = unsafe { eventq.into_hhdm_ptr::<u64>().add(slot * 4) };
            let word0 = unsafe { ptr::read_volatile(event) };
            let address = unsafe { ptr::read_volatile(event.add(2)) };
            FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
            crate::early_logln!(
                "[smmu] DMA fault event={:#x} sid={} iova={:#x}",
                word0 & 0xff,
                word0 >> 32,
                address
            );
            consumer = (consumer + 1) & (EVENT_ENTRIES * 2 - 1);
        }
        IRQ_EVENT_CONS.store(consumer, Ordering::Release);
        write32(mmio, EVTQ_CONS, consumer);
    }
    true
}

pub fn fault_count() -> u64 {
    FAULT_COUNT.load(Ordering::Acquire)
}

/// Number of fault events the hardware has produced but the interrupt path
/// has not consumed yet. This is also useful to diagnose a requester whose
/// MSI path is itself affected by a translation failure.
pub fn pending_fault_events() -> u32 {
    let mmio = IRQ_MMIO.load(Ordering::Acquire);
    if mmio == 0 {
        return 0;
    }
    let producer = read32(mmio, EVTQ_PROD) & (EVENT_ENTRIES * 2 - 1);
    let consumer = IRQ_EVENT_CONS.load(Ordering::Acquire);
    producer.wrapping_sub(consumer) & (EVENT_ENTRIES * 2 - 1)
}

/// Guarded abandonment probe; does not initialize or publish hardware.
pub(super) fn test_with_backend_locked(action: impl FnOnce()) {
    let _guard = SMMU.lock();
    action();
}

/// Private, never hardware-published walkers; the data frame is borrowed.
pub(super) fn test_table_admission() {
    let baseline = super::dma_tables::used();
    let data = crate::memory::PreparingUserFrame::allocate_zeroed().unwrap();
    let mut domain = Domain::new(1, 0, 5, None).unwrap();
    let initial = domain.tables.pages();
    domain.tables.set_limit(initial + 3);
    domain.map_page(0x4000_0000, data.frame(), true).unwrap();
    assert_eq!(domain.tables.pages(), initial + 3);
    let l3 = domain.l3_tables[&(0x4000_0000 >> 21)];
    unsafe { (*l3.into_hhdm_mut::<PageTable>())[0].clear() };
    domain.map_page(0x4000_0000, data.frame(), false).unwrap();
    assert_eq!(domain.tables.pages(), initial + 3);
    domain.tables.set_limit(initial + 4);
    assert_eq!(domain.map_page(0x8000_0000, data.frame(), true), Err(Error::MapFailed));
    assert_eq!(domain.tables.pages(), initial + 4);
    assert_eq!(domain.map_page(0x8000_0000, data.frame(), true), Err(Error::MapFailed));
    assert_eq!(domain.tables.pages(), initial + 4);
    domain.tables.set_limit(initial + 5);
    domain.map_page(0x8000_0000, data.frame(), true).unwrap();
    assert_eq!(domain.tables.pages(), initial + 5);
    domain.tables.cancel_unpublished().unwrap();
    assert_eq!(super::dma_tables::used(), baseline);
    data.release().unwrap();
}

/// Serialized fixture places the next map across a cached/fresh leaf-table boundary.
pub(super) fn test_reject_sparse_map(id: u64) {
    with_smmu(|unit| {
        let domain = unit.domains.get_mut(&id).unwrap();
        domain.next_iova = IOVA_START + (2 * 1024 * 1024 - PAGE_SIZE) as u64;
        domain.tables.set_limit(domain.tables.pages());
        Ok(())
    })
    .unwrap();
}
