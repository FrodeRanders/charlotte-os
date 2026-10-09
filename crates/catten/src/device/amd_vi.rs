//! AMD-Vi (IOMMU) DMA remapping.
//!
//! One kernel-owned second-level translation domain is created per delegated
//! PCI requester, mirroring the Intel VT-d driver. Drivers receive only a
//! `DmaDomain` capability and IOVAs; the device table, page tables, and
//! physical addresses remain kernel-private.
//!
//! The AMD-Vi model uses the PCI requester id (bus:device:function) directly
//! as the 16-bit device id indexing a 32-byte device-table entry. A "translated"
//! entry (Mode=4) points at a standard 4-level IA-32e page table. The interrupt
//! address range (0xfee00000..0xfeefffff) is passed through by the hardware, so
//! MSI/MSI-X delivery needs no explicit identity mapping.

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};
use core::{
    ptr,
    sync::atomic::{
        AtomicU64,
        AtomicUsize,
        Ordering,
    },
};

use spin::LazyLock;

pub use super::dma_common::{
    Direction,
    Error,
};
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
        isa::interface::memory::AddressSpaceInterface,
        multiprocessor::spin::mutex::Mutex,
    },
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
const IOVA_START: u64 = 0x1000_0000;
// Physical address field mask (bits 51:12), shared by the device-table entry
// and every page-table entry.
const ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

// Register offsets (bytes) from the remapping unit's register base.
const DEV_TABLE: usize = 0x0000;
const CMD_BASE: usize = 0x0008;
const EVENT_BASE: usize = 0x0010;
const CONTROL: usize = 0x0018;
const CMD_HEAD: usize = 0x2000;
const CMD_TAIL: usize = 0x2008;
const EVENT_HEAD: usize = 0x2010;
const EVENT_TAIL: usize = 0x2018;
const STATUS: usize = 0x2020;

// Control register bits.
const CONTROL_IOMMU_EN: u64 = 1 << 0;
const CONTROL_EVENT_LOG_EN: u64 = 1 << 2;
const CONTROL_CMD_BUF_EN: u64 = 1 << 12;

// Status register bits.
const STATUS_EVENT_INT: u64 = 1 << 1;
const STATUS_EVENT_OVF: u64 = 1 << 0;

// Device table entry (DTE) fields.
const DTE_VALID: u64 = 1 << 0;
const DTE_TRANSLATION_VALID: u64 = 1 << 1;
const DTE_MODE_SHIFT: u64 = 9;
const DTE_MODE_4LEVEL: u64 = 4;
const DTE_PERM_READ: u64 = 1 << 61;
const DTE_PERM_WRITE: u64 = 1 << 62;

// Page-table entry fields.
const PTE_PRESENT: u64 = 1 << 0;
const PTE_NEXT_SHIFT: u64 = 9;

// Command buffer entry codes (16-byte commands; code in cmd[0] bits 63:60).
const CMD_INVAL_DEVTAB: u64 = 0x02;
const CMD_INVAL_ALL: u64 = 0x08;

const CMD_BUFFER_ENTRIES: u64 = 256;
const CMD_BUFFER_BYTES: usize = CMD_BUFFER_ENTRIES as usize * 16;
const EVENT_LOG_ENTRIES: u64 = 256;
const EVENT_LOG_BYTES: usize = EVENT_LOG_ENTRIES as usize * 16;
const DEVICE_TABLE_ENTRIES: usize = 1 << 16;
const DEVICE_TABLE_BYTES: usize = DEVICE_TABLE_ENTRIES * 32;
const DEVICE_TABLE_FRAMES: usize = DEVICE_TABLE_BYTES / PAGE_SIZE;

struct Mapping {
    pin: DmaPin,
    pages: usize,
}

pub(super) struct Domain {
    retiring: bool,
    source_id: u16,
    root: PAddr,
    tables: Tables,
    next_iova: u64,
    mappings: BTreeMap<u64, Mapping>,
    quarantined_pins: Vec<DmaPin>,
}

