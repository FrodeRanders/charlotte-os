//! Device capabilities: MMIO regions and interrupt objects.
//!
//! This is the kernel half of the userspace-driver model (architecture doc
//! §10, Phase 8). A driver manager (the supervisor) grants a driver
//! protection domain exactly the authority it needs — a delegated MMIO
//! region capability and an interrupt capability — and nothing else: no
//! arbitrary physical memory and no arbitrary interrupt vectors (§10.1).
//!
//! Two first-class object types are added to the three-primitive model
//! (Capabilities, Endpoints, Memory Objects) as derived facilities:
//!
//! - [`DeviceObject::Mmio`] — a page-granular device register window that a driver can map into its
//!   own address space as Device-nGnRnE memory, reachable from EL0 under its own page table;
//! - [`DeviceObject::Interrupt`] — an interrupt source whose readiness is delivered to the driver's
//!   completion queue. This reuses the same notification machinery as endpoint readiness (Phase 7):
//!   an IRQ posts a coalesced wake to the bound CQ (§16.3: readiness is a notification to inspect
//!   state, not a completion record).
//!
//! Interrupt delivery follows the kernel interrupt path of §10.2: the IRQ
//! handler identifies and masks the source, marks the interrupt object
//! pending, posts a CQ wake to make the owning driver shard runnable, and
//! returns. The driver drains its CQ, handles the device, and re-arms the
//! source with [`interrupt_ack`]. Repeated interrupts coalesce into one wake
//! per CQ (§9.4).
//!
//! Capability possession is the authority: grants are minted only kernel-side
//! by the supervisor and delivered downward, exactly like bootstrap
//! endpoints, so there is no user-facing grant syscall.

#[cfg(target_arch = "x86_64")]
pub mod vt_d;
#[cfg(target_arch = "aarch64")]
pub mod smmu;

use alloc::collections::BTreeMap;
use core::sync::atomic::{
    AtomicU32,
    AtomicU64,
    Ordering,
};

use concurrent_queue::ConcurrentQueue;
#[cfg(target_arch = "x86_64")]
use vt_d as dma;
#[cfg(target_arch = "aarch64")]
use smmu as dma;
use spin::LazyLock;

use crate::{
    completion::CqId,
    cpu::{
        isa::{
            interface::memory::address::Address,
            lp::{
                LpId,
                ops::get_lp_id,
            },
        },
        multiprocessor::spin::mutex::Mutex,
    },
    logln,
    memory::{
        AddressSpaceId,
        VAddr,
        physical::PAddr,
    },
};

const SCHED_TRACE: bool = false;

macro_rules! sched_trace {
    ($($arg:tt)*) => {
        if SCHED_TRACE {
            logln!($($arg)*);
        }
    };
}

const PAGE_SIZE: usize = 4096;

/// A per-address-space handle naming a device object (an MMIO region or an
/// interrupt source). Ids are allocated per address space and start at 1.
pub type DeviceCap = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceError {
    /// No such device capability in the address space's table.
    UnknownCapability,
    /// The capability names a different device object type than the operation
    /// requires (for example mapping an interrupt object).
    WrongType,
    /// The MMIO region is already mapped in the caller's address space.
    AlreadyMapped,
    /// The MMIO region is not mapped in the caller's address space.
    NotMapped,
    /// The kernel could not install (or remove) the requested page mapping.
    MapFailed,
    /// The interrupt object is not bound to a completion queue.
    NotBound,
    /// The interrupt object is already bound to a completion queue.
    AlreadyBound,
    /// The requested MMIO base is not page-aligned.
    NotPageAligned,
    /// The interrupt id is not a routable device interrupt.
    InvalidInterrupt,
    /// Another live capability already owns the interrupt source.
    InterruptInUse,
    /// The address-space id cannot be represented in the lock-free route table.
    InvalidAddressSpace,
    /// The requested MMIO range overflows the physical address representation.
    InvalidRange,
    DmaUnavailable,
    DmaInvalid,
}

/// A device register window granted to a driver domain.
#[derive(Debug, Clone, Copy)]
struct MmioRegion {
    phys_base: usize,
    pages: usize,
    /// The user virtual base at which the region is currently mapped, if any.
    mapped: Option<VAddr>,
}

/// An interrupt source granted to a driver domain. Delivery-side state
/// (pending/lifetime counters, the INTID→queue route) lives in the lock-free
/// tables below so interrupt context never takes a lock; this object holds
/// only the management-side state.
#[derive(Debug, Clone, Copy)]
struct InterruptObject {
    intid: u32,
    /// The completion queue readiness is delivered to, once bound.
    cq: Option<CqId>,
    /// The LP the source is routed to (set at bind time).
    target_lp: LpId,
}

