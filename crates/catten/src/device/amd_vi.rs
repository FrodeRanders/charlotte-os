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
use super::dma_tables::{
    Scope,
    Tables,
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

struct Domain {
    retiring: bool,
    source_id: u16,
    root: PAddr,
    tables: Tables,
    next_iova: u64,
    mappings: BTreeMap<u64, Mapping>,
    quarantined_pins: Vec<DmaPin>,
}

impl Domain {
    fn new(source_id: u16) -> Result<Self, Error> {
        let mut tables = Tables::new(Scope::Domain);
        let root = tables.allocate_frame()?;
        Ok(Self {
            retiring: false,
            source_id,
            root,
            tables,
            next_iova: IOVA_START,
            mappings: BTreeMap::new(),
            quarantined_pins: Vec::new(),
        })
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

struct Unit {
    base: usize,
    devtab: PAddr,
    _tables: Tables,
    cmd_buf: PAddr,
    cmd_tail: u32,
    completion: PAddr,
    completion_epoch: u64,
    next_domain: u64,
    domains: BTreeMap<u64, Domain>,
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

static UNIT: LazyLock<Mutex<Option<Unit>>> = LazyLock::new(|| Mutex::new(None));
static IRQ_MMIO: AtomicUsize = AtomicUsize::new(0);
static FAULT_COUNT: AtomicU64 = AtomicU64::new(0);

fn read64(base: usize, offset: usize) -> u64 {
    unsafe { ptr::read_volatile((base + offset) as *const u64) }
}

fn write64(base: usize, offset: usize, value: u64) {
    unsafe { ptr::write_volatile((base + offset) as *mut u64, value) }
}

fn initialize(config: crate::environment::acpi::sdt::ivrs::IvrsConfig) -> Result<Unit, Error> {
    let mut current = AddressSpace::get_current();
    current.map_mmio_region(config.base, 0x4000).map_err(|_| Error::MapFailed)?;
    let base = unsafe { PAddr::from(config.base as u64).into_hhdm_ptr::<u8>() } as usize;

    let mut tables = Tables::new(Scope::Unit);
    let devtab = tables.allocate(DEVICE_TABLE_FRAMES, DEVICE_TABLE_BYTES)?;
    let cmd_buf = tables.allocate_frame()?;
    let event_log = tables.allocate_frame()?;
    // The completion cell outlives every timed-out coherent store.
    let completion = tables.allocate_frame()?;
    tables.publish();
    // Cover the complete 16-bit DeviceID space. Bits 8:0 encode one less than
    // the table length in 4-KiB units (511 for a 2-MiB table).
    write64(base, DEV_TABLE, u64::from(devtab) | (DEVICE_TABLE_FRAMES as u64 - 1));
    write64(base, CMD_BASE, u64::from(cmd_buf) | (8 << 56));
    write64(base, EVENT_BASE, u64::from(event_log) | (8 << 56));
    write64(base, CMD_HEAD, 0);
    write64(base, CMD_TAIL, 0);
    write64(base, EVENT_HEAD, 0);
    write64(base, EVENT_TAIL, 0);
    write64(base, CONTROL, CONTROL_IOMMU_EN | CONTROL_CMD_BUF_EN | CONTROL_EVENT_LOG_EN);

    IRQ_MMIO.store(base, Ordering::Release);
    crate::logln!("[amdvi] enabled AMD-Vi at {:#x}", config.base);

    Ok(Unit {
        base,
        devtab,
        _tables: tables,
        cmd_buf,
        cmd_tail: 0,
        completion,
        completion_epoch: 0,
        next_domain: 1,
        domains: BTreeMap::new(),
        sources: BTreeMap::new(),
    })
}

fn with_unit<R>(f: impl FnOnce(&mut Unit) -> Result<R, Error>) -> Result<R, Error> {
    let mut guard = UNIT.lock();
    if guard.is_none() {
        let config =
            crate::environment::acpi::sdt::ivrs::discover_amd_vi().ok_or(Error::Unsupported)?;
        *guard = Some(initialize(config)?);
    }
    f(guard.as_mut().expect("AMD-Vi unit initialized"))
}

pub fn initialize_early() -> Result<(), Error> {
    with_unit(|_| Ok(()))
}

pub fn stream_id(requester_id: u32) -> Result<u32, Error> {
    crate::environment::acpi::sdt::ivrs::discover_amd_vi().ok_or(Error::Unsupported)?;
    u16::try_from(requester_id).map(u32::from).map_err(|_| Error::InvalidStream)
}

pub(crate) fn create_domain_with_reset(
    sid: u32,
    _msi_address: Option<u64>,
    reset: impl FnOnce(bool) -> Result<(), Error>,
) -> Result<u64, Error> {
    with_unit(|unit| {
        let source_id = u16::try_from(sid).map_err(|_| Error::InvalidStream)?;
        match unit.sources.get(&source_id) {
            Some(0) => reset(true)?,
            Some(_) => return Err(Error::StreamInUse),
            None => reset(false)?,
        }
        let id = unit.next_domain;
        unit.next_domain = unit.next_domain.checked_add(1).ok_or(Error::MapFailed)?;
        let domain = Domain::new(source_id)?;
        let root = domain.root;
        unit.domains.insert(id, domain);
        unit.sources.insert(source_id, id);
        unit.domains.get_mut(&id).unwrap().tables.publish();
        unit.write_dte(source_id, root, true);
        if let Err(error) = unit.flush_device_table(source_id) {
            unit.domains.get_mut(&id).unwrap().retiring = true;
            unit.write_dte(source_id, PAddr::from(0u64), false);
            if unit.flush_device_table(source_id).is_ok()
                && unit.flush_iotlb().is_ok()
                && unit.domains.get_mut(&id).unwrap().tables.release().is_ok()
            {
                *unit.sources.get_mut(&source_id).unwrap() = 0;
                unit.domains.remove(&id).expect("new IOMMU domain disappeared");
            } else {
                crate::logln!(
                    "[amdvi] quarantining failed domain {} for source {:#x}",
                    id,
                    source_id
                );
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
        direction.device_reads(),
        direction.device_writes(),
        exclusive,
    )
    .map_err(|_| Error::Memory)?;
    let mut pending_pin = Some(pin);
    let result = with_unit(|unit| {
        let mapped = {
            let domain = unit.domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?;
            let pin = pending_pin.take().expect("DMA pin consumed twice");
            domain.map(pin, direction)
        };
        let iova = match mapped {
            Ok(iova) => iova,
            Err((error, pin, prefix)) => {
                if !prefix || (!super::test_reject_map_rollback() && unit.flush_iotlb().is_ok()) {
                    pending_pin = Some(pin);
                } else {
                    unit.domains.get_mut(&domain_id).unwrap().quarantined_pins.push(pin);
                }
                return Err(error);
            }
        };
        if let Err(error) = unit.flush_iotlb() {
            let mapping = unit
                .domains
                .get_mut(&domain_id)
                .expect("AMD-Vi domain disappeared during map rollback")
                .clear_mapping(iova)
                .expect("new AMD-Vi mapping disappeared during rollback");
            if unit.flush_iotlb().is_ok() {
                pending_pin = Some(mapping.pin);
            } else {
                unit.domains.get_mut(&domain_id).unwrap().quarantined_pins.push(mapping.pin);
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
        let mapping =
            unit.domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?.clear_mapping(iova)?;
        if let Err(error) = unit.flush_iotlb() {
            unit.domains.get_mut(&domain_id).unwrap().quarantined_pins.push(mapping.pin);
            return Err(error);
        }
        Ok(mapping)
    })?;
    object::unpin_dma(mapping.pin);
    Ok(())
}

pub fn destroy_domain(domain_id: u64) -> Result<(), Error> {
    let retired = with_unit(|unit| {
        let Some(domain) = unit.domains.get_mut(&domain_id) else {
            return Ok(None);
        };
        domain.retiring = true;
        let source_id = domain.source_id;
        // Remove the translation entry and flush the cached DTE before freeing
        // the page tables a device might still walk.
        unit.write_dte(source_id, PAddr::from(0u64), false);
        unit.flush_device_table(source_id)?;
        if super::test_reject_retirement() {
            return Err(Error::HardwareTimeout);
        }
        unit.flush_iotlb()?;
        unit.domains.get_mut(&domain_id).unwrap().tables.release()?;
        // Hardware translation/drain completion does not reset queued device
        // work. Keep the requester fenced until a confirmed device reset.
        *unit.sources.get_mut(&source_id).expect("registered source") = 0;
        Ok(unit.domains.remove(&domain_id))
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

/// Private, never hardware-published walkers; the data frame is borrowed.
pub(super) fn test_table_admission() {
    let baseline = super::dma_tables::used();
    let data = crate::memory::PreparingUserFrame::allocate_zeroed().unwrap();
    let mut domain = Domain::new(0).unwrap();
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
    drop(domain);
    assert_eq!(super::dma_tables::used(), baseline);
    drop(data);
}

/// Serialized fixture places the next map across a cached/fresh leaf-table boundary.
pub(super) fn test_reject_sparse_map(id: u64) {
    with_unit(|unit| {
        let domain = unit.domains.get_mut(&id).unwrap();
        domain.next_iova = IOVA_START + (2 * 1024 * 1024 - PAGE_SIZE) as u64;
        domain.tables.set_limit(domain.tables.pages());
        Ok(())
    })
    .unwrap();
}
