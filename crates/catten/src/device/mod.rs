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

use alloc::collections::BTreeMap;
use core::sync::atomic::{
    AtomicU32,
    AtomicU64,
    Ordering,
};

use concurrent_queue::ConcurrentQueue;
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
}

#[derive(Debug)]
struct AsDeviceCaps {
    next: DeviceCap,
    caps: BTreeMap<DeviceCap, DeviceObject>,
}

impl AsDeviceCaps {
    fn new() -> Self {
        Self {
            next: 1,
            caps: BTreeMap::new(),
        }
    }

    fn insert(&mut self, object: DeviceObject) -> DeviceCap {
        let id = self.next;
        self.next = self.next.checked_add(1).expect("device capability id overflow");
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
const MAX_ROUTED_INTID: usize = 256;

static ROUTE_TABLE: [AtomicU64; MAX_ROUTED_INTID] = [const { AtomicU64::new(0) }; MAX_ROUTED_INTID];
static IRQ_PENDING: [AtomicU32; MAX_ROUTED_INTID] = [const { AtomicU32::new(0) }; MAX_ROUTED_INTID];
static IRQ_COUNT: [AtomicU64; MAX_ROUTED_INTID] = [const { AtomicU64::new(0) }; MAX_ROUTED_INTID];

/// Deferred `(asid, cq)` wakes queued by interrupt context, delivered by
/// [`drain_deferred_wakes`] from thread context. Wakes coalesce (§9.4), so
/// the bound capacity only needs to cover the number of distinct driver
/// queues with generous headroom.
static DEFERRED_WAKES: LazyLock<ConcurrentQueue<u64>> =
    LazyLock::new(|| ConcurrentQueue::bounded(MAX_ROUTED_INTID));

/// Force construction of interrupt-ingress state before scheduler preemption
/// or device IRQ delivery is enabled. `spin::LazyLock` itself uses spinning;
/// first use from a preempted/IRQ context would otherwise have the same owner
/// progress hazard as a plain runtime spin lock.
pub fn prepare_interrupt_ingress() {
    LazyLock::force(&DEFERRED_WAKES);
}

fn pack_route(asid: AddressSpaceId, cq: CqId) -> u64 {
    debug_assert!(asid != 0 && asid <= u32::MAX as usize, "driver asid must pack into 32 bits");
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
    _asid: AddressSpaceId,
    _vaddr: VAddr,
    _frame: PAddr,
    _writable: bool,
) -> Result<(), ()> {
    Err(())
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

#[cfg(not(target_arch = "aarch64"))]
fn arch_enable_irq(_intid: u32, _target_lp: LpId) {}
#[cfg(not(target_arch = "aarch64"))]
fn arch_disable_irq(_intid: u32) {}
#[cfg(not(target_arch = "aarch64"))]
fn arch_clear_irq_pending(_intid: u32) {}

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
    if phys_base % PAGE_SIZE != 0 {
        return Err(DeviceError::NotPageAligned);
    }
    if pages == 0 {
        return Err(DeviceError::MapFailed);
    }
    let mut devices = DEVICES.lock();
    let caps = devices.entry(owner).or_insert_with(AsDeviceCaps::new);
    Ok(caps.insert(DeviceObject::Mmio(MmioRegion {
        phys_base,
        pages,
        mapped: None,
    })))
}

/// Grant an interrupt source to `owner`. The driver binds it to a completion
/// queue with [`interrupt_bind_cq`], which arms and routes the interrupt.
pub fn grant_interrupt(owner: AddressSpaceId, intid: u32) -> Result<DeviceCap, DeviceError> {
    let mut devices = DEVICES.lock();
    let caps = devices.entry(owner).or_insert_with(AsDeviceCaps::new);
    Ok(caps.insert(DeviceObject::Interrupt(InterruptObject {
        intid,
        cq: None,
        target_lp: 0,
    })))
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
    let mut devices = DEVICES.lock();
    let object = lookup_mut(&mut devices, asid, cap)?;
    let DeviceObject::Mmio(region) = object else {
        return Err(DeviceError::WrongType);
    };
    if region.mapped.is_some() {
        return Err(DeviceError::AlreadyMapped);
    }
    let (phys_base, pages) = (region.phys_base, region.pages);

    let mut mapped = 0usize;
    for index in 0..pages {
        let vaddr = base + (index * PAGE_SIZE);
        let frame = PAddr::from((phys_base + index * PAGE_SIZE) as u64);
        if arch_map_user_mmio(asid, vaddr, frame, writable).is_err() {
            for cleanup in 0..mapped {
                let _ = arch_unmap(asid, base + (cleanup * PAGE_SIZE));
            }
            return Err(DeviceError::MapFailed);
        }
        mapped += 1;
    }

    // Re-borrow to record the mapping (the map may have taken the AS table lock).
    if let Ok(DeviceObject::Mmio(region)) = lookup_mut(&mut devices, asid, cap) {
        region.mapped = Some(base);
    }
    Ok(())
}

/// Unmap a previously mapped MMIO region from the caller's address space.
pub fn mmio_unmap(asid: AddressSpaceId, cap: DeviceCap) -> Result<(), DeviceError> {
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
    let intid;
    let target_lp = get_lp_id();
    {
        let mut devices = DEVICES.lock();
        let object = lookup_mut(&mut devices, asid, cap)?;
        let DeviceObject::Interrupt(irq) = object else {
            return Err(DeviceError::WrongType);
        };
        if irq.cq.is_some() {
            return Err(DeviceError::AlreadyBound);
        }
        if irq.intid as usize >= MAX_ROUTED_INTID {
            return Err(DeviceError::InvalidInterrupt);
        }
        irq.cq = Some(cq);
        irq.target_lp = target_lp;
        intid = irq.intid;
    }
    // Publish the route and reset the coalescing counter before arming, so a
    // delivery that races the enable observes a consistent route.
    IRQ_PENDING[intid as usize].store(0, Ordering::Release);
    ROUTE_TABLE[intid as usize].store(pack_route(asid, cq), Ordering::Release);
    arch_enable_irq(intid, target_lp);
    Ok(())
}

/// Acknowledge handling of an interrupt: clear the pending count and re-arm
/// (unmask) the source so the next interrupt can be delivered. Returns the
/// number of coalesced interrupts consumed since the last acknowledgement.
pub fn interrupt_ack(asid: AddressSpaceId, cap: DeviceCap) -> Result<u32, DeviceError> {
    let (intid, target_lp) = {
        let mut devices = DEVICES.lock();
        let object = lookup_mut(&mut devices, asid, cap)?;
        let DeviceObject::Interrupt(irq) = object else {
            return Err(DeviceError::WrongType);
        };
        if irq.cq.is_none() {
            return Err(DeviceError::NotBound);
        }
        (irq.intid, irq.target_lp)
    };
    let consumed = IRQ_PENDING[intid as usize].swap(0, Ordering::AcqRel);
    // Re-arm the source (it was masked on delivery).
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
    let intid = irq.intid as usize;
    Ok((IRQ_PENDING[intid].load(Ordering::Acquire), IRQ_COUNT[intid].load(Ordering::Acquire)))
}

/// Mask an interrupt source and remove its route. Idempotent.
fn unroute_interrupt(intid: u32) {
    if (intid as usize) < MAX_ROUTED_INTID {
        ROUTE_TABLE[intid as usize].store(0, Ordering::Release);
    }
    arch_disable_irq(intid);
}

// ---- teardown --------------------------------------------------------------

/// Close a device capability, releasing its resources: an MMIO region is
/// unmapped, an interrupt source is masked and its route removed.
pub fn close_cap(asid: AddressSpaceId, cap: DeviceCap) -> Result<(), DeviceError> {
    let object = {
        let devices = DEVICES.lock();
        devices
            .get(&asid)
            .and_then(|caps| caps.caps.get(&cap))
            .copied()
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
    }
    let mut devices = DEVICES.lock();
    devices
        .get_mut(&asid)
        .and_then(|caps| caps.caps.remove(&cap))
        .map(|_| ())
        .ok_or(DeviceError::UnknownCapability)
}

/// Inspection: the owning address space of the interrupt route for `intid`,
/// if any. A driver's route is installed by [`interrupt_bind_cq`] and removed
/// on [`close_cap`] or [`close_address_space`], so this reports whether a
/// live driver currently owns the interrupt — used to verify that device
/// authority is reclaimed when a driver domain is torn down (architecture
/// doc §13, success criterion 9).
pub fn interrupt_route_owner(intid: u32) -> Option<AddressSpaceId> {
    if intid as usize >= MAX_ROUTED_INTID {
        return None;
    }
    match ROUTE_TABLE[intid as usize].load(Ordering::Acquire) {
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
    if intid as usize >= MAX_ROUTED_INTID {
        return false;
    }
    let packed = ROUTE_TABLE[intid as usize].load(Ordering::Acquire);
    if packed == 0 {
        return false;
    }

    // Mask and de-pend the source (MMIO only) until the driver acknowledges.
    arch_disable_irq(intid);
    arch_clear_irq_pending(intid);

    IRQ_PENDING[intid as usize].fetch_add(1, Ordering::AcqRel);
    IRQ_COUNT[intid as usize].fetch_add(1, Ordering::AcqRel);

    let (asid, cq) = unpack_route(packed);
    sched_trace!(
        "[sched] irq-deliver INTID={} count={} -> AS={} CQ={}",
        intid,
        IRQ_COUNT[intid as usize].load(Ordering::Acquire),
        asid,
        cq
    );

    // Hand the coalesced readiness wake to thread context. A full queue means
    // an equivalent wake is already pending delivery, so dropping is safe.
    let _ = DEFERRED_WAKES.push(packed);
    true
}

/// Deliver any wakes queued by [`deliver_interrupt`]. Must be called from
/// thread context (it calls `completion::wake`, which takes locks and may
/// make threads runnable); the idle loop and cooperative `yield_lp` both call
/// it, so a driver blocked in `CQ_WAIT` is released promptly once its LP has
/// nothing else to run.
pub fn drain_deferred_wakes() {
    let mut drained = 0u32;
    while let Ok(packed) = DEFERRED_WAKES.pop() {
        let (asid, cq) = unpack_route(packed);
        sched_trace!("[sched] drain-wake AS={} CQ={}", asid, cq);
        crate::completion::wake(asid, cq);
        drained += 1;
    }
    if drained > 0 && SCHED_TRACE {
        logln!("[sched] drained {} deferred wake(s)", drained);
    }
}

// ---- helpers ---------------------------------------------------------------

fn lookup_mut<'a>(
    devices: &'a mut BTreeMap<AddressSpaceId, AsDeviceCaps>,
    asid: AddressSpaceId,
    cap: DeviceCap,
) -> Result<&'a mut DeviceObject, DeviceError> {
    devices
        .get_mut(&asid)
        .and_then(|caps| caps.caps.get_mut(&cap))
        .ok_or(DeviceError::UnknownCapability)
}