impl Domain {
    #[allow(clippy::result_large_err)] // Return the complete private owner inline.
    fn new(source_id: u16) -> Result<Self, (Error, super::detached_domain::DetachedDomain<Self>)> {
        let mut domain = Self {
            retiring: false,
            source_id,
            root: PAddr::from(0u64),
            tables: Tables::new(Scope::Domain),
            next_iova: IOVA_START,
            mappings: BTreeMap::new(),
            quarantined_pins: Vec::new(),
        };
        match domain.tables.allocate_frame() {
            Ok(root) => {
                domain.root = root;
                Ok(domain)
            }
            Err(error) => Err((error, super::detached_domain::DetachedDomain::new(domain))),
        }
    }

    pub(super) fn private_tables(&mut self) -> &mut Tables {
        &mut self.tables
    }

    fn map_page(&mut self, iova: u64, frame: PAddr, writable: bool) -> Result<(), Error> {
        let mut parent = self.root;
        // Descend the non-leaf levels 4 (PML4), 3 (PDPT), 2 (PD).
        for level in (2u64..=4).rev() {
            let shift = 12 + (level - 1) * 9;
            let index = ((iova >> shift) & 0x1ff) as usize;
            let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
            let value = unsafe { entry.read_volatile() };
            if value & PTE_PRESENT == 0 {
                let next = self.tables.allocate_frame()?;
                core::sync::atomic::fence(Ordering::Release);
                let next_level = (level - 1) << PTE_NEXT_SHIFT;
                unsafe {
                    entry.write_volatile(
                        (u64::from(next) & ADDR_MASK)
                            | PTE_PRESENT
                            | next_level
                            | DTE_PERM_READ
                            | DTE_PERM_WRITE,
                    );
                }
            }
            parent = PAddr::from(unsafe { entry.read_volatile() } & ADDR_MASK);
        }
        // Leaf level 1 (PT): a 4 KiB page, NextLevel = 0.
        let index = ((iova >> 12) & 0x1ff) as usize;
        let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
        if unsafe { entry.read_volatile() } & PTE_PRESENT != 0 {
            return Err(Error::MapFailed);
        }
        let perms = if writable {
            DTE_PERM_READ | DTE_PERM_WRITE
        } else {
            DTE_PERM_READ
        };
        unsafe { entry.write_volatile((u64::from(frame) & ADDR_MASK) | PTE_PRESENT | perms) };
        Ok(())
    }

    fn clear_page(&mut self, iova: u64) {
        let mut parent = self.root;
        for level in (2u64..=4).rev() {
            let shift = 12 + (level - 1) * 9;
            let index = ((iova >> shift) & 0x1ff) as usize;
            let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
            let value = unsafe { entry.read_volatile() };
            if value & PTE_PRESENT == 0 {
                return;
            }
            parent = PAddr::from(value & ADDR_MASK);
        }
        let index = ((iova >> 12) & 0x1ff) as usize;
        let entry = unsafe { parent.into_hhdm_mut::<u64>().add(index) };
        unsafe { entry.write_volatile(0) };
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
                    self.clear_page(iova + (rollback_index * PAGE_SIZE) as u64);
                }
                return Err((error, pin, index != 0));
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
        if self.retiring {
            return Err(Error::UnknownDomain);
        }
        let mapping = self.mappings.remove(&iova).ok_or(Error::UnknownMapping)?;
        for index in 0..mapping.pages {
            self.clear_page(iova + (index * PAGE_SIZE) as u64);
        }
        Ok(mapping)
    }
}

struct Commands {
    base: usize,
    cmd_buf: PAddr,
    cmd_tail: u32,
    completion: PAddr,
    completion_epoch: u64,
}

struct Unit {
    commands: Option<Commands>,
    devtab: PAddr,
    _tables: Tables,
    next_domain: u64,
    domains: BTreeMap<u64, Option<Domain>>,
    sources: BTreeMap<u16, u64>,
}