#[derive(Debug, Clone, Copy)]
enum DeviceObject {
    Mmio(MmioRegion),
    Interrupt(InterruptObject),
    DmaDomain {
        id: u64,
    },
}

#[derive(Debug)]
struct AsDeviceCaps {
    caps: BTreeMap<DeviceCap, DeviceObject>,
}

impl AsDeviceCaps {
    fn new() -> Self {
        Self {
            caps: BTreeMap::new(),
        }
    }

    fn insert(&mut self, owner: AddressSpaceId, object: DeviceObject) -> DeviceCap {
        let id = crate::capability::allocate(owner, crate::capability::ObjectKind::Device);
        self.caps.insert(id, object);
        id
    }
}

static DEVICES: LazyLock<Mutex<BTreeMap<AddressSpaceId, AsDeviceCaps>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

// ---- lock-free interrupt delivery state -------------------------------------
//
// Interrupt context must never block on a kernel lock: the interrupted thread
// on the same core may hold it (architecture doc §10.2 durable-design note).
// The delivery path therefore works exclusively on this lock-free state:
//
// - `ROUTE_TABLE[intid]` packs the owning `(asid, cq)` of a bound interrupt (0 = unrouted; a driver
//   address space id is never 0, so a present route is always nonzero). Written by bind/close in
//   thread context, read atomically by `deliver_interrupt`.
// - `IRQ_PENDING`/`IRQ_COUNT` are the per-INTID coalescing counters.
// - `DEFERRED_WAKES` carries the packed `(asid, cq)` of each delivery out of interrupt context;
//   [`drain_deferred_wakes`] performs the actual `completion::wake` (which takes locks and may wake
//   threads) from thread context — the idle loop and cooperative yield both drain it.

/// One more than the highest INTID a driver interrupt may use.
pub(crate) const MAX_ROUTED_INTID: usize = 256;
/// The first GIC Shared Peripheral Interrupt. SGIs and PPIs are private to a
/// processor and cannot be delegated as device interrupts.
// AArch64 SPIs (the GIC's device interrupts) start at INTID 32; x86_64 Global
// System Interrupts are numbered from 0.
#[cfg(target_arch = "aarch64")]
const MIN_ROUTED_INTID: u32 = 32;
#[cfg(not(target_arch = "aarch64"))]
const MIN_ROUTED_INTID: u32 = 0;
/// The first LPI INTID delivered by the GIC ITS. LPIs are always numbered from
/// 8192 (see `cpu::isa::aarch64::interrupts::gic::lpi`).
#[cfg(target_arch = "aarch64")]
const LPI_INTID_BASE: u32 = 8192;
/// Number of LPI slots tracked by the routing tables (LPIs `LPI_INTID_BASE` ..
/// `LPI_INTID_BASE + LPI_SLOTS - 1`).
#[cfg(target_arch = "aarch64")]
const LPI_SLOTS: usize = 32;
/// x86_64 MSI interrupts are numbered in a synthetic range above the wired GSI
/// space: `MSI_INTID_BASE` .. `MSI_INTID_BASE + MSI_SLOTS - 1`.
#[cfg(target_arch = "x86_64")]
pub(crate) const MSI_INTID_BASE: u32 = 256;
#[cfg(target_arch = "x86_64")]
const MSI_SLOTS: usize = 220;
/// Total routing-table slots: the SPI/GSI space plus the LPI (AArch64) or MSI
/// (x86_64) window.
#[cfg(target_arch = "aarch64")]
pub(crate) const TOTAL_ROUTE_SLOTS: usize = MAX_ROUTED_INTID + LPI_SLOTS;
#[cfg(target_arch = "x86_64")]
pub(crate) const TOTAL_ROUTE_SLOTS: usize = MAX_ROUTED_INTID + MSI_SLOTS;

