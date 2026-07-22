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
const MAX_SPINS: u64 = 80_000_000;

#[cfg(target_arch = "aarch64")]
static IRQ_CAP: AtomicU64 = AtomicU64::new(0);
#[cfg(target_arch = "aarch64")]
static ROUND1_RELEASED: AtomicU32 = AtomicU32::new(0);
#[cfg(target_arch = "aarch64")]
static ROUND2_START: AtomicU32 = AtomicU32::new(0);
#[cfg(target_arch = "aarch64")]
static ROUND2_RELEASED: AtomicU32 = AtomicU32::new(0);

pub fn test_device_capabilities() {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::{
            cpu::scheduler::spawn_thread,
            device::{
                self,
                DeviceError,
            },
            memory::KERNEL_ASID,
        };

        logln!("Testing device capabilities (MMIO regions and interrupt objects)...");

        completion_open();

        // --- Capability-model negative tests -------------------------------
        let mmio =
            device::grant_mmio(DEV_ASID, 0x0900_0000, 1).expect("[device] grant_mmio failed");
        let irq =
            device::grant_interrupt(DEV_ASID, TEST_SPI).expect("[device] grant_interrupt failed");

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

        // Close the throwaway MMIO grant; the interrupt grant is consumed by
        // the delivery rounds below.
        device::close_cap(DEV_ASID, mmio).expect("[device] close_cap(mmio) failed");

        // --- Interrupt delivery to a completion queue ----------------------
        device::interrupt_bind_cq(DEV_ASID, irq, 0).expect("[device] interrupt_bind_cq failed");
        assert_eq!(
            device::interrupt_bind_cq(DEV_ASID, irq, 0),
            Err(DeviceError::AlreadyBound),
            "[device] double-binding an interrupt must fail"
        );
        IRQ_CAP.store(irq, Ordering::Release);

        let _waiter = spawn_thread(KERNEL_ASID, irq_waiter);
        let _driver = spawn_thread(KERNEL_ASID, irq_driver);
        logln!("[device] interrupt waiter and driver deferred");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping device capability test (AArch64 only).");
    }
}

#[cfg(target_arch = "aarch64")]
fn completion_open() {
    // A completion-queue address space so interrupt readiness has somewhere
    // to be delivered (queue 0).
    crate::completion::open_address_space_with_cq(DEV_ASID, 8, 8);
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
            close_user_address_space,
            physical::PAddr,
        },
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
    close_user_address_space(asid).expect("[device] close_user_address_space failed");
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
    completion::wait_on_cq(DEV_ASID, 0, 1);
    device_phase(2, irq, 0);
    let (pending, count) =
        device::interrupt_status(DEV_ASID, irq).expect("[device] status after round 1 failed");
    assert!(pending >= 1, "[device] round 1 must observe a pending interrupt");
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
    completion::wait_on_cq(DEV_ASID, 0, 1);
    device_phase(5, irq, 0);
    let (pending, _) =
        device::interrupt_status(DEV_ASID, irq).expect("[device] status after round 2 failed");
    assert!(pending >= 1, "[device] round 2 must observe a pending interrupt");
    let _ = device::interrupt_ack(DEV_ASID, irq).expect("[device] ack round 2 failed");
    ROUND2_RELEASED.store(1, Ordering::Release);
    device_phase(6, u64::from(pending), 0);
}

#[cfg(target_arch = "aarch64")]
extern "C" fn irq_driver() {
    use crate::{
        device,
    };

    let irq = IRQ_CAP.load(Ordering::Acquire);
    device_phase(10, irq, 0);

    // Give the waiter a chance to block first; the fast path covers the case
    // where it has not.
    for _ in 0..64 {
        crate::cpu::scheduler::sleep_millis(1);
    }

    // Round 1: simulate exactly what the IRQ dispatcher does for this INTID.
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
    for _ in 0..64 {
        crate::cpu::scheduler::sleep_millis(1);
    }
    let _ = irq; // cap consumed by the waiter; keep symmetry with round 1
    crate::cpu::isa::interrupts::gic::set_spi_pending(TEST_SPI);
    device_phase(13, irq, TEST_SPI as u64);
    // QEMU/HVF can occasionally lose a distributor software-pend transition.
    // A real level-triggered device keeps its line asserted until ack, so
    // faithfully model that property here: while neither the capability's
    // pending counter nor the waiter reports delivery, reassert the source at
    // a modest interval. Never re-pend after delivery, since doing so while the
    // source is masked would create an artificial second interrupt on ack.
    let mut waits = 0u32;
    while ROUND2_RELEASED.load(Ordering::Acquire) == 0 {
        waits += 1;
        assert!(waits < 2_000, "[device] FAILED waiting for round 2 GIC delivery");
        crate::cpu::scheduler::sleep_millis(1);
        if waits.is_multiple_of(16) {
            let (pending, _) = device::interrupt_status(DEV_ASID, irq)
                .expect("[device] status while awaiting round 2 failed");
            if pending == 0 {
                crate::cpu::isa::interrupts::gic::set_spi_pending(TEST_SPI);
            }
        }
    }
    device_phase(14, irq, 0);

    // Tear down the interrupt cap: mask and unroute the source.
    device::close_cap(DEV_ASID, IRQ_CAP.load(Ordering::Acquire))
        .expect("[device] close_cap(irq) failed");

    logln!(
        "[device] SUCCESS: MMIO map/unmap, capability-model rejections, and interrupt delivery to \
         a completion queue via both the kernel path and a real GIC SPI all verified."
    );
}

#[cfg(target_arch = "aarch64")]
fn device_phase(phase: u64, a: u64, b: u64) {
    crate::debug_trace::trace(crate::debug_trace::TAG_DEVICE_PHASE, phase, a, b);
}

#[cfg(target_arch = "aarch64")]
fn spin_until(flag: &AtomicU32, what: &str) {
    let mut spins: u64 = 0;
    while flag.load(Ordering::Acquire) == 0 {
        spins += 1;
        assert!(spins < MAX_SPINS, "[device] FAILED waiting for {}", what);
        crate::cpu::scheduler::sleep_millis(1);
    }
}