impl Unit {
    fn write_dte(&self, source_id: u16, root: PAddr, valid: bool) {
        let entry = unsafe { self.devtab.into_hhdm_mut::<u64>().add(source_id as usize * 4) };
        let d0 = if valid {
            DTE_VALID
                | DTE_TRANSLATION_VALID
                | (DTE_MODE_4LEVEL << DTE_MODE_SHIFT)
                | (u64::from(root) & ADDR_MASK)
                | DTE_PERM_READ
                | DTE_PERM_WRITE
        } else {
            0
        };
        unsafe {
            if !valid {
                entry.write_volatile(0);
                return;
            }
            entry.add(1).write_volatile(0);
            entry.add(2).write_volatile(0);
            entry.add(3).write_volatile(0);
            core::sync::atomic::fence(Ordering::Release);
            entry.write_volatile(d0);
        }
    }

    fn flush_device_table(&mut self, source_id: u16) -> Result<(), Error> {
        self.commands.as_mut().ok_or(Error::OperationInFlight)?.flush_device_table(source_id)
    }

    fn flush_iotlb(&mut self) -> Result<(), Error> {
        self.commands.as_mut().ok_or(Error::OperationInFlight)?.flush_iotlb()
    }
}

impl Commands {
    fn queue_command(&mut self, cmd0: u64, cmd1: u64) -> Result<(), Error> {
        let tail = self.cmd_tail as usize;
        let head = read64(self.base, CMD_HEAD);
        let new_tail = (tail + 16) & (CMD_BUFFER_BYTES - 1);
        if head >= CMD_BUFFER_BYTES as u64 || head & 15 != 0 || new_tail as u64 == head {
            // A timed-out command can still be in flight. Never overwrite its
            // ring slot merely because a later caller wants to retry.
            return Err(Error::HardwareTimeout);
        }
        let entry = unsafe { self.cmd_buf.into_hhdm_mut::<u64>().add(tail / 8) };
        unsafe {
            entry.write_volatile(cmd0);
            entry.add(1).write_volatile(cmd1);
        }
        core::sync::atomic::fence(Ordering::Release);
        self.cmd_tail = new_tail as u32;
        write64(self.base, CMD_TAIL, new_tail as u64);
        Ok(())
    }

