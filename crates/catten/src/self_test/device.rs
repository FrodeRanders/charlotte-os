//! Self-test: device capabilities (MMIO regions and interrupt objects).
//!
//! Exercises the kernel half of the userspace-driver model (architecture doc
//! §10, Phase 8):
//!
//! - negative tests for the capability model: unknown caps, wrong-type operations, acknowledging an
//!   unbound interrupt, unmapping an unmapped region;
//! - mapping an MMIO region capability into a real address space as user-accessible device memory,
//!   then unmapping it;
//! - interrupt delivery to a completion queue: a thread blocked in a single `wait_on_cq` is
//!   released both by the deterministic kernel delivery path (`deliver_interrupt`, what the IRQ
//!   dispatcher calls) and by a **real** GIC software-pended SPI routed through the live interrupt
//!   path, and the interrupt object tracks pending/ack state across re-arming.
//!
//! The waiter and driver run as scheduled kernel threads, mirroring the
//! `cq_wait` self-test: every release condition is also observed by the
//! wait's fast path if it is posted before the waiter blocks, so the flow is
//! robust to scheduling order.

#[cfg(target_arch = "aarch64")]
use core::sync::atomic::{
    AtomicU32,
    AtomicU64,
    Ordering,
};

use crate::logln;

/// Pseudo address-space id for the kernel-API capability tests (only present
/// in the device and completion registries, never scheduled).
#[cfg(target_arch = "aarch64")]
const DEV_ASID: usize = 0x000d_e71c;

/// A spare Shared Peripheral Interrupt id on the QEMU `virt` machine, unused
/// by the platform devices we drive, so pending it in software is harmless.
#[cfg(target_arch = "aarch64")]
const TEST_SPI: u32 = 42;

#[cfg(target_arch = "aarch64")]
static IRQ_CAP: AtomicU64 = AtomicU64::new(0);
#[cfg(target_arch = "aarch64")]
static ROUND1_RELEASED: AtomicU32 = AtomicU32::new(0);
#[cfg(target_arch = "aarch64")]
static ROUND2_START: AtomicU32 = AtomicU32::new(0);
#[cfg(target_arch = "aarch64")]
static ROUND2_RELEASED: AtomicU32 = AtomicU32::new(0);
#[cfg(target_arch = "aarch64")]
static WAITER_PHASE: AtomicU32 = AtomicU32::new(0);
#[cfg(target_arch = "aarch64")]
static DRIVER_PHASE: AtomicU32 = AtomicU32::new(0);