/// Map an INTID to a routing-table slot: the SPI/GSI space is indexed directly
/// and the LPI (AArch64) or MSI (x86_64) window is packed after it. Returns
/// `None` for unroutable INTIDs.
fn route_slot(intid: u32) -> Option<usize> {
    let i = intid as usize;
    if i < MAX_ROUTED_INTID {
        return Some(i);
    }
    #[cfg(target_arch = "aarch64")]
    if i >= LPI_INTID_BASE as usize && i < LPI_INTID_BASE as usize + LPI_SLOTS {
        return Some(MAX_ROUTED_INTID + (i - LPI_INTID_BASE as usize));
    }
    #[cfg(target_arch = "x86_64")]
    if i >= MSI_INTID_BASE as usize && i < MSI_INTID_BASE as usize + MSI_SLOTS {
        return Some(MAX_ROUTED_INTID + (i - MSI_INTID_BASE as usize));
    }
    None
}

static ROUTE_TABLE: [AtomicU64; TOTAL_ROUTE_SLOTS] =
    [const { AtomicU64::new(0) }; TOTAL_ROUTE_SLOTS];
/// Generation of each interrupt route. It advances on every bind and unroute,
/// fencing deferred wakes that were queued for a previous driver lifetime.
static ROUTE_GENERATION: [AtomicU64; TOTAL_ROUTE_SLOTS] =
    [const { AtomicU64::new(0) }; TOTAL_ROUTE_SLOTS];
static IRQ_PENDING: [AtomicU32; TOTAL_ROUTE_SLOTS] =
    [const { AtomicU32::new(0) }; TOTAL_ROUTE_SLOTS];
static IRQ_COUNT: [AtomicU64; TOTAL_ROUTE_SLOTS] = [const { AtomicU64::new(0) }; TOTAL_ROUTE_SLOTS];

/// Deferred `(asid, cq)` wakes queued by interrupt context, delivered by
/// [`drain_deferred_wakes`] from thread context. Wakes coalesce (§9.4), so
/// the bound capacity only needs to cover the number of distinct driver
/// queues with generous headroom.
#[derive(Clone, Copy)]
struct DeferredWake {
    intid: u32,
    route_generation: u64,
}

static DEFERRED_WAKES: LazyLock<ConcurrentQueue<DeferredWake>> =
    LazyLock::new(|| ConcurrentQueue::bounded(TOTAL_ROUTE_SLOTS));

/// Force construction of interrupt-ingress state before scheduler preemption
/// or device IRQ delivery is enabled. `spin::LazyLock` itself uses spinning;
/// first use from a preempted/IRQ context would otherwise have the same owner
/// progress hazard as a plain runtime spin lock.
pub fn prepare_interrupt_ingress() {
    LazyLock::force(&DEFERRED_WAKES);
}

fn pack_route(asid: AddressSpaceId, cq: CqId) -> u64 {
    debug_assert!(asid != 0 && u32::try_from(asid).is_ok(), "driver asid must pack into 32 bits");
    ((asid as u64) << 32) | cq as u64
}

fn unpack_route(packed: u64) -> (AddressSpaceId, CqId) {
    ((packed >> 32) as AddressSpaceId, (packed & 0xffff_ffff) as CqId)
}

// ---- arch glue -------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
fn arch_map_user_mmio(
    asid: AddressSpaceId,
    vaddr: VAddr,
    frame: PAddr,
    writable: bool,
) -> Result<(), ()> {
    let mut table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    let address_space = table.get_mut(asid).map_err(|_| ())?;
    address_space.map_user_mmio_page(vaddr, frame, writable).map_err(|_| ())
}

#[cfg(not(target_arch = "aarch64"))]
fn arch_map_user_mmio(
    asid: AddressSpaceId,
    vaddr: VAddr,
    frame: PAddr,
    writable: bool,
) -> Result<(), ()> {
    use crate::{
        cpu::isa::interface::memory::AddressSpaceInterface,
        memory::linear::{
            MemoryMapping,
            PageType,
        },
    };
    let mut table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    let address_space = table.get_mut(asid).map_err(|_| ())?;
    let page_type = if writable {
        PageType::UserData
    } else {
        PageType::UserRoData
    };
    // `map_existing_page` installs the mapping without zeroing the target, so
    // the device register block is left untouched.
    address_space
        .map_existing_page(MemoryMapping {
            vaddr,
            paddr: frame,
            page_type,
        })
        .map_err(|_| ())
}

fn arch_unmap(asid: AddressSpaceId, vaddr: VAddr) -> Result<(), ()> {
    use crate::cpu::isa::interface::memory::AddressSpaceInterface;
    let mut table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    let address_space = table.get_mut(asid).map_err(|_| ())?;
    address_space.unmap_page(vaddr).map(|_| ()).map_err(|_| ())
}

#[cfg(target_arch = "aarch64")]
fn arch_enable_irq(intid: u32, target_lp: LpId) {
    crate::cpu::isa::interrupts::gic::enable_spi(intid, target_lp);
}

