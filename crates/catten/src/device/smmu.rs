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

use super::{
    dma_tables::{
        Scope,
        Tables,
    },
    unit_initialization::{
        self,
        Rejected,
        UnitBacking,
        UnitState,
    },
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
    OperationInFlight,
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

pub(super) struct Domain {
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

struct Commands {
    base: usize,
    cmdq: PAddr,
    cmd_prod: u32,
}

struct Smmu {
    commands: Option<Commands>,
    sid_bits: u8,
    oas: u8,
    strtab: PAddr,
    _tables: Tables,
    next_domain: u64,
    domains: BTreeMap<u64, Option<Domain>>,
    streams: BTreeMap<u32, u64>,
}

impl UnitBacking for Smmu {
    fn tables(&mut self) -> &mut Tables {
        &mut self._tables
    }
}

static SMMU: LazyLock<Mutex<UnitState<Smmu>>> = LazyLock::new(|| Mutex::new(UnitState::Vacant));

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
    #[allow(clippy::result_large_err)] // Return the complete private owner inline.
    fn new(
        asid: u16,
        sid: u32,
        oas: u8,
        msi_address: Option<u64>,
    ) -> Result<Self, (Error, super::detached_domain::DetachedDomain<Self>)> {
        let mut domain = Self {
            retiring: false,
            sid,
            asid,
            root: PAddr::from(0u64),
            cd: PAddr::from(0u64),
            tables: Tables::new(Scope::Domain),
            l3_tables: BTreeMap::new(),
            next_iova: IOVA_START,
            mappings: BTreeMap::new(),
            quarantined_pins: Vec::new(),
        };
        let prepared = (|| {
            domain.root = domain.tables.allocate_frame()?;
            domain.cd = domain.tables.allocate_frame()?;
            let cd_words = unsafe { domain.cd.into_hhdm_mut::<u64>() };
            // Private CD is complete before any hardware-visible STE.
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
                ptr::write_volatile(cd_words.add(1), u64::from(domain.root));
                ptr::write_volatile(cd_words.add(3), 0xff);
            }
            if let Some(address) = msi_address {
                let page = address & !(PAGE_SIZE as u64 - 1);
                domain.map_page(page, PAddr::from(page), true)?;
            }
            Ok(())
        })();
        match prepared {
            Ok(()) => Ok(domain),
            Err(error) => Err((error, super::detached_domain::DetachedDomain::new(domain))),
        }
    }

    pub(super) fn private_tables(&mut self) -> &mut Tables {
        &mut self.tables
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

    fn map(
        &mut self,
        pending: &mut super::mapping::PendingPin,
        direction: Direction,
    ) -> Result<u64, (Error, bool)> {
        let pin = pending.borrow();
        if self.retiring {
            return Err((Error::UnknownDomain, false));
        }
        if self.mappings.values().any(|mapping| mapping.pin.object_id() == pin.object_id())
            || self.quarantined_pins.iter().any(|held| held.object_id() == pin.object_id())
        {
            return Err((Error::AlreadyMapped, false));
        }
        // Prepare enough quarantine capacity for every live mapping, before leaves.
        if self.quarantined_pins.try_reserve(self.mappings.len() + 1).is_err() {
            return Err((Error::Memory, false));
        }
        let pages = pin.frames().len();
        let Some(bytes) = (pages as u64).checked_mul(PAGE_SIZE as u64) else {
            return Err((Error::OutOfIova, false));
        };
        let iova = self.next_iova;
        let Some(next_iova) = self
            .next_iova
            .checked_add(bytes)
            .and_then(|next| next.checked_add(PAGE_SIZE as u64 - 1))
            .map(|next| next & !(PAGE_SIZE as u64 - 1))
        else {
            return Err((Error::OutOfIova, false));
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
                return Err((error, index != 0));
            }
        }
        barrier();
        self.next_iova = next_iova;
        self.mappings.insert(
            iova,
            Mapping {
                pin: pending.take(),
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

impl Commands {
    fn issue(&mut self, command: [u64; 2]) -> Result<(), Error> {
        if !charlotte_lifecycle::iommu::smmu_queue_has_space(
            self.cmd_prod,
            read32(self.base, CMDQ_CONS),
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
        write32(self.base, CMDQ_PROD, self.cmd_prod);
        Ok(())
    }

    fn sync(&mut self) -> Result<(), Error> {
        self.issue([0x46, 0])?;
        for _ in 0..1_000_000 {
            if read32(self.base, CMDQ_CONS) == self.cmd_prod {
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
}

impl Smmu {
    fn issue(&mut self, command: [u64; 2]) -> Result<(), Error> {
        self.commands.as_mut().ok_or(Error::OperationInFlight)?.issue(command)
    }

    fn sync(&mut self) -> Result<(), Error> {
        self.commands.as_mut().ok_or(Error::OperationInFlight)?.sync()
    }

    fn write_ste(&mut self, sid: u32, cd: Option<PAddr>) -> Result<(), Error> {
        self.publish_ste(sid, cd)?;
        self.commands.as_mut().ok_or(Error::OperationInFlight)?.invalidate_ste(sid)
    }

    fn publish_ste(&mut self, sid: u32, cd: Option<PAddr>) -> Result<(), Error> {
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
            return Ok(());
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
        Ok(())
    }
}

#[allow(clippy::result_large_err)] // Keep the complete unit owner inline, without allocation.
fn initialize(
    mut config: SmmuV3Config,
) -> Result<super::detached_domain::DetachedDomain<Smmu>, Rejected<Smmu>> {
    if !config.coherent {
        // Non-coherent table walks require explicit cache maintenance, which
        // this first implementation intentionally does not pretend to offer.
        return Err(Rejected::before_backing(Error::Unsupported));
    }
    let mut current = AddressSpace::get_current();
    current
        .map_mmio_region(config.base, 0x2_0000)
        .map_err(|_| Rejected::before_backing(Error::MapFailed))?;
    let physical_base = config.base;
    config.base = unsafe { PAddr::from(physical_base as u64).into_hhdm_ptr::<u8>() } as usize;
    let idr0 = read32(config.base, IDR0);
    let idr1 = read32(config.base, IDR1);
    let idr5 = read32(config.base, IDR5);
    if idr0 & (1 << 1) == 0 || idr5 & (1 << 4) == 0 {
        return Err(Rejected::before_backing(Error::Unsupported));
    }
    let sid_bits = (idr1 & 0x3f) as u8;
    if sid_bits == 0 || sid_bits > 16 || idr1 & ((1 << 30) | (1 << 29)) != 0 {
        return Err(Rejected::before_backing(Error::Unsupported));
    }
    let entries = 1usize << sid_bits;
    let strtab_bytes =
        entries.checked_mul(STE_SIZE).ok_or_else(|| Rejected::before_backing(Error::MapFailed))?;
    let strtab_frames = strtab_bytes.div_ceil(PAGE_SIZE);
    let mut owner = super::detached_domain::DetachedDomain::new(Smmu {
        commands: Some(Commands {
            base: config.base,
            cmdq: PAddr::from(0u64),
            cmd_prod: 0,
        }),
        sid_bits,
        oas: (idr5 & 7) as u8,
        strtab: PAddr::from(0u64),
        _tables: Tables::new(Scope::Unit),
        next_domain: 1,
        domains: BTreeMap::new(),
        streams: BTreeMap::new(),
    });
    let smmu = owner.value_mut();
    let mut rollback_private = true;
    let prepared = (|| {
        smmu.strtab = smmu
            ._tables
            .allocate(strtab_frames, strtab_bytes.next_power_of_two().max(PAGE_SIZE))?;
        for sid in 0..entries {
            let ste = unsafe { smmu.strtab.into_hhdm_mut::<u64>().add(sid * 8) };
            unsafe { ptr::write_volatile(ste, STE_VALID) };
        }
        smmu.commands.as_mut().unwrap().cmdq = smmu._tables.allocate_frame()?;
        let eventq = smmu._tables.allocate_frame()?;
        if unit_initialization::reject_complete() {
            return Err(Error::MapFailed);
        }
        rollback_private = false; // Hardware control starts; no initialization replay.
        write32(config.base, CR0, 0);
        unit_initialization::wait_boundary();
        wait_ack(config.base, CR0_ACK, 0)?;
        // Inner-shareable WB table and queue walks.
        write32(config.base, CR1, (3 << 10) | (1 << 8) | (1 << 6) | (3 << 4) | (1 << 2) | 1);
        write32(config.base, CR2, (1 << 2) | (1 << 1));
        unit_initialization::publication_boundary();
        smmu._tables.publish();
        write64(config.base, STRTAB_BASE, u64::from(smmu.strtab) | (1 << 62));
        write32(config.base, STRTAB_BASE_CFG, sid_bits as u32);
        write64(config.base, CMDQ_BASE, u64::from(smmu.commands.as_ref().unwrap().cmdq) | 8);
        write32(config.base, CMDQ_PROD, 0);
        write32(config.base, CMDQ_CONS, 0);
        write64(config.base, EVTQ_BASE, u64::from(eventq) | 7);
        write32(config.base, EVTQ_PROD, 0);
        write32(config.base, EVTQ_CONS, 0);
        barrier();
        write32(config.base, CR0, CR0_CMDQEN);
        unit_initialization::wait_boundary();
        wait_ack(config.base, CR0_ACK, CR0_CMDQEN)?;
        smmu.issue([0x04, 0])?;
        smmu.issue([0x30, 0])?;
        unit_initialization::wait_boundary();
        smmu.sync()?;
        write32(config.base, CR0, CR0_CMDQEN | CR0_EVTQEN | CR0_SMMUEN);
        unit_initialization::wait_boundary();
        wait_ack(config.base, CR0_ACK, CR0_CMDQEN | CR0_EVTQEN | CR0_SMMUEN)?;
        write32(config.base, IRQ_CTRL, IRQ_EVTQ | IRQ_GERROR);
        unit_initialization::wait_boundary();
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
        Ok(())
    })();
    match prepared {
        Ok(()) => Ok(owner),
        Err(error) if rollback_private => Err(Rejected::with_owner(error, owner)),
        Err(error) => Err(Rejected::retain_owner(error, owner)),
    }
}

fn with_smmu<R>(f: impl FnOnce(&mut Smmu) -> Result<R, Error>) -> Result<R, Error> {
    let mut guard = SMMU.lock();
    // Ordinary operations never initialize hardware beneath their callers'
    // lifecycle/device/config guards. Only boot claims a vacant unit.
    let smmu = guard.installed()?;
    if smmu.commands.is_none() {
        return Err(Error::OperationInFlight);
    }
    f(smmu)
}

// Only the containing maintenance owner may restore its moved engine/domain.
// No initialization, waiting or physical cleanup occurs under this hold.
fn with_registered<R>(f: impl FnOnce(&mut Smmu) -> Result<R, Error>) -> Result<R, Error> {
    let mut guard = SMMU.lock();
    f(guard.installed()?)
}

/// Initialize the platform SMMU before driver domains begin competing for
/// physical memory.
///
/// A linear stream table can require a large aligned contiguous allocation
/// (4 MiB on QEMU's 16-bit StreamID implementation). Deferring this until an
/// EL0 driver happens to request its first DMA domain made success depend on
/// how earlier boot allocations fragmented physical memory.
#[allow(clippy::result_large_err)] // Keep the complete unit owner inline, without allocation.
fn prepare_unit() -> Result<super::detached_domain::DetachedDomain<Smmu>, Rejected<Smmu>> {
    let config = crate::environment::acpi::sdt::iort::discover_smmuv3()
        .ok_or_else(|| Rejected::before_backing(Error::Unsupported))?;
    initialize(config)
}

pub fn initialize_early() -> Result<(), Error> {
    unit_initialization::initialize(&SMMU, prepare_unit, |unit| unit.commands.is_some())
}

pub(crate) fn test_initialization() {
    unit_initialization::test_real(&SMMU, prepare_unit, 3, 5);
}

pub fn stream_id(requester_id: u32) -> Result<u32, Error> {
    let config =
        crate::environment::acpi::sdt::iort::discover_smmuv3().ok_or(Error::Unsupported)?;
    config.stream_id(requester_id).ok_or(Error::InvalidStream)
}

pub(crate) fn create_domain_with_reset(
    sid: u32,
    msi_address: Option<u64>,
    creation: &mut super::DmaCreation,
    reset: impl FnOnce(bool) -> Result<(), Error>,
) -> Result<(), Error> {
    if creation.is_armed() {
        return Err(Error::OperationInFlight);
    }
    with_smmu(|smmu| {
        match smmu.streams.get(&sid) {
            Some(0) => reset(true)?,
            Some(_) => return Err(Error::StreamInUse),
            None => reset(false)?,
        }
        let id = smmu.next_domain;
        smmu.next_domain += 1;
        let asid = u16::try_from(id).map_err(|_| Error::MapFailed)?;
        let domain = match Domain::new(asid, sid, smmu.oas, msi_address) {
            Ok(domain) => domain,
            Err((error, domain)) => {
                creation.retain_private(super::private_domain::PrivateDomain::Smmu(domain));
                return Err(error);
            }
        };
        if super::test_reject_private_complete() {
            creation.retain_private(super::private_domain::PrivateDomain::Smmu(
                super::detached_domain::DetachedDomain::new(domain),
            ));
            return Err(Error::MapFailed);
        }
        let cd = domain.cd;
        smmu.domains.insert(id, Some(domain));
        smmu.streams.insert(sid, id);
        smmu.domains.get_mut(&id).and_then(Option::as_mut).unwrap().tables.publish();
        creation.record(id);
        let configured = smmu.write_ste(sid, Some(cd)).and_then(|()| {
            if super::test_reject_creation() {
                Err(Error::HardwareTimeout)
            } else {
                Ok(())
            }
        });
        if let Err(error) = configured {
            smmu.domains.get_mut(&id).and_then(Option::as_mut).unwrap().retiring = true;
            // Abort publication is serialized; its CFGI/TLBI/SYNC and physical
            // rollback belong to the enclosing grant after all local guards.
            let _ = smmu.publish_ste(sid, None);
            return Err(error);
        }
        Ok(())
    })
}

fn claim_mapping(
    domain_id: u64,
) -> Result<super::detached_domain::Maintenance<Domain, Commands>, Error> {
    with_smmu(|smmu| {
        let domain =
            smmu.domains.get(&domain_id).and_then(Option::as_ref).ok_or(Error::UnknownDomain)?;
        if domain.retiring {
            return Err(Error::UnknownDomain);
        }
        let domain = smmu.domains.get_mut(&domain_id).unwrap().take().unwrap();
        let commands = smmu.commands.take().expect("admitted mapping engine");
        Ok(super::detached_domain::Maintenance::new(domain, commands))
    })
}

fn restore_mapping(
    domain_id: u64,
    mut owner: super::mapping::MappingMaintenance<Domain, Commands>,
) -> super::mapping::PendingPin {
    let source = owner.held.domain.value_mut().sid;
    with_registered(|smmu| {
        assert!(smmu.commands.is_none(), "claimed mapping engine replaced");
        assert_eq!(smmu.streams.get(&source), Some(&domain_id));
        let slot = smmu.domains.get_mut(&domain_id).expect("claimed mapping slot");
        assert!(slot.is_none(), "claimed mapping domain replaced");
        *slot = Some(owner.held.domain.into_inner());
        smmu.commands = Some(owner.held.commands.into_inner());
        Ok(owner.pending)
    })
    .expect("admitted mapping unit disappeared")
}

/// Production is composed with the exact-root/capability owner in device::mapping.
/// Direct backend calls are confined to kernel boot fixtures.
pub fn map(
    domain_id: u64,
    caller: crate::memory::AddressSpaceId,
    memory_cap: u64,
    direction: Direction,
    exclusive: bool,
) -> Result<u64, Error> {
    map_at(domain_id, caller, memory_cap, direction, exclusive, |_| {})
}

pub(super) fn map_at(
    domain_id: u64,
    caller: crate::memory::AddressSpaceId,
    memory_cap: u64,
    direction: Direction,
    exclusive: bool,
    mut before_completion: impl FnMut(super::mapping::Phase),
) -> Result<u64, Error> {
    let pin = object::pin_for_dma(
        caller,
        memory_cap,
        direction.0 & Direction::DEVICE_READ.0 != 0,
        direction.device_writes(),
        exclusive,
    )
    .map_err(|_| Error::Memory)?;
    let pending = super::mapping::PendingPin::new(Some(pin));
    let held = match claim_mapping(domain_id) {
        Ok(held) => held,
        Err(error) => {
            pending.release();
            return Err(error);
        }
    };
    let mut owner = super::mapping::MappingMaintenance::new(held, pending);
    let mapped = owner.held.domain.value_mut().map(&mut owner.pending, direction);
    let result = match mapped {
        Err((error, prefix)) => {
            if prefix {
                before_completion(super::mapping::Phase::Rollback);
                if super::test_reject_map_rollback()
                    || owner
                        .held
                        .commands
                        .value_mut()
                        .invalidate_asid(owner.held.domain.value_mut().asid)
                        .is_err()
                {
                    let pin = owner.pending.take();
                    owner.held.domain.value_mut().quarantined_pins.push(pin);
                }
            }
            Err(error)
        }
        Ok(iova) => {
            before_completion(super::mapping::Phase::Map);
            match owner
                .held
                .commands
                .value_mut()
                .invalidate_asid(owner.held.domain.value_mut().asid)
            {
                Ok(()) => Ok(iova),
                Err(error) => {
                    // Failed initial maintenance retains the existing mapping/pin;
                    // no IOVA is returned, and domain retirement must complete it.
                    Err(error)
                }
            }
        }
    };
    // Restore exact state in existing cells before releasing confirmed pins,
    // outside serialization. Abandonment retains every field and both claims.
    restore_mapping(domain_id, owner).release();
    result
}

pub fn unmap(domain_id: u64, iova: u64) -> Result<(), Error> {
    unmap_at(domain_id, iova, |_| {})
}

pub(super) fn unmap_at(
    domain_id: u64,
    iova: u64,
    mut before_completion: impl FnMut(super::mapping::Phase),
) -> Result<(), Error> {
    let held = claim_mapping(domain_id)?;
    let mut owner =
        super::mapping::MappingMaintenance::new(held, super::mapping::PendingPin::new(None));
    let result = match owner.held.domain.value_mut().clear_mapping(iova) {
        Err(error) => Err(error),
        Ok(mapping) => {
            owner.pending.retain(mapping.pin);
            before_completion(super::mapping::Phase::Unmap);
            let completed = if super::test_reject_unmap_completion() {
                Err(Error::HardwareTimeout)
            } else {
                owner.held.commands.value_mut().invalidate_asid(owner.held.domain.value_mut().asid)
            };
            if completed.is_err() {
                // Quarantine storage was admitted before the original map.
                // Never allocate an error-path reinsertion node or release
                // a data pin without the backend's actual completion proof.
                let pin = owner.pending.take();
                owner.held.domain.value_mut().quarantined_pins.push(pin);
            }
            completed
        }
    };
    restore_mapping(domain_id, owner).release();
    result
}

pub fn destroy_domain(domain_id: u64) -> Result<(), Error> {
    destroy_domain_at(domain_id, || {}, || {})
}

/// Per-call boot-fixture boundaries; production supplies empty hooks.
pub(super) fn destroy_domain_at(
    domain_id: u64,
    before_maintenance: impl FnOnce(),
    after_detach: impl FnOnce(),
) -> Result<(), Error> {
    let maintenance = with_smmu(|smmu| {
        let Some(slot) = smmu.domains.get_mut(&domain_id) else {
            return Ok(None);
        };
        let domain = slot.as_mut().ok_or(Error::UnknownDomain)?;
        domain.retiring = true;
        let sid = domain.sid;
        // Publish the rejecting descriptor under registry serialization. No
        // configuration wait or table release may occur until this guard leaves.
        smmu.publish_ste(sid, None)?;
        let domain = smmu.domains.get_mut(&domain_id).unwrap().take().unwrap();
        let commands = smmu.commands.take().expect("admitted command engine");
        Ok(Some(super::detached_domain::Maintenance::new(domain, commands)))
    })?;
    let Some(mut owner) = maintenance else {
        return Ok(());
    };
    before_maintenance();
    let sid = owner.domain.value_mut().sid;
    let asid = owner.domain.value_mut().asid;
    let completed = (|| {
        owner.commands.value_mut().invalidate_ste(sid)?;
        if super::test_reject_retirement() {
            return Err(Error::HardwareTimeout);
        }
        owner.commands.value_mut().invalidate_asid(asid)
    })();
    // Restore the actual engine (including timeout producer/epoch state) and
    // the rejected domain under one hold of their original admitted registry.
    let detached = with_registered(|smmu| {
        assert!(smmu.commands.is_none(), "claimed command engine replaced");
        assert_eq!(smmu.streams.get(&sid), Some(&domain_id));
        let slot = smmu.domains.get_mut(&domain_id).expect("claimed domain slot");
        assert!(slot.is_none(), "claimed domain replaced");
        smmu.commands = Some(owner.commands.into_inner());
        if let Err(error) = completed {
            *slot = Some(owner.domain.into_inner());
            return Err(error);
        }
        Ok(owner.domain)
    })?;
    let mut detached = detached;
    after_detach();
    let sid = detached.value_mut().sid;
    // Physical release freezes before touching the allocator. Even partial
    // rejection returns the exact owner to its existing slot without allocation.
    if let Err(error) = detached.value_mut().tables.release() {
        with_registered(|smmu| {
            assert_eq!(smmu.streams.get(&sid), Some(&domain_id));
            let slot = smmu.domains.get_mut(&domain_id).expect("claimed domain slot");
            assert!(slot.is_none(), "claimed domain replaced");
            *slot = Some(detached.into_inner());
            Ok(())
        })?;
        return Err(error);
    }
    with_registered(|smmu| {
        assert!(matches!(smmu.domains.get(&domain_id), Some(None)), "claimed domain replaced");
        assert_eq!(smmu.streams.get(&sid), Some(&domain_id));
        // Drain does not reset queued device work; reset still owns this fence.
        *smmu.streams.get_mut(&sid).unwrap() = 0;
        smmu.domains.remove(&domain_id);
        Ok(())
    })?;
    let domain = detached.into_inner();
    for mapping in domain.mappings.into_values() {
        object::unpin_dma(mapping.pin);
    }
    for pin in domain.quarantined_pins {
        object::unpin_dma(pin);
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

/// Boundary probe; does not initialize hardware or spin on contention.
pub(super) fn test_assert_backend_available() {
    drop(SMMU.try_lock().expect("physical release holds backend registry"));
}

/// Private RAM registers/ring, never installed in the SMMU.
pub(super) fn test_command_engines() {
    let (tables, (registers, cmdq)) = Tables::prepare_unpublished(Scope::Unit, |tables| {
        Ok((tables.allocate_frame()?, tables.allocate_frame()?))
    })
    .unwrap();
    let base = unsafe { registers.into_hhdm_mut::<u8>() } as usize;
    {
        let mut slot = Some(Commands {
            base,
            cmdq,
            cmd_prod: 0,
        });
        let mut owner = super::detached_domain::DetachedDomain::new(slot.take().unwrap());
        assert_eq!(owner.value_mut().invalidate_asid(7), Err(Error::HardwareTimeout));
        slot = Some(owner.into_inner());
        let commands = slot.as_mut().unwrap();
        assert_eq!(commands.cmd_prod, 2);
        let ring = unsafe { cmdq.into_hhdm_ptr::<u64>() };
        let first = unsafe { ring.read_volatile() };
        assert_eq!(first, 0x11 | (7 << 48));
        assert_eq!(unsafe { ring.add(2).read_volatile() }, 0x46);
        assert_eq!(commands.invalidate_ste(9), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_prod, 4);
        assert_eq!(read32(base, CMDQ_PROD), 4);
        assert_eq!(unsafe { ring.read_volatile() }, first);
        write32(base, CMDQ_CONS, 4 ^ QUEUE_ENTRIES); // Full ring.
        let word = unsafe { ring.add(8).read_volatile() };
        assert_eq!(commands.issue([0x30, 0]), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_prod, 4);
        assert_eq!(unsafe { ring.add(8).read_volatile() }, word);
        write32(base, CMDQ_CONS, 1 << 24); // Invalid consumer must not allow reuse.
        assert_eq!(commands.issue([0x30, 0]), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_prod, 4);
        // One slot remains: TLBI enters the queue, then SYNC admission rejects.
        write32(base, CMDQ_CONS, 5 ^ QUEUE_ENTRIES);
        let sync_word = unsafe { ring.add(10).read_volatile() };
        assert_eq!(commands.invalidate_asid(8), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_prod, 5);
        assert_eq!(read32(base, CMDQ_PROD), 5);
        assert_eq!(unsafe { ring.add(8).read_volatile() }, 0x11 | (8 << 48));
        assert_eq!(unsafe { ring.add(10).read_volatile() }, sync_word);
    } // Private command authority ends before backing cancellation.
    tables.cancel_unpublished().unwrap();
    crate::logln!(
        "[smmu command owner] private timeout producer, full-ring and malformed-consumer \
         rejection preserved"
    );
}

/// Guarded abandonment probe; does not initialize or publish hardware.
pub(super) fn test_with_backend_locked(action: impl FnOnce()) {
    let _guard = SMMU.lock();
    action();
}

/// Private typed payload with real tables/metadata, never in the registry.
pub(super) fn test_private_domain() -> super::private_domain::PrivateDomain {
    let mut domain =
        Domain::new(1, 0, 5, Some(0xfee0_0000)).unwrap_or_else(|(error, mut owner)| {
            owner.value_mut().tables.cancel_private().unwrap();
            drop(owner.into_inner());
            panic!("private retention fixture failed: {:?}", error)
        });

    domain.quarantined_pins.try_reserve(2).unwrap();
    super::private_domain::PrivateDomain::Smmu(super::detached_domain::DetachedDomain::new(domain))
}

/// Private, never hardware-published walkers; the data frame is borrowed.
pub(super) fn test_table_admission() {
    let baseline = super::dma_tables::used();
    let data = crate::memory::PreparingUserFrame::allocate_zeroed().unwrap();
    let mut domain = Domain::new(1, 0, 5, None).unwrap_or_else(|(error, mut domain)| {
        domain.value_mut().tables.cancel_private().unwrap();
        drop(domain.into_inner());
        panic!("private walker preparation rejected: {:?}", error)
    });
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
        let domain = unit.domains.get_mut(&id).and_then(Option::as_mut).unwrap();
        domain.next_iova = IOVA_START + (2 * 1024 * 1024 - PAGE_SIZE) as u64;
        domain.tables.set_limit(domain.tables.pages());
        Ok(())
    })
    .unwrap();
}