pub fn test_device_capabilities() {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::{
            device::{
                self,
                DeviceError,
            },
            self_test::results::{
                self,
                TestId,
            },
        };

        logln!("Testing device capabilities (MMIO regions and interrupt objects)...");

        completion_open();

        // --- Capability-model negative tests -------------------------------
        let mmio =
            device::grant_mmio(DEV_ASID, 0x0900_0000, 1).expect("[device] grant_mmio failed");
        let mut irq =
            device::grant_interrupt(DEV_ASID, TEST_SPI).expect("[device] grant_interrupt failed");
        assert_eq!(
            device::grant_interrupt(DEV_ASID, 31),
            Err(DeviceError::InvalidInterrupt),
            "[device] private interrupts must not be delegated"
        );
        assert_eq!(
            device::grant_interrupt(0, TEST_SPI + 1),
            Err(DeviceError::InvalidAddressSpace),
            "[device] kernel ASID must not be packed as a driver route"
        );
        assert_eq!(
            device::grant_mmio(DEV_ASID, usize::MAX & !(4096 - 1), 2),
            Err(DeviceError::InvalidRange),
            "[device] overflowing MMIO grants must be rejected"
        );
        assert_eq!(
            device::grant_interrupt(DEV_ASID + 1, TEST_SPI),
            Err(DeviceError::InterruptInUse),
            "[device] an INTID must have a single capability owner"
        );

        assert_eq!(
            device::mmio_map(DEV_ASID, 0xdead_beef, crate::memory::VAddr::from(0x4000usize), true),
            Err(DeviceError::UnknownCapability),
            "[device] mapping an unknown capability must fail"
        );
        assert_eq!(
            device::mmio_map(DEV_ASID, irq, crate::memory::VAddr::from(0x4000usize), true),
            Err(DeviceError::WrongType),
            "[device] mapping an interrupt capability as MMIO must fail"
        );
        assert_eq!(
            device::interrupt_bind_cq(DEV_ASID, mmio, 0),
            Err(DeviceError::WrongType),
            "[device] binding an MMIO capability as an interrupt must fail"
        );
        assert_eq!(
            device::interrupt_ack(DEV_ASID, irq),
            Err(DeviceError::NotBound),
            "[device] acknowledging an unbound interrupt must fail"
        );
        assert_eq!(
            device::mmio_unmap(DEV_ASID, mmio),
            Err(DeviceError::NotMapped),
            "[device] unmapping an unmapped region must fail"
        );
        logln!("[device] capability-model negative tests passed");

        // --- MMIO map / unmap against a real address space -----------------
        test_mmio_map_unmap();
        test_failed_dma_map_releases_pin();
        test_stale_address_space_handle();
        crate::cpu::isa::memory::paging::self_test_hw_asid_allocator();
        irq = test_stale_interrupt_wake(irq);

        // Close the throwaway MMIO grant; the interrupt grant is consumed by
        // the delivery rounds below.
        device::close_cap(DEV_ASID, mmio).expect("[device] close_cap(mmio) failed");

        // --- Interrupt delivery to a completion queue ----------------------
        device::interrupt_bind_cq(DEV_ASID, irq, 0).expect("[device] interrupt_bind_cq failed");
        // TEST_SPI has no physical line behind it: round 2 asserts it solely
        // through GICD_ISPENDR. Model that synthetic source as an edge rather
        // than relying on a nonexistent device to hold a level asserted.
        crate::cpu::isa::interrupts::gic::configure_synthetic_spi_edge(TEST_SPI);
        assert_eq!(
            device::interrupt_bind_cq(DEV_ASID, irq, 0),
            Err(DeviceError::AlreadyBound),
            "[device] double-binding an interrupt must fail"
        );
        IRQ_CAP.store(irq, Ordering::Release);

        results::spawn_verifier(TestId::Device, irq_driver);
        logln!("[device] interrupt waiter and driver deferred");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping device capability test (AArch64 only).");
    }
}

/// A delayed teardown handle must not close a new domain that reused the
/// predecessor's numeric ASID.
#[cfg(target_arch = "aarch64")]
fn test_stale_address_space_handle() {
    use crate::{
        memory::{
            AddressSpaceCloseError,
            close_user_address_space_handle,
        },
        service::loader,
    };

    let old = loader::create_user_address_space_handle();
    let old_hw_asid = crate::memory::ADDRESS_SPACE_TABLE
        .lock()
        .get(old.id())
        .expect("[device] old address space missing")
        .hw_asid();
    assert_ne!(old_hw_asid, 0, "user address space must have a hardware ASID");
    close_user_address_space_handle(old).expect("[device] initial AS close failed");
    let replacement = loader::create_user_address_space_handle();
    let replacement_hw_asid = crate::memory::ADDRESS_SPACE_TABLE
        .lock()
        .get(replacement.id())
        .expect("[device] replacement address space missing")
        .hw_asid();
    assert_eq!(replacement.id(), old.id(), "address-space test expected slot reuse");
    assert_ne!(replacement.generation(), old.generation());
    assert_eq!(
        replacement_hw_asid, old_hw_asid,
        "address-space test expected invalidated hardware-ASID recycling"
    );
    assert_eq!(
        close_user_address_space_handle(old),
        Err(AddressSpaceCloseError::StaleHandle),
        "stale address-space handle closed its replacement"
    );
    assert!(crate::memory::address_space_handle_is_current(replacement));
    close_user_address_space_handle(replacement).expect("[device] replacement AS close failed");
    logln!("[device] stale address-space teardown rejected after ASID reuse");
}

