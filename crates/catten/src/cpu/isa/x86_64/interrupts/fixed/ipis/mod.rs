use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

core::arch::global_asm!(include_str!("ipis.asm"));

unsafe extern "custom" {
    pub fn isr_asynchronous_ipi();
    pub fn isr_synchronous_ipi();
    pub fn isr_scheduler_ipi();
}

use alloc::vec::Vec;

use charlotte_lifecycle::shootdown::{
    Acknowledgement,
    BeginError,
    Shootdown,
};

static SYNC_COORDINATOR: Shootdown = Shootdown::new();
static SYNC_ACKS: spin::Once<Vec<Acknowledgement>> = spin::Once::new();
static SYNC_SHOOTDOWN_READY: AtomicBool = AtomicBool::new(false);
const SHOOTDOWN_TIMEOUT_MS: u64 = 100;
const ADMISSION_TIMEOUT_MS: u64 = 500;

#[derive(Debug)]
pub enum ShootdownError {
    Busy,
    InterruptsMasked,
    Exhausted,
    Delivery(u32),
    TimedOut,
}

pub fn enable_sync_shootdowns() {
    SYNC_ACKS.call_once(|| {
        let count = crate::cpu::multiprocessor::get_lp_count() as usize;
        let mut acknowledgements = Vec::new();
        acknowledgements
            .try_reserve_exact(count)
            .expect("mandatory shootdown metadata allocation failed");
        acknowledgements.resize_with(count, Acknowledgement::new);
        acknowledgements
    });
    SYNC_SHOOTDOWN_READY.store(true, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_asynchronous_ipi() {
    // Drain pending IPI RPCs queued for this LP. The architecture-independent
    // handler dispatches TLB maintenance, scheduler wakeups, and typed
    // Closures (ShardMailbox).
    crate::cpu::multiprocessor::ipi::drain_local_ipi_queue();
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_synchronous_ipi() {
    // Capture before flush. A delayed old handler must never acknowledge a
    // newer request merely because that request appeared while it was flushing.
    let epoch = SYNC_COORDINATOR.requested();
    crate::cpu::isa::x86_64::memory::tlb::flush_all_local();
    if let Some(acks) = SYNC_ACKS.get() {
        acks[crate::cpu::isa::lp::ops::get_lp_id() as usize].complete(epoch);
    }
}

/// A bounded epoch-fenced rendezvous. No IPC/lifecycle/table guard may cross it.
/// Failure returns without proof of quiescence; retain the owning backing
/// receipt. A retry gets a fresh epoch and cannot count stale/duplicate IPIs.
pub fn try_send_sync_shootdown() -> Result<(), ShootdownError> {
    rendezvous(send_hardware)
}

fn send_hardware(lp: u32) -> bool {
    use crate::cpu::isa::{
        constants::interrupt_vectors::SYNC_IPI_VECTOR,
        interface::interrupts::LocalIntCtlrIfce,
        interrupts::LocalIntCtlr,
    };
    LocalIntCtlr::send_unicast_ipi(lp, SYNC_IPI_VECTOR).is_ok()
}

fn rendezvous(mut send: impl FnMut(u32) -> bool) -> Result<(), ShootdownError> {
    use crate::cpu::isa::lp::ops::{
        get_int_state,
        get_lp_id,
        mask_interrupts,
        unmask_interrupts,
    };
    if !SYNC_SHOOTDOWN_READY.load(Ordering::Acquire)
        || crate::cpu::multiprocessor::get_lp_count() <= 1
    {
        crate::cpu::isa::x86_64::memory::tlb::flush_all_local();
        return Ok(());
    }
    // Enabling IRQs beneath an unknown masking guard violates its contract.
    // Reject that context; callers with split-phase receipts can retain them.
    if !get_int_state() {
        return Err(ShootdownError::InterruptsMasked);
    }
    let admission_deadline =
        crate::cpu::scheduler::monotonic_millis().saturating_add(ADMISSION_TIMEOUT_MS);
    let attempt = loop {
        match SYNC_COORDINATOR.try_begin() {
            Ok(owner) => break owner,
            Err(BeginError::Exhausted) => return Err(ShootdownError::Exhausted),
            Err(BeginError::Busy) => {
                if crate::cpu::scheduler::monotonic_millis() >= admission_deadline {
                    return Err(ShootdownError::Busy);
                }
                // No coordinator/LP-local owner is held yet. Let a preempted
                // initiator run rather than spinning on its own worker LP.
                crate::cpu::scheduler::yield_lp();
            }
        }
    };
    let deadline = crate::cpu::scheduler::monotonic_millis().saturating_add(SHOOTDOWN_TIMEOUT_MS);
    let acks = SYNC_ACKS.get().expect("shootdown enabled without acknowledgements");
    // Only this local flush/identity capture is nonpreemptible. The initiating
    // thread may migrate after it; never acknowledge its old CPU on a new CPU.
    mask_interrupts!();
    let self_id = get_lp_id();
    crate::cpu::isa::x86_64::memory::tlb::flush_all_local();
    acks[self_id as usize].complete(attempt.epoch());
    unmask_interrupts!();
    if let Err(lp) = send_to_remote_lps(acks.len() as u32, self_id, &mut send) {
        return Err(ShootdownError::Delivery(lp));
    }
    while !attempt.completed(acks) {
        if crate::cpu::scheduler::monotonic_millis() >= deadline {
            return Err(ShootdownError::TimedOut);
        }
        core::hint::spin_loop();
    }
    Ok(())
}

/// Mandatory legacy callers cannot release backing on rejection. Owned cleanup
/// uses the fallible API; panic here stops this operation before unsafe reuse.
pub fn send_sync_shootdown() {
    try_send_sync_shootdown().expect("unconfirmed TLB invalidation; backing reuse forbidden");
}

fn send_to_remote_lps(
    lp_count: u32,
    self_id: u32,
    mut send: impl FnMut(u32) -> bool,
) -> Result<(), u32> {
    for lp in 0..lp_count {
        if lp != self_id && !send(lp) {
            return Err(lp);
        }
    }
    Ok(())
}

/// Single-mutator boot fixture, using an independent coordinator and fake send.
/// No hardware state/barrier is reset to simulate success.
pub(crate) fn test_failed_delivery() {
    assert!(!SYNC_SHOOTDOWN_READY.load(Ordering::Acquire));
    let coordinator = Shootdown::new();
    let acks = core::array::from_fn::<_, 4, _>(|_| Acknowledgement::new());
    let attempt = coordinator.try_begin().unwrap();
    let old = attempt.epoch();
    let mut seen = 0;
    assert_eq!(
        send_to_remote_lps(4, 0, |lp| {
            seen += 1;
            lp != 2
        }),
        Err(2)
    );
    assert_eq!(seen, 2);
    acks[0].complete(old);
    acks[1].complete(old);
    assert!(!attempt.completed(&acks));
    drop(attempt);
    let retry = coordinator.try_begin().unwrap();
    acks[2].complete(old);
    acks[3].complete(old);
    assert!(!retry.completed(&acks));
    for ack in &acks {
        ack.complete(retry.epoch());
    }
    assert!(retry.completed(&acks));
    assert_eq!(send_to_remote_lps(4, 2, |lp| lp != 2), Ok(()));
    crate::logln!(
        "[ipi] failed delivery and stale/duplicate acknowledgements reject; fresh epoch recovery \
         passed"
    );
}

/// Deferred guest adapter omits one actual remote delivery. This never fakes a
/// physical acknowledgement; the caller keeps its retired root on rejection.
pub(crate) fn test_incomplete_rendezvous(timeout: bool) -> Result<(), ShootdownError> {
    assert!(SYNC_SHOOTDOWN_READY.load(Ordering::Acquire));
    let mut omitted = false;
    rendezvous(|lp| {
        if !omitted {
            omitted = true;
            return timeout;
        }
        send_hardware(lp)
    })
}