    fn submit_command(&mut self, cmd0: u64, cmd1: u64) -> Result<(), Error> {
        let epoch = self.completion_epoch.checked_add(1).ok_or(Error::HardwareTimeout)?;
        let command =
            charlotte_lifecycle::iommu::amd_completion_command(u64::from(self.completion), epoch)
                .ok_or(Error::Unsupported)?;
        self.completion_epoch = epoch;
        self.queue_command(cmd0, cmd1)?;
        self.queue_command(command[0], command[1])?;
        for _ in 0..1_000_000 {
            let observed = unsafe { self.completion.into_hhdm_ptr::<u64>().read_volatile() };
            if charlotte_lifecycle::iommu::completion_matches(observed, epoch) {
                core::sync::atomic::fence(Ordering::Acquire);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        // The semaphore and command backing remain Unit-owned, including on
        // timeout. A late completion cannot satisfy a different retry epoch.
        Err(Error::HardwareTimeout)
    }

    fn flush_device_table(&mut self, source_id: u16) -> Result<(), Error> {
        self.submit_command((CMD_INVAL_DEVTAB << 60) | source_id as u64, 0)
    }

    fn flush_iotlb(&mut self) -> Result<(), Error> {
        self.submit_command(CMD_INVAL_ALL << 60, 0)
    }
}

impl UnitBacking for Unit {
    fn tables(&mut self) -> &mut Tables {
        &mut self._tables
    }
}

static UNIT: LazyLock<Mutex<UnitState<Unit>>> = LazyLock::new(|| Mutex::new(UnitState::Vacant));
static IRQ_MMIO: AtomicUsize = AtomicUsize::new(0);
static FAULT_COUNT: AtomicU64 = AtomicU64::new(0);

fn read64(base: usize, offset: usize) -> u64 {
    unsafe { ptr::read_volatile((base + offset) as *const u64) }
}

fn write64(base: usize, offset: usize, value: u64) {
    unsafe { ptr::write_volatile((base + offset) as *mut u64, value) }
}

#[allow(clippy::result_large_err)] // Keep the complete unit owner inline, without allocation.
fn initialize(
    config: crate::environment::acpi::sdt::ivrs::IvrsConfig,
) -> Result<super::detached_domain::DetachedDomain<Unit>, Rejected<Unit>> {
    let mut current = AddressSpace::get_current();
    current
        .map_mmio_region(config.base, 0x4000)
        .map_err(|_| Rejected::before_backing(Error::MapFailed))?;
    let base = unsafe { PAddr::from(config.base as u64).into_hhdm_ptr::<u8>() } as usize;
    let mut owner = super::detached_domain::DetachedDomain::new(Unit {
        commands: Some(Commands {
            base,
            cmd_buf: PAddr::from(0u64),
            cmd_tail: 0,
            completion: PAddr::from(0u64),
            completion_epoch: 0,
        }),
        devtab: PAddr::from(0u64),
        _tables: Tables::new(Scope::Unit),
        next_domain: 1,
        domains: BTreeMap::new(),
        sources: BTreeMap::new(),
    });
    let unit = owner.value_mut();
    let mut rollback_private = true;
    let prepared = (|| {
        unit.devtab = unit._tables.allocate(DEVICE_TABLE_FRAMES, DEVICE_TABLE_BYTES)?;
        let commands = unit.commands.as_mut().unwrap();
        commands.cmd_buf = unit._tables.allocate_frame()?;
        let event_log = unit._tables.allocate_frame()?;
        commands.completion = unit._tables.allocate_frame()?;
        if unit_initialization::reject_complete() {
            return Err(Error::MapFailed);
        }
        rollback_private = false; // Hardware control starts; no initialization replay.
        unit_initialization::publication_boundary();
        unit._tables.publish();
        // Cover the complete 16-bit DeviceID space; 511 encodes 2 MiB.
        write64(base, DEV_TABLE, u64::from(unit.devtab) | (DEVICE_TABLE_FRAMES as u64 - 1));
        write64(base, CMD_BASE, u64::from(commands.cmd_buf) | (8 << 56));
        write64(base, EVENT_BASE, u64::from(event_log) | (8 << 56));
        write64(base, CMD_HEAD, 0);
        write64(base, CMD_TAIL, 0);
        write64(base, EVENT_HEAD, 0);
        write64(base, EVENT_TAIL, 0);
        write64(base, CONTROL, CONTROL_IOMMU_EN | CONTROL_CMD_BUF_EN | CONTROL_EVENT_LOG_EN);
        IRQ_MMIO.store(base, Ordering::Release);
        crate::logln!("[amdvi] enabled AMD-Vi at {:#x}", config.base);
        Ok(())
    })();
    match prepared {
        Ok(()) => Ok(owner),
        Err(error) if rollback_private => Err(Rejected::with_owner(error, owner)),
        Err(error) => Err(Rejected::retain_owner(error, owner)),
    }
}

fn with_unit<R>(f: impl FnOnce(&mut Unit) -> Result<R, Error>) -> Result<R, Error> {
    let mut guard = UNIT.lock();
    // Ordinary operations never initialize hardware beneath their callers'
    // lifecycle/device/config guards. Only boot claims a vacant unit.
    let unit = guard.installed()?;
    if unit.commands.is_none() {
        return Err(Error::OperationInFlight);
    }
    f(unit)
}

// Only the containing maintenance owner may restore its moved engine/domain.
// No initialization, waiting or physical cleanup occurs under this hold.
fn with_registered<R>(f: impl FnOnce(&mut Unit) -> Result<R, Error>) -> Result<R, Error> {
    let mut guard = UNIT.lock();
    f(guard.installed()?)
}

#[allow(clippy::result_large_err)] // Keep the complete unit owner inline, without allocation.
fn prepare_unit() -> Result<super::detached_domain::DetachedDomain<Unit>, Rejected<Unit>> {
    let config = crate::environment::acpi::sdt::ivrs::discover_amd_vi()
        .ok_or_else(|| Rejected::before_backing(Error::Unsupported))?;
    initialize(config)
}

pub fn initialize_early() -> Result<(), Error> {
    unit_initialization::initialize(&UNIT, prepare_unit, |unit| unit.commands.is_some())
}

pub(super) fn test_initialization() {
    unit_initialization::test_real(&UNIT, prepare_unit, 4, 0);
}

pub fn stream_id(requester_id: u32) -> Result<u32, Error> {
    crate::environment::acpi::sdt::ivrs::discover_amd_vi().ok_or(Error::Unsupported)?;
    u16::try_from(requester_id).map(u32::from).map_err(|_| Error::InvalidStream)
}

pub(crate) fn create_domain_with_reset(
    sid: u32,
    _msi_address: Option<u64>,
    creation: &mut super::DmaCreation,
    reset: impl FnOnce(bool) -> Result<(), Error>,
) -> Result<(), Error> {
    if creation.is_armed() {
        return Err(Error::OperationInFlight);
    }
    with_unit(|unit| {
        let source_id = u16::try_from(sid).map_err(|_| Error::InvalidStream)?;
        match unit.sources.get(&source_id) {
            Some(0) => reset(true)?,
            Some(_) => return Err(Error::StreamInUse),
            None => reset(false)?,
        }
        let id = unit.next_domain;
        unit.next_domain = unit.next_domain.checked_add(1).ok_or(Error::MapFailed)?;
        let domain = match Domain::new(source_id) {
            Ok(domain) => domain,
            Err((error, domain)) => {
                creation.retain_private(super::private_domain::PrivateDomain::AmdVi(domain));
                return Err(error);
            }
        };
        if super::test_reject_private_complete() {
            creation.retain_private(super::private_domain::PrivateDomain::AmdVi(
                super::detached_domain::DetachedDomain::new(domain),
            ));
            return Err(Error::MapFailed);
        }
        let root = domain.root;
        unit.domains.insert(id, Some(domain));
        unit.sources.insert(source_id, id);
        unit.domains.get_mut(&id).and_then(Option::as_mut).unwrap().tables.publish();
        creation.record(id);
        unit.write_dte(source_id, root, true);
        let configured = unit.flush_device_table(source_id).and_then(|()| {
            if super::test_reject_creation() {
                Err(Error::HardwareTimeout)
            } else {
                Ok(())
            }
        });
        if let Err(error) = configured {
            unit.domains.get_mut(&id).and_then(Option::as_mut).unwrap().retiring = true;
            unit.write_dte(source_id, PAddr::from(0u64), false);
            // Do not lose the enclosing grant's rollback obligation or perform
            // maintenance/physical release beneath this guard.
            return Err(error);
        }
        Ok(())
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
        direction.device_reads(),
        direction.device_writes(),
        exclusive,
    )
    .map_err(|_| Error::Memory)?;
    let mut pending_pin = Some(pin);
    let result = with_unit(|unit| {
        let mapped = {
            let domain = unit
                .domains
                .get_mut(&domain_id)
                .and_then(Option::as_mut)
                .ok_or(Error::UnknownDomain)?;
            let pin = pending_pin.take().expect("DMA pin consumed twice");
            domain.map(pin, direction)
        };
        let iova = match mapped {
            Ok(iova) => iova,
            Err((error, pin, prefix)) => {
                if !prefix || (!super::test_reject_map_rollback() && unit.flush_iotlb().is_ok()) {
                    pending_pin = Some(pin);
                } else {
                    unit.domains
                        .get_mut(&domain_id)
                        .and_then(Option::as_mut)
                        .unwrap()
                        .quarantined_pins
                        .push(pin);
                }
                return Err(error);
            }
        };
        if let Err(error) = unit.flush_iotlb() {
            let mapping = unit
                .domains
                .get_mut(&domain_id)
                .and_then(Option::as_mut)
                .expect("AMD-Vi domain disappeared during map rollback")
                .clear_mapping(iova)
                .expect("new AMD-Vi mapping disappeared during rollback");
            if unit.flush_iotlb().is_ok() {
                pending_pin = Some(mapping.pin);
            } else {
                unit.domains
                    .get_mut(&domain_id)
                    .and_then(Option::as_mut)
                    .unwrap()
                    .quarantined_pins
                    .push(mapping.pin);
            }
            return Err(error);
        }
        Ok(iova)
    });
    if let Some(pin) = pending_pin {
        object::unpin_dma(pin);
    }
    result
}

pub fn unmap(domain_id: u64, iova: u64) -> Result<(), Error> {
    let mapping = with_unit(|unit| {
        let mapping = unit
            .domains
            .get_mut(&domain_id)
            .and_then(Option::as_mut)
            .ok_or(Error::UnknownDomain)?
            .clear_mapping(iova)?;
        if let Err(error) = unit.flush_iotlb() {
            unit.domains
                .get_mut(&domain_id)
                .and_then(Option::as_mut)
                .unwrap()
                .quarantined_pins
                .push(mapping.pin);
            return Err(error);
        }
        Ok(mapping)
    })?;
    object::unpin_dma(mapping.pin);
    Ok(())
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
    let maintenance = with_unit(|unit| {
        let Some(slot) = unit.domains.get_mut(&domain_id) else {
            return Ok(None);
        };
        let domain = slot.as_mut().ok_or(Error::UnknownDomain)?;
        domain.retiring = true;
        let source_id = domain.source_id;
        // Publish the rejecting descriptor under registry serialization. No
        // configuration wait or table release may occur until this guard leaves.
        unit.write_dte(source_id, PAddr::from(0u64), false);
        let domain = unit.domains.get_mut(&domain_id).unwrap().take().unwrap();
        let commands = unit.commands.take().expect("admitted command engine");
        Ok(Some(super::detached_domain::Maintenance::new(domain, commands)))
    })?;
    let Some(mut owner) = maintenance else {
        return Ok(());
    };
    before_maintenance();
    let source_id = owner.domain.value_mut().source_id;
    let completed = (|| {
        owner.commands.value_mut().flush_device_table(source_id)?;
        if super::test_reject_retirement() {
            return Err(Error::HardwareTimeout);
        }
        owner.commands.value_mut().flush_iotlb()
    })();
    // Restore the actual engine (including timeout producer/epoch state) and
    // the rejected domain under one hold of their original admitted registry.
    let detached = with_registered(|unit| {
        assert!(unit.commands.is_none(), "claimed command engine replaced");
        assert_eq!(unit.sources.get(&source_id), Some(&domain_id));
        let slot = unit.domains.get_mut(&domain_id).expect("claimed domain slot");
        assert!(slot.is_none(), "claimed domain replaced");
        unit.commands = Some(owner.commands.into_inner());
        if let Err(error) = completed {
            *slot = Some(owner.domain.into_inner());
            return Err(error);
        }
        Ok(owner.domain)
    })?;
    let mut detached = detached;
    after_detach();
    let source_id = detached.value_mut().source_id;
    // Physical release freezes before touching the allocator. Even partial
    // rejection returns the exact owner to its existing slot without allocation.
    if let Err(error) = detached.value_mut().tables.release() {
        with_registered(|unit| {
            assert_eq!(unit.sources.get(&source_id), Some(&domain_id));
            let slot = unit.domains.get_mut(&domain_id).expect("claimed domain slot");
            assert!(slot.is_none(), "claimed domain replaced");
            *slot = Some(detached.into_inner());
            Ok(())
        })?;
        return Err(error);
    }
    with_registered(|unit| {
        assert!(matches!(unit.domains.get(&domain_id), Some(None)), "claimed domain replaced");
        assert_eq!(unit.sources.get(&source_id), Some(&domain_id));
        // Drain does not reset queued device work; reset still owns this fence.
        *unit.sources.get_mut(&source_id).unwrap() = 0;
        unit.domains.remove(&domain_id);
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

/// Handle an AMD-Vi fault interrupt. The first implementation has no MSI fault
/// route (the AMD-Vi delivers events through its own PCI MSI capability), so
/// faults are drained by [`fault_count`] instead; this exists for interface
/// parity with the other IOMMU drivers.
pub fn handle_interrupt(_intid: u32) -> bool {
    false
}

fn pending_event_bytes(base: usize) -> u64 {
    let buf_mask = (EVENT_LOG_BYTES - 1) as u64;
    let head = read64(base, EVENT_HEAD) & buf_mask;
    let tail = read64(base, EVENT_TAIL) & buf_mask;
    tail.wrapping_sub(head) & buf_mask
}

/// Number of DMA translation faults observed since boot. Latched events are
/// drained (the head pointer is advanced) here because there is no MSI fault
/// route yet.
pub fn fault_count() -> u64 {
    let base = IRQ_MMIO.load(Ordering::Acquire);
    if base == 0 {
        return FAULT_COUNT.load(Ordering::Acquire);
    }
    let pending = pending_event_bytes(base) / 16;
    if pending > 0 {
        let buf_mask = (EVENT_LOG_BYTES - 1) as u64;
        let tail = read64(base, EVENT_TAIL) & buf_mask;
        write64(base, EVENT_HEAD, tail);
        let status = read64(base, STATUS);
        write64(base, STATUS, status & (STATUS_EVENT_INT | STATUS_EVENT_OVF));
        return FAULT_COUNT.fetch_add(pending, Ordering::Relaxed) + pending;
    }
    FAULT_COUNT.load(Ordering::Acquire)
}

/// Number of fault events the hardware has latched but not yet consumed.
pub fn pending_fault_events() -> u32 {
    let base = IRQ_MMIO.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    (pending_event_bytes(base) / 16) as u32
}

/// Boundary probe; does not initialize hardware or spin on contention.
pub(super) fn test_assert_backend_available() {
    drop(UNIT.try_lock().expect("physical release holds backend registry"));
}

/// Private RAM register/ring fixture, never installed in an IOMMU.
pub(super) fn test_command_engine() {
    let (tables, (registers, cmd_buf, completion)) =
        Tables::prepare_unpublished(Scope::Unit, |tables| {
            Ok((tables.allocate(3, PAGE_SIZE)?, tables.allocate_frame()?, tables.allocate_frame()?))
        })
        .unwrap();
    let base = unsafe { registers.into_hhdm_mut::<u8>() } as usize;
    {
        let mut slot = Some(Commands {
            base,
            cmd_buf,
            cmd_tail: 0,
            completion,
            completion_epoch: 0,
        });
        let mut owner = super::detached_domain::DetachedDomain::new(slot.take().unwrap());
        assert_eq!(owner.value_mut().flush_iotlb(), Err(Error::HardwareTimeout));
        slot = Some(owner.into_inner());
        let commands = slot.as_mut().unwrap();
        assert_eq!(commands.cmd_tail, 32);
        assert_eq!(commands.completion_epoch, 1);
        let ring = unsafe { cmd_buf.into_hhdm_ptr::<u64>() };
        let completion_command =
            charlotte_lifecycle::iommu::amd_completion_command(u64::from(completion), 1).unwrap();
        assert_eq!(unsafe { ring.add(2).read_volatile() }, completion_command[0]);
        assert_eq!(unsafe { ring.add(3).read_volatile() }, 1);
        // A stale coherent store must not satisfy the new exact epoch.
        unsafe { completion.into_hhdm_mut::<u64>().write_volatile(1) };
        assert_eq!(commands.flush_iotlb(), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_tail, 64);
        assert_eq!(commands.completion_epoch, 2);
        assert_eq!(unsafe { ring.add(3).read_volatile() }, 1);
        assert_eq!(unsafe { ring.add(7).read_volatile() }, 2);
        assert_eq!(read64(base, CMD_TAIL), 64);
        write64(base, CMD_HEAD, 80); // Full at the next slot: preserve prior work.
        let word = unsafe { ring.add(8).read_volatile() };
        assert_eq!(commands.flush_iotlb(), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_tail, 64);
        assert_eq!(commands.completion_epoch, 3);
        assert_eq!(unsafe { ring.add(8).read_volatile() }, word);
        // One slot remains: invalidation submits but its completion cannot.
        // Preserve that installed prefix and the exact new producer/epoch.
        write64(base, CMD_HEAD, 96);
        let completion_word = unsafe { ring.add(10).read_volatile() };
        assert_eq!(commands.flush_iotlb(), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_tail, 80);
        assert_eq!(commands.completion_epoch, 4);
        assert_eq!(read64(base, CMD_TAIL), 80);
        assert_eq!(unsafe { ring.add(8).read_volatile() }, CMD_INVAL_ALL << 60);
        assert_eq!(unsafe { ring.add(10).read_volatile() }, completion_word);
        commands.completion_epoch = u64::MAX; // Private exhaustion probe.
        assert_eq!(commands.flush_iotlb(), Err(Error::HardwareTimeout));
        assert_eq!(commands.cmd_tail, 80);
        assert_eq!(commands.completion_epoch, u64::MAX);
    } // Private command authority ends before backing cancellation.
    tables.cancel_unpublished().unwrap();
    crate::logln!(
        "[amdvi command owner] private timeout state, stale epoch, full-ring and exhaustion \
         rejection preserved"
    );
}

/// Guarded abandonment probe; does not initialize or publish hardware.
pub(super) fn test_with_backend_locked(action: impl FnOnce()) {
    let _guard = UNIT.lock();
    action();
}

/// Private typed payload with real tables/metadata, never in the registry.
pub(super) fn test_private_domain() -> super::private_domain::PrivateDomain {
    let mut domain = Domain::new(0).unwrap_or_else(|(error, mut owner)| {
        owner.value_mut().tables.cancel_private().unwrap();
        drop(owner.into_inner());
        panic!("private retention fixture failed: {:?}", error)
    });
    domain.tables.allocate_frame().unwrap();
    domain.quarantined_pins.try_reserve(2).unwrap();
    super::private_domain::PrivateDomain::AmdVi(super::detached_domain::DetachedDomain::new(domain))
}

/// Private, never hardware-published walkers; the data frame is borrowed.
pub(super) fn test_table_admission() {
    let baseline = super::dma_tables::used();
    let data = crate::memory::PreparingUserFrame::allocate_zeroed().unwrap();
    let mut domain = Domain::new(0).unwrap_or_else(|(error, mut domain)| {
        domain.value_mut().tables.cancel_private().unwrap();
        drop(domain.into_inner());
        panic!("private walker preparation rejected: {:?}", error)
    });
    let initial = domain.tables.pages();
    domain.tables.set_limit(initial + 3);
    domain.map_page(0x4000_0000, data.frame(), true).unwrap();
    assert_eq!(domain.tables.pages(), initial + 3);
    domain.clear_page(0x4000_0000);
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
    with_unit(|unit| {
        let domain = unit.domains.get_mut(&id).and_then(Option::as_mut).unwrap();
        domain.next_iova = IOVA_START + (2 * 1024 * 1024 - PAGE_SIZE) as u64;
        domain.tables.set_limit(domain.tables.pages());
        Ok(())
    })
    .unwrap();
}