/// A deferred IRQ wake queued for a retired route must not be delivered when
/// the same INTID, numeric ASID, and CQ are rebound. Only route generation can
/// distinguish these otherwise-identical tuples.
#[cfg(target_arch = "aarch64")]
fn test_stale_interrupt_wake(old: u64) -> u64 {
    crate::device::interrupt_bind_cq(DEV_ASID, old, 0)
        .expect("[device] stale-wake initial bind failed");
    assert!(crate::device::deliver_interrupt(TEST_SPI));
    crate::device::close_cap(DEV_ASID, old).expect("[device] stale-wake initial close failed");

    let replacement = crate::device::grant_interrupt(DEV_ASID, TEST_SPI)
        .expect("[device] stale-wake replacement interrupt grant failed");
    crate::device::interrupt_bind_cq(DEV_ASID, replacement, 0)
        .expect("[device] stale-wake replacement bind failed");
    assert_eq!(
        crate::device::drain_deferred_wakes(),
        0,
        "wake from retired interrupt route reached its replacement"
    );
    crate::device::close_cap(DEV_ASID, replacement)
        .expect("[device] stale-wake replacement close failed");
    logln!("[device] stale deferred interrupt wake rejected after route reuse");
    crate::device::grant_interrupt(DEV_ASID, TEST_SPI)
        .expect("[device] stale-wake final interrupt grant failed")
}

#[cfg(target_arch = "aarch64")]
fn completion_open() {
    // A completion-queue address space so interrupt readiness has somewhere
    // to be delivered (queue 0).
    crate::completion::open_address_space_with_cq(DEV_ASID, 8, 8);
}

/// A DMA map pins the object before acquiring the SMMU registry. Even when the
/// domain lookup fails, the pin must be rolled back so the owner can close and
/// reclaim the object.
#[cfg(target_arch = "aarch64")]
fn test_failed_dma_map_releases_pin() {
    use crate::{
        device::smmu,
        memory::object,
        self_test::close_test_address_space,
        service::loader,
    };

    let asid = loader::create_user_address_space();
    let memory = object::allocate(asid, 1).expect("[device] DMA rollback object allocation failed");
    let result = smmu::map(u64::MAX, asid, memory, smmu::Direction::DEVICE_READ, false);
    assert!(
        result == Err(smmu::Error::UnknownDomain) || result == Err(smmu::Error::Unsupported),
        "[device] invalid DMA domain must reject mapping (got {result:?})"
    );
    object::close_cap(asid, memory).expect("[device] failed DMA map leaked its memory pin");
    close_test_address_space(asid).expect("[device] DMA rollback address-space cleanup failed");
    logln!("[device] failed DMA map released its memory pin");
}

/// Map an MMIO region capability into a real (non-running) address space as
/// user device memory, then unmap and reclaim. Uses a spare physical frame as
/// the stand-in device register block; it is never accessed, only mapped.
#[cfg(target_arch = "aarch64")]
fn test_mmio_map_unmap() {
    use crate::{
        device,
        memory::{
            PHYSICAL_FRAME_ALLOCATOR,
            VAddr,
            physical::PAddr,
        },
        self_test::close_test_address_space,
        service::loader,
    };

    let asid = loader::create_user_address_space();
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("[device] failed to allocate stand-in device frame");
    let phys_base = <PAddr as Into<u64>>::into(frame) as usize;

    let cap = device::grant_mmio(asid, phys_base, 1).expect("[device] grant_mmio (real AS) failed");
    let base = VAddr::from(0x0000_0000_0004_0000usize);
    device::mmio_map(asid, cap, base, true).expect("[device] mmio_map into real AS failed");
    assert_eq!(
        device::mmio_map(asid, cap, base, true),
        Err(device::DeviceError::AlreadyMapped),
        "[device] double-mapping an MMIO region must fail"
    );
    device::mmio_unmap(asid, cap).expect("[device] mmio_unmap failed");
    device::close_cap(asid, cap).expect("[device] close_cap (real AS) failed");

    // Return the stand-in frame and tear down the throwaway address space.
    PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .deallocate_frame(frame)
        .expect("[device] failed to free stand-in device frame");
    close_test_address_space(asid).expect("[device] close address space failed");
    logln!("[device] MMIO map/unmap into a real address space passed");
}