#[cfg(target_arch = "aarch64")]
fn arch_disable_irq(intid: u32) {
    crate::cpu::isa::interrupts::gic::disable_spi(intid);
}

#[cfg(target_arch = "aarch64")]
fn arch_clear_irq_pending(intid: u32) {
    crate::cpu::isa::interrupts::gic::clear_spi_pending(intid);
}

#[cfg(target_arch = "x86_64")]
fn arch_enable_irq(intid: u32, target_lp: LpId) {
    crate::cpu::isa::interrupts::device_irq::enable_irq(intid, target_lp);
}

#[cfg(target_arch = "x86_64")]
fn arch_disable_irq(intid: u32) {
    crate::cpu::isa::interrupts::device_irq::disable_irq(intid);
}

#[cfg(target_arch = "x86_64")]
fn arch_clear_irq_pending(intid: u32) {
    crate::cpu::isa::interrupts::device_irq::clear_irq_pending(intid);
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
fn arch_enable_irq(_intid: u32, _target_lp: LpId) {}
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
fn arch_disable_irq(_intid: u32) {}
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
fn arch_clear_irq_pending(_intid: u32) {}

/// Whether the kernel's MSI mechanism is available on this platform.
pub fn msi_available() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        crate::cpu::isa::interrupts::gic::msi_available()
    }
    #[cfg(target_arch = "x86_64")]
    {
        crate::cpu::isa::interrupts::device_irq::msi_available()
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        false
    }
}

/// Allocate one MSI for the PCI function identified by `device_id` (its PCI
/// Requester ID), returning the address/data pair written to raise it.
pub fn allocate_msi(
    device_id: u32,
) -> Option<
    crate::device_management::drivers::busses::pci_express::ecam::capabilities::standard::msi::MsiMessage,
>{
    #[cfg(target_arch = "aarch64")]
    {
        crate::cpu::isa::interrupts::gic::allocate_msi(device_id)
    }
    #[cfg(target_arch = "x86_64")]
    {
        crate::cpu::isa::interrupts::device_irq::allocate_msi(device_id)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = device_id;
        None
    }
}

// ---- grants (kernel-side, supervisor only) ---------------------------------

/// Grant a page-granular MMIO region to `owner`. `phys_base` and the region
/// size are page-aligned by the caller; the driver later maps it with
/// [`mmio_map`]. This is minted only kernel-side (the supervisor), never
/// through a syscall.
pub fn grant_mmio(
    owner: AddressSpaceId,
    phys_base: usize,
    pages: usize,
) -> Result<DeviceCap, DeviceError> {
    if !phys_base.is_multiple_of(PAGE_SIZE) {
        return Err(DeviceError::NotPageAligned);
    }
    if pages == 0 {
        return Err(DeviceError::MapFailed);
    }
    let byte_len = pages.checked_mul(PAGE_SIZE).ok_or(DeviceError::InvalidRange)?;
    phys_base.checked_add(byte_len).ok_or(DeviceError::InvalidRange)?;
    let mut devices = DEVICES.lock();
    let caps = devices.entry(owner).or_insert_with(AsDeviceCaps::new);
    Ok(caps.insert(
        owner,
        DeviceObject::Mmio(MmioRegion {
            phys_base,
            pages,
            mapped: None,
        }),
    ))
}

/// Grant an interrupt source to `owner`. The driver binds it to a completion
/// queue with [`interrupt_bind_cq`], which arms and routes the interrupt.
pub fn grant_interrupt(owner: AddressSpaceId, intid: u32) -> Result<DeviceCap, DeviceError> {
    let mut devices = DEVICES.lock();
    if owner == 0 || u32::try_from(owner).is_err() {
        return Err(DeviceError::InvalidAddressSpace);
    }
    if intid < MIN_ROUTED_INTID || route_slot(intid).is_none() {
        return Err(DeviceError::InvalidInterrupt);
    }
    if devices.values().any(|caps| {
        caps.caps
            .values()
            .any(|object| matches!(object, DeviceObject::Interrupt(irq) if irq.intid == intid))
    }) {
        return Err(DeviceError::InterruptInUse);
    }
    let caps = devices.entry(owner).or_insert_with(AsDeviceCaps::new);
    Ok(caps.insert(
        owner,
        DeviceObject::Interrupt(InterruptObject {
            intid,
            cq: None,
            target_lp: 0,
        }),
    ))
}

