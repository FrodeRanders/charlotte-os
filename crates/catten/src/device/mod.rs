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
//! - [`DeviceObject::Mmio`] — a page-granular device register window that a
//!   driver can map into its own address space as Device-nGnRnE memory,
//!   reachable from EL0 under its own page table;
//! - [`DeviceObject::Interrupt`] — an interrupt source whose readiness is
//!   delivered to the driver's completion queue. This reuses the same
//!   notification machinery as endpoint readiness (Phase 7): an IRQ posts a
//!   coalesced wake to the bound CQ (§16.3: readiness is a notification to
//!   inspect state, not a completion record).
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

use spin::{
    LazyLock,
    Mutex,
};

use crate::{
    completion::CqId,
    cpu::isa::{
        interface::memory::address::Address,
        lp::{
            LpId,
            ops::get_lp_id,
        },
    },
    memory::{
        AddressSpaceId,
        VAddr,
        physical::PAddr,
    },
};

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

/// An interrupt source granted to a driver domain.
#[derive(Debug, Clone, Copy)]
struct InterruptObject {
    intid: u32,
    /// The completion queue readiness is delivered to, once bound.
    cq: Option<CqId>,
    /// The LP the source is routed to (set at bind time).
    target_lp: LpId,
    /// Interrupts delivered since the last [`interrupt_ack`] (coalescible).
    pending: u32,
    /// Lifetime interrupt count, for inspection.
    count: u64,
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

/// The routing entry consulted from interrupt context to steer a delivered
/// INTID to the owning driver's completion queue. Kept small and `Copy` so
/// the IRQ path can copy it out under a short `try_lock`.
#[derive(Debug, Clone, Copy)]
struct IrqRoute {
    asid: AddressSpaceId,
    cap: DeviceCap,
    cq: CqId,
}

static DEVICES: LazyLock<Mutex<BTreeMap<AddressSpaceId, AsDeviceCaps>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Global INTID → owning-driver route table. Separate from [`DEVICES`] so the
/// interrupt path can look up a route with a short, independent `try_lock`.
static ROUTES: LazyLock<Mutex<BTreeMap<u32, IrqRoute>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

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
        pending: 0,
        count: 0,
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
/// marks the object pending, and posts a coalesced wake to `cq` so the driver
/// shard — blocked in a single `CQ_WAIT` — becomes runnable (architecture doc
/// §10.2, unified shard wait of §7).
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
        irq.cq = Some(cq);
        irq.target_lp = target_lp;
        irq.pending = 0;
        intid = irq.intid;
    }
    ROUTES.lock().insert(intid, IrqRoute {
        asid,
        cap,
        cq,
    });
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
    let consumed = core::mem::take(&mut irq.pending);
    let intid = irq.intid;
    let target_lp = irq.target_lp;
    drop(devices);
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
    Ok((irq.pending, irq.count))
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
        DeviceObject::Interrupt(irq) => {
            arch_disable_irq(irq.intid);
            ROUTES.lock().remove(&irq.intid);
        }
    }
    let mut devices = DEVICES.lock();
    devices
        .get_mut(&asid)
        .and_then(|caps| caps.caps.remove(&cap))
        .map(|_| ())
        .ok_or(DeviceError::UnknownCapability)
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
            DeviceObject::Interrupt(irq) => {
                arch_disable_irq(irq.intid);
                ROUTES.lock().remove(&irq.intid);
            }
        }
    }
}

// ---- interrupt delivery (interrupt context) --------------------------------

/// Steer a delivered INTID to its owning driver domain. Called from the
/// architecture IRQ dispatcher for INTIDs not claimed by the kernel itself.
///
/// Runs in interrupt context, so it uses `try_lock` throughout and never
/// blocks: if a lock is momentarily held it simply masks the source and
/// returns; the driver observes progress on the next delivery. It masks the
/// source (so a level-triggered device does not storm the CPU until the
/// driver acknowledges), marks the interrupt object pending, and posts a
/// coalesced readiness wake to the driver's completion queue.
///
/// Returns `true` if the INTID was claimed by a bound driver interrupt object.
pub fn deliver_interrupt(intid: u32) -> bool {
    let route = match ROUTES.try_lock() {
        Some(routes) => match routes.get(&intid) {
            Some(route) => *route,
            None => return false,
        },
        None => return false,
    };

    // Mask and de-pend the source until the driver acknowledges.
    arch_disable_irq(intid);
    arch_clear_irq_pending(intid);

    // Mark the interrupt object pending (best-effort under try_lock).
    if let Some(mut devices) = DEVICES.try_lock() {
        if let Some(DeviceObject::Interrupt(irq)) =
            devices.get_mut(&route.asid).and_then(|caps| caps.caps.get_mut(&route.cap))
        {
            irq.pending = irq.pending.saturating_add(1);
            irq.count = irq.count.saturating_add(1);
        }
    }

    // Coalesced readiness notification to the driver's completion queue.
    crate::completion::wake(route.asid, route.cq);
    true
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