#[cfg(target_arch = "aarch64")]
extern "C" fn irq_waiter() {
    use crate::{
        completion,
        device,
    };

    let irq = IRQ_CAP.load(Ordering::Acquire);
    device_phase(1, irq, 0);

    // Round 1: released by the deterministic kernel delivery path.
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    let (pending, count) = loop {
        let _ = completion::wait_on_cq_timeout(DEV_ASID, 0, 1, 100);
        let status =
            device::interrupt_status(DEV_ASID, irq).expect("[device] status after round 1 failed");
        if status.0 != 0 {
            break status;
        }
        // A CQ is deliberately a unified readiness source. A stale or
        // unrelated generation may release the wait, so the interrupt
        // object's pending counter remains the authoritative condition.
        deadline.assert_pending("device round 1 interrupt");
    };
    device_phase(2, irq, 0);
    assert!(count >= 1, "[device] round 1 lifetime count must advance");
    let consumed = device::interrupt_ack(DEV_ASID, irq).expect("[device] ack round 1 failed");
    assert_eq!(consumed, pending, "[device] ack must consume the pending count");
    let (pending_after, _) =
        device::interrupt_status(DEV_ASID, irq).expect("[device] status after ack failed");
    assert_eq!(pending_after, 0, "[device] ack must clear pending");
    ROUND1_RELEASED.store(1, Ordering::Release);
    device_phase(3, u64::from(pending), count);

    // Round 2: released by a real GIC software-pended SPI through the live
    // interrupt path (dispatcher → deliver_interrupt → CQ wake).
    spin_until(&ROUND2_START, "round 2 start");
    device_phase(4, irq, 0);
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    let pending = loop {
        let _ = completion::wait_on_cq_timeout(DEV_ASID, 0, 1, 100);
        let (pending, _) =
            device::interrupt_status(DEV_ASID, irq).expect("[device] status after round 2 failed");
        if pending != 0 {
            break pending;
        }
        deadline.assert_pending("device round 2 interrupt");
    };
    device_phase(5, irq, 0);
    let _ = device::interrupt_ack(DEV_ASID, irq).expect("[device] ack round 2 failed");
    ROUND2_RELEASED.store(1, Ordering::Release);
    device_phase(6, u64::from(pending), 0);
}