pub fn grant_dma_domain(
    owner: AddressSpaceId,
    requester_id: u32,
    msi_address: Option<u64>,
) -> Result<DeviceCap, DeviceError> {
    let sid = dma::stream_id(requester_id).map_err(|_| DeviceError::DmaUnavailable)?;
    let id = dma::create_domain(sid, msi_address).map_err(|_| DeviceError::DmaUnavailable)?;
    let mut devices = DEVICES.lock();
    let caps = devices.entry(owner).or_insert_with(AsDeviceCaps::new);
    Ok(caps.insert(
        owner,
        DeviceObject::DmaDomain {
            id,
        },
    ))
}

/// Resolve a PCI requester id to the DMA stream id used by the platform's
/// IOMMU (SMMU on AArch64; VT-d source id on x86_64).
pub fn stream_id(requester_id: u32) -> Result<u32, DeviceError> {
    dma::stream_id(requester_id).map_err(|_| DeviceError::DmaUnavailable)
}

/// Number of DMA translation faults observed since boot.
pub fn fault_count() -> u64 {
    dma::fault_count()
}

pub fn dma_map(
    asid: AddressSpaceId,
    domain_cap: DeviceCap,
    memory_cap: u64,
    direction: u32,
) -> Result<u64, DeviceError> {
    dma_map_with_ownership(asid, domain_cap, memory_cap, direction, false)
}

/// Map memory for exclusive device ownership. The memory object must have no
/// CPU mappings, active lends, or other DMA pins, and the kernel rejects all
/// new CPU mappings and lends until the IOMMU mapping is removed.
pub fn dma_map_exclusive(
    asid: AddressSpaceId,
    domain_cap: DeviceCap,
    memory_cap: u64,
    direction: u32,
) -> Result<u64, DeviceError> {
    dma_map_with_ownership(asid, domain_cap, memory_cap, direction, true)
}

fn dma_map_with_ownership(
    asid: AddressSpaceId,
    domain_cap: DeviceCap,
    memory_cap: u64,
    direction: u32,
    exclusive: bool,
) -> Result<u64, DeviceError> {
    let direction = dma::Direction::from_bits(direction).map_err(|_| DeviceError::DmaInvalid)?;
    let id = {
        let mut devices = DEVICES.lock();
        let object = lookup_mut(&mut devices, asid, domain_cap)?;
        let DeviceObject::DmaDomain {
            id,
        } = object
        else {
            return Err(DeviceError::WrongType);
        };
        *id
    };
    dma::map(id, asid, memory_cap, direction, exclusive).map_err(|_| DeviceError::DmaInvalid)
}

pub fn dma_unmap(
    asid: AddressSpaceId,
    domain_cap: DeviceCap,
    iova: u64,
) -> Result<(), DeviceError> {
    let id = {
        let mut devices = DEVICES.lock();
        let object = lookup_mut(&mut devices, asid, domain_cap)?;
        let DeviceObject::DmaDomain {
            id,
        } = object
        else {
            return Err(DeviceError::WrongType);
        };
        *id
    };
    dma::unmap(id, iova).map_err(|_| DeviceError::DmaInvalid)
}

// ---- MMIO operations -------------------------------------------------------

/// Map an MMIO region capability into the caller's address space at `base`,
/// as Device-nGnRnE memory reachable from EL0.
pub fn mmio_map(
    asid: AddressSpaceId,
    cap: DeviceCap,
    base: VAddr,
    writable: bool,
) -> Result<(), DeviceError> {
    if !base.is_aligned_to(PAGE_SIZE) {
        return Err(DeviceError::NotPageAligned);
    }
    // Serialize the capability check, page-table update, and mapping record
    // against teardown and ASID reuse.
    let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    let mut devices = DEVICES.lock();
    let object = lookup_mut(&mut devices, asid, cap)?;
    let DeviceObject::Mmio(region) = object else {
        return Err(DeviceError::WrongType);
    };
    if region.mapped.is_some() {
        return Err(DeviceError::AlreadyMapped);
    }
    let (phys_base, pages) = (region.phys_base, region.pages);
    map_mmio_at(&mut devices, asid, cap, base, phys_base, pages, writable)
}

/// Map a device's MMIO region into the caller's address space at a
/// kernel-assigned scratch address (the same window the memory layer uses,
/// so device and memory mappings can never collide) and return it.
pub fn mmio_map_any(
    asid: AddressSpaceId,
    cap: DeviceCap,
    writable: bool,
) -> Result<VAddr, DeviceError> {
    // Take lifecycle before DEVICES: teardown uses the same ordering. The
    // scratch reservation and MMIO mapping must target one AS generation.
    let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    let mut devices = DEVICES.lock();
    let object = lookup_mut(&mut devices, asid, cap)?;
    let DeviceObject::Mmio(region) = object else {
        return Err(DeviceError::WrongType);
    };
    let (phys_base, pages) = (region.phys_base, region.pages);
    let base =
        crate::memory::object::reserve_scratch(asid, pages).map_err(|_| DeviceError::MapFailed)?;
    map_mmio_at(&mut devices, asid, cap, base, phys_base, pages, writable)?;
    Ok(base)
}

fn map_mmio_at(
    devices: &mut impl core::ops::DerefMut<Target = BTreeMap<AddressSpaceId, AsDeviceCaps>>,
    asid: AddressSpaceId,
    cap: DeviceCap,
    base: VAddr,
    phys_base: usize,
    pages: usize,
    writable: bool,
) -> Result<(), DeviceError> {
    for index in 0..pages {
        let vaddr = base + (index * PAGE_SIZE);
        let frame = PAddr::from((phys_base + index * PAGE_SIZE) as u64);
        if arch_map_user_mmio(asid, vaddr, frame, writable).is_err() {
            for cleanup in 0..index {
                let _ = arch_unmap(asid, base + (cleanup * PAGE_SIZE));
            }
            return Err(DeviceError::MapFailed);
        }
    }

    // Re-borrow to record the mapping (the map may have taken the AS table lock).
    if let Ok(DeviceObject::Mmio(region)) = lookup_mut(devices, asid, cap) {
        region.mapped = Some(base);
    }
    Ok(())
}

/// Unmap a previously mapped MMIO region from the caller's address space.
pub fn mmio_unmap(asid: AddressSpaceId, cap: DeviceCap) -> Result<(), DeviceError> {
    let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    let mut devices = DEVICES.lock();
    let object = lookup_mut(&mut devices, asid, cap)?;
    let DeviceObject::Mmio(region) = object else {
        return Err(DeviceError::WrongType);
    };
    let base = region.mapped.ok_or(DeviceError::NotMapped)?;
    let pages = region.pages;
    for index in 0..pages {
        let _ = arch_unmap(asid, base + (index * PAGE_SIZE));
    }
    if let Ok(DeviceObject::Mmio(region)) = lookup_mut(&mut devices, asid, cap) {
        region.mapped = None;
    }
    Ok(())
}

// ---- interrupt operations --------------------------------------------------

/// Bind an interrupt capability to one of the caller's completion queues and
/// arm the source. After binding, each delivered interrupt masks the source,
/// counts it, and (from thread context) posts a coalesced wake to `cq` so the
/// driver shard — blocked in a single `CQ_WAIT` — becomes runnable
/// (architecture doc §10.2, unified shard wait of §7).
pub fn interrupt_bind_cq(
    asid: AddressSpaceId,
    cap: DeviceCap,
    cq: CqId,
) -> Result<(), DeviceError> {
    let target_lp = get_lp_id();
    let mut devices = DEVICES.lock();
    let object = lookup_mut(&mut devices, asid, cap)?;
    let DeviceObject::Interrupt(irq) = object else {
        return Err(DeviceError::WrongType);
    };
    if irq.cq.is_some() {
        return Err(DeviceError::AlreadyBound);
    }
    let intid = irq.intid;
    irq.cq = Some(cq);
    irq.target_lp = target_lp;

    // Publish the route and reset the coalescing counter before arming, so a
    // delivery that races the enable observes a consistent route. Keep the
    // capability-table lock through the architecture operation so a concurrent
    // close cannot remove the capability and leave an orphaned route.
    let slot = route_slot(intid).expect("[dev] bound interrupt has no routing slot");
    IRQ_PENDING[slot].store(0, Ordering::Release);
    ROUTE_GENERATION[slot].fetch_add(1, Ordering::AcqRel);
    ROUTE_TABLE[slot].store(pack_route(asid, cq), Ordering::Release);
    arch_enable_irq(intid, target_lp);
    Ok(())
}