#[cfg(target_arch = "aarch64")]
extern "C" fn irq_driver() {
    use crate::{
        cpu::scheduler::spawn_thread,
        device,
        memory::KERNEL_ASID,
    };

    let irq = IRQ_CAP.load(Ordering::Acquire);
    device_phase(10, irq, 0);
    let _waiter = spawn_thread(KERNEL_ASID, irq_waiter);

    // Round 1: simulate exactly what the IRQ dispatcher does for this INTID.
    // Delivery is intentionally unordered with the waiter: CQ readiness and
    // the pending count are persistent, so the wait fast path must cover an
    // interrupt that arrives first.
    assert!(
        device::deliver_interrupt(TEST_SPI),
        "[device] deliver_interrupt must claim the bound INTID"
    );
    device_phase(11, irq, 0);
    spin_until(&ROUND1_RELEASED, "round 1 release");
    device_phase(12, irq, 0);

    // Round 2: pend the SPI in the real GIC and let the hardware path deliver
    // it. The prior ack re-armed the source.
    ROUND2_START.store(1, Ordering::Release);
    let _ = irq; // cap consumed by the waiter; keep symmetry with round 1
    crate::cpu::isa::interrupts::gic::set_spi_pending(TEST_SPI);
    device_phase(13, irq, TEST_SPI as u64);
    // QEMU/HVF can occasionally lose a distributor software-pend transition.
    // A real level-triggered device keeps its line asserted until ack, so
    // faithfully model that property here: while neither the capability's
    // pending counter nor the waiter reports delivery, reassert the source at
    // a modest interval. Never re-pend after delivery, since doing so while the
    // source is masked would create an artificial second interrupt on ack.
    let deadline = crate::self_test::results::Deadline::after_millis(5_000);
    let mut next_reassert = crate::cpu::scheduler::monotonic_millis().saturating_add(16);
    while ROUND2_RELEASED.load(Ordering::Acquire) == 0 {
        deadline.assert_pending("round 2 GIC delivery");
        crate::cpu::scheduler::yield_lp();
        let now = crate::cpu::scheduler::monotonic_millis();
        if now >= next_reassert {
            let (pending, _) = device::interrupt_status(DEV_ASID, irq)
                .expect("[device] status while awaiting round 2 failed");
            if pending == 0 {
                crate::cpu::isa::interrupts::gic::set_spi_pending(TEST_SPI);
            }
            next_reassert = now.saturating_add(16);
        }
    }
    device_phase(14, irq, 0);

    // Tear down the interrupt cap: mask and unroute the source.
    device::close_cap(DEV_ASID, IRQ_CAP.load(Ordering::Acquire))
        .expect("[device] close_cap(irq) failed");
    let recycled = device::grant_interrupt(DEV_ASID + 1, TEST_SPI)
        .expect("[device] closed INTID must become grantable");
    device::close_cap(DEV_ASID + 1, recycled).expect("[device] recycled interrupt close failed");

    logln!(
        "[device] SUCCESS: MMIO map/unmap, capability-model rejections, and interrupt delivery to \
         a completion queue via both the kernel path and a real GIC SPI all verified."
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::Device);
}