/// Acknowledge handling of an interrupt: clear the pending count and re-arm
/// (unmask) the source so the next interrupt can be delivered. Returns the
/// number of coalesced interrupts consumed since the last acknowledgement.
pub fn interrupt_ack(asid: AddressSpaceId, cap: DeviceCap) -> Result<u32, DeviceError> {
    let mut devices = DEVICES.lock();
    let object = lookup_mut(&mut devices, asid, cap)?;
    let DeviceObject::Interrupt(irq) = object else {
        return Err(DeviceError::WrongType);
    };
    if irq.cq.is_none() {
        return Err(DeviceError::NotBound);
    }
    let (intid, target_lp) = (irq.intid, irq.target_lp);
    let slot = route_slot(intid).expect("[dev] acknowledged interrupt has no routing slot");
    let consumed = IRQ_PENDING[slot].swap(0, Ordering::AcqRel);
    // Re-arm the source (it was masked on delivery). Holding DEVICES prevents
    // close from racing this operation and re-enabling a reclaimed source.
    arch_enable_irq(intid, target_lp);
    Ok(consumed)
}

/// Inspection: the number of interrupts pending since the last acknowledgement
/// and the lifetime interrupt count.
pub fn interrupt_status(asid: AddressSpaceId, cap: DeviceCap) -> Result<(u32, u64), DeviceError> {
    let mut devices = DEVICES.lock();
    let object = lookup_mut(&mut devices, asid, cap)?;
    let DeviceObject::Interrupt(irq) = object else {
        return Err(DeviceError::WrongType);
    };
    let intid = irq.intid;
    let slot = route_slot(intid).expect("[dev] inspected interrupt has no routing slot");
    Ok((IRQ_PENDING[slot].load(Ordering::Acquire), IRQ_COUNT[slot].load(Ordering::Acquire)))
}

/// Mask an interrupt source and remove its route. Idempotent.
fn unroute_interrupt(intid: u32) {
    if let Some(slot) = route_slot(intid) {
        // Invalidate queued wakes before clearing the route. A delivery racing
        // these stores may enqueue either generation, but neither can match a
        // subsequently rebound route.
        ROUTE_GENERATION[slot].fetch_add(1, Ordering::AcqRel);
        ROUTE_TABLE[slot].store(0, Ordering::Release);
    }
    arch_disable_irq(intid);
}

// ---- teardown --------------------------------------------------------------

/// Close a device capability, releasing its resources: an MMIO region is
/// unmapped, an interrupt source is masked and its route removed.
pub fn close_cap(asid: AddressSpaceId, cap: DeviceCap) -> Result<(), DeviceError> {
    let object = {
        let mut devices = DEVICES.lock();
        devices
            .get_mut(&asid)
            .and_then(|caps| caps.caps.remove(&cap))
            .ok_or(DeviceError::UnknownCapability)?
    };
    match object {
        DeviceObject::Mmio(region) => {
            if let Some(base) = region.mapped {
                for index in 0..region.pages {
                    let _ = arch_unmap(asid, base + (index * PAGE_SIZE));
                }
            }
        }
        DeviceObject::Interrupt(irq) => unroute_interrupt(irq.intid),
        DeviceObject::DmaDomain {
            id,
        } => {
            if dma::destroy_domain(id).is_err() {
                let mut devices = DEVICES.lock();
                devices.entry(asid).or_insert_with(AsDeviceCaps::new).caps.insert(cap, object);
                return Err(DeviceError::DmaInvalid);
            }
        }
    }
    let revoked = crate::capability::remove(asid, cap, crate::capability::ObjectKind::Device);
    assert!(revoked, "device payload capability was absent from unified table");
    Ok(())
}

/// Inspection: the owning address space of the interrupt route for `intid`,
/// if any. A driver's route is installed by [`interrupt_bind_cq`] and removed
/// on [`close_cap`] or [`close_address_space`], so this reports whether a
/// live driver currently owns the interrupt — used to verify that device
/// authority is reclaimed when a driver domain is torn down (architecture
/// doc §13, success criterion 9).
pub fn interrupt_route_owner(intid: u32) -> Option<AddressSpaceId> {
    let slot = route_slot(intid)?;
    match ROUTE_TABLE[slot].load(Ordering::Acquire) {
        0 => None,
        packed => Some(unpack_route(packed).0),
    }
}

/// Reclaim every device capability owned by `asid` on address-space teardown:
/// unmap MMIO regions, mask and unroute interrupt sources. Called from
/// `close_user_address_space`.
pub fn close_address_space(asid: AddressSpaceId) {
    let objects = {
        let mut devices = DEVICES.lock();
        match devices.remove(&asid) {
            Some(caps) => caps.caps,
            None => return,
        }
    };
    for cap in objects.keys() {
        assert!(
            crate::capability::remove(asid, *cap, crate::capability::ObjectKind::Device),
            "device payload capability was absent from unified table"
        );
    }
    for object in objects.values() {
        match object {
            DeviceObject::Mmio(region) => {
                if let Some(base) = region.mapped {
                    for index in 0..region.pages {
                        let _ = arch_unmap(asid, base + (index * PAGE_SIZE));
                    }
                }
            }
            DeviceObject::Interrupt(irq) => unroute_interrupt(irq.intid),
            DeviceObject::DmaDomain {
                id,
            } => {
                if let Err(error) = dma::destroy_domain(*id) {
                    crate::logln!(
                        "[dma] quarantining DMA domain {} after teardown failure: {:?}",
                        id,
                        error
                    );
                }
            }
        }
    }
}

// ---- interrupt delivery (interrupt context) --------------------------------

/// Steer a delivered INTID to its owning driver domain. Called from the
/// architecture IRQ dispatcher for INTIDs not claimed by the kernel itself.
///
/// Runs in interrupt context and is **entirely lock-free**: it reads the
/// route atomically, masks the source with MMIO only (so a level-triggered
/// device does not storm the CPU until the driver acknowledges), bumps the
/// coalescing counters atomically, and queues a deferred wake. The actual
/// `completion::wake` — which takes locks and may make threads runnable — is
/// performed later by [`drain_deferred_wakes`] from thread context, so the
/// interrupted thread can never be holding a lock this path needs
/// (architecture doc §10.2 durable-design requirement).
///
/// Returns `true` if the INTID was claimed by a bound driver interrupt.
pub fn deliver_interrupt(intid: u32) -> bool {
    let Some(slot) = route_slot(intid) else {
        return false;
    };
    let route_generation = ROUTE_GENERATION[slot].load(Ordering::Acquire);
    let packed = ROUTE_TABLE[slot].load(Ordering::Acquire);
    if packed == 0 {
        return false;
    }

    // Mask and de-pend the source (MMIO only) until the driver acknowledges.
    arch_disable_irq(intid);
    arch_clear_irq_pending(intid);

    IRQ_PENDING[slot].fetch_add(1, Ordering::AcqRel);
    IRQ_COUNT[slot].fetch_add(1, Ordering::AcqRel);

    let (asid, cq) = unpack_route(packed);
    sched_trace!(
        "[sched] irq-deliver INTID={} count={} -> AS={} CQ={}",
        intid,
        IRQ_COUNT[slot].load(Ordering::Acquire),
        asid,
        cq
    );

    // Hand the coalesced readiness wake to thread context. A full queue means
    // an equivalent wake is already pending delivery, so dropping is safe.
    let _ = DEFERRED_WAKES.push(DeferredWake {
        intid,
        route_generation,
    });
    true
}

/// Deliver any wakes queued by [`deliver_interrupt`]. Must be called from
/// thread context (it calls `completion::wake`, which takes locks and may
/// make threads runnable); the idle loop and cooperative `yield_lp` both call
/// it, so a driver blocked in `CQ_WAIT` is released promptly once its LP has
/// nothing else to run.
pub fn drain_deferred_wakes() -> usize {
    let mut drained = 0u32;
    while let Ok(wake) = DEFERRED_WAKES.pop() {
        let Some(index) = route_slot(wake.intid) else {
            continue;
        };
        if ROUTE_GENERATION[index].load(Ordering::Acquire) != wake.route_generation {
            continue;
        }
        let packed = ROUTE_TABLE[index].load(Ordering::Acquire);
        if packed == 0 {
            continue;
        }
        let (asid, cq) = unpack_route(packed);
        sched_trace!("[sched] drain-wake AS={} CQ={}", asid, cq);
        crate::completion::wake(asid, cq);
        drained += 1;
    }
    if drained > 0 && SCHED_TRACE {
        logln!("[sched] drained {} deferred wake(s)", drained);
    }
    drained as usize
}

// ---- helpers ---------------------------------------------------------------

fn lookup_mut(
    devices: &mut BTreeMap<AddressSpaceId, AsDeviceCaps>,
    asid: AddressSpaceId,
    cap: DeviceCap,
) -> Result<&mut DeviceObject, DeviceError> {
    if !crate::capability::contains(asid, cap, crate::capability::ObjectKind::Device) {
        return Err(DeviceError::UnknownCapability);
    }
    devices
        .get_mut(&asid)
        .and_then(|caps| caps.caps.get_mut(&cap))
        .ok_or(DeviceError::UnknownCapability)
}