#[cfg(target_arch = "aarch64")]
fn device_phase(phase: u64, a: u64, b: u64) {
    if phase < 10 {
        WAITER_PHASE.store(phase as u32, Ordering::Release);
    } else {
        DRIVER_PHASE.store(phase as u32, Ordering::Release);
    }
    crate::debug_trace::trace(crate::debug_trace::TAG_DEVICE_PHASE, phase, a, b);
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn progress() -> (u32, u32) {
    (WAITER_PHASE.load(Ordering::Acquire), DRIVER_PHASE.load(Ordering::Acquire))
}

#[cfg(target_arch = "aarch64")]
fn spin_until(flag: &AtomicU32, what: &str) {
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    while flag.load(Ordering::Acquire) == 0 {
        deadline.assert_pending(what);
        // This is an atomic thread-to-thread handoff. Depending on a timer to
        // poll it makes the interrupt test fail when an otherwise unrelated
        // timer PPI is delayed on the coordinator's LP.
        crate::cpu::scheduler::yield_lp();
    }
}

/// x86_64 IOAPIC routing smoke test.
///
/// Routes a Global System Interrupt through the device-capability path
/// (grant → bind, which programs the IOAPIC redirection table), then injects a
/// synthetic delivery on the routed vector via a self-IPI and verifies the
/// architecture-independent delivery path claims it.
#[cfg(target_arch = "x86_64")]
pub fn test_ioapic_routing() {
    use crate::{
        cpu::isa::{
            interface::interrupts::LocalIntCtlrIfce,
            interrupts::LocalIntCtlr,
            lp::ops::get_lp_id,
        },
        device,
    };

    // A pseudo address-space id for the kernel-API capability test; never
    // scheduled, only present in the device registry.
    const TEST_ASID: usize = 0x000d_e71c;
    // An unused wired GSI on QEMU q35 (the RTC's interrupt line).
    const TEST_GSI: u32 = 8;

    logln!("[device] testing x86_64 IOAPIC routing (GSI {TEST_GSI})...");

    let cap =
        device::grant_interrupt(TEST_ASID, TEST_GSI).expect("[device] grant_interrupt failed");
    device::interrupt_bind_cq(TEST_ASID, cap, 0).expect("[device] interrupt_bind_cq failed");

    let vector = crate::cpu::isa::interrupts::device_irq::gsi_vector(TEST_GSI)
        .expect("[device] GSI was not routed to a vector");
    logln!("[device] GSI {TEST_GSI} routed to vector {vector}");

    // Inject a synthetic delivery on the routed vector: a self-IPI exercises
    // the dynamic ISR → handler → deliver_interrupt path without a real device.
    LocalIntCtlr::send_unicast_ipi(get_lp_id(), vector).expect("[device] self-IPI failed");

    let deadline = crate::self_test::results::Deadline::after_millis(5_000);
    loop {
        let (_pending, count) = device::interrupt_status(TEST_ASID, cap).unwrap();
        if count >= 1 {
            break;
        }
        deadline.assert_pending("IOAPIC-routed interrupt delivery");
        crate::cpu::scheduler::yield_lp();
    }
    let (pending, count) = device::interrupt_status(TEST_ASID, cap).unwrap();
    assert_eq!(count, 1, "one IOAPIC-routed interrupt must be counted");
    assert_eq!(pending, 1, "the delivered interrupt must remain pending until acknowledged");
    device::close_cap(TEST_ASID, cap).expect("[device] close_cap failed");
    logln!("[device] x86_64 IOAPIC routing delivered a routed GSI to the completion path.");
}

/// x86_64 MSI allocation smoke test.
///
/// Allocates an MSI (delivered directly to the LAPIC via the message address),
/// verifies the message is well-formed, routes it through the device-capability
/// path, and injects a synthetic delivery on the returned vector.
#[cfg(target_arch = "x86_64")]
pub fn test_msi_routing() {
    use crate::{
        cpu::isa::{
            interface::interrupts::LocalIntCtlrIfce,
            interrupts::LocalIntCtlr,
            lp::ops::get_lp_id,
        },
        device,
    };

    const TEST_ASID: usize = 0x000d_e71c;

    logln!("[device] testing x86_64 MSI allocation...");
    let message = device::allocate_msi(0).expect("[device] allocate_msi failed");
    assert_eq!(
        message.address & 0xffff_fff0,
        0xfee0_0000,
        "MSI address must target the LAPIC MSI window"
    );
    let vector = message.data as u8;
    assert!((35..=254).contains(&vector), "MSI data must encode a dynamic vector, got {vector}");
    assert!(message.intid >= 256, "MSI intid must live in the synthetic MSI range");
    logln!(
        "[device] MSI address={:#x} data={} intid={}",
        message.address,
        message.data,
        message.intid
    );

    let cap =
        device::grant_interrupt(TEST_ASID, message.intid).expect("[device] grant_interrupt failed");
    device::interrupt_bind_cq(TEST_ASID, cap, 0).expect("[device] interrupt_bind_cq failed");

    // Inject a synthetic delivery on the returned vector (a self-IPI stands in
    // for the device writing the MSI message to the LAPIC).
    LocalIntCtlr::send_unicast_ipi(get_lp_id(), vector).expect("[device] MSI self-IPI failed");

    let deadline = crate::self_test::results::Deadline::after_millis(5_000);
    loop {
        let (_pending, count) = device::interrupt_status(TEST_ASID, cap).unwrap();
        if count >= 1 {
            break;
        }
        deadline.assert_pending("MSI interrupt delivery");
        crate::cpu::scheduler::yield_lp();
    }
    device::close_cap(TEST_ASID, cap).expect("[device] close_cap failed");
    logln!("[device] x86_64 MSI allocation delivered a routed interrupt to the completion path.");
}
