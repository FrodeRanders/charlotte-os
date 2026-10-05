use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};

core::arch::global_asm!(include_str!("ipis.asm"));

unsafe extern "custom" {
    pub fn isr_asynchronous_ipi();
    pub fn isr_synchronous_ipi();
    pub fn isr_scheduler_ipi();
}

/// Rendezvous barrier for the synchronous IPI. The initiating LP stores the
/// number of target LPs; each target's sync-IPI handler decrements it after
/// flushing its TLB, and the initiator spins until it reaches zero.
///
/// A single barrier suffices because a synchronous shootdown is a system-wide
/// rendezvous that must never be initiated from interrupt context or while a
/// target LP could be spinning on an interrupt-masking lock; concurrent
/// shootdowns are therefore precluded by construction.
#[unsafe(no_mangle)]
pub static SYNC_IPI_BARRIER: AtomicU64 = AtomicU64::new(0);
static SYNC_IPI_OWNER: AtomicBool = AtomicBool::new(false);
static SYNC_SHOOTDOWN_READY: AtomicBool = AtomicBool::new(false);

pub fn enable_sync_shootdowns() {
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
    // A synchronous IPI is a TLB-shootdown request. Flush this LP's non-global
    // translations (a CR3 reload — PCID is not enabled) and acknowledge the
    // rendezvous. The handler runs with interrupts masked by the interrupt
    // gate, so this completes before any other work resumes on this LP.
    crate::cpu::isa::x86_64::memory::tlb::flush_current_non_global();
    SYNC_IPI_BARRIER.fetch_sub(1, Ordering::SeqCst);
}

/// Initiate a synchronous cross-LP TLB shootdown: flush the calling LP locally,
/// interrupt every other LP (which flushes and decrements [`SYNC_IPI_BARRIER`]),
/// and spin until every target has acknowledged.
///
/// # Safety / liveness
///
/// This must only be called from a context that holds no interrupt-masking lock
/// and with local interrupts enabled: a target LP spinning on such a lock (or a
/// secondary LP still masked before scheduler admission) could not field the
/// IPI, and the rendezvous would deadlock.
pub fn send_sync_shootdown() {
    use crate::cpu::isa::{
        constants::interrupt_vectors::SYNC_IPI_VECTOR,
        interface::interrupts::LocalIntCtlrIfce,
        interrupts::LocalIntCtlr,
        lp::ops::{
            get_int_state,
            get_lp_id,
            mask_interrupts,
            unmask_interrupts,
        },
    };

    if !SYNC_SHOOTDOWN_READY.load(Ordering::Acquire) {
        crate::cpu::isa::x86_64::memory::tlb::flush_current_non_global();
        return;
    }
    let lp_count = crate::cpu::multiprocessor::get_lp_count();
    if lp_count <= 1 {
        crate::cpu::isa::x86_64::memory::tlb::flush_current_non_global();
        return;
    }
    let interrupts_were_enabled = get_int_state();
    if !interrupts_were_enabled {
        // SYSCALL enters with IF cleared. No locks may be held here, but this
        // LP must remain able to acknowledge the current owner while waiting
        // to serialize concurrent shootdowns.
        unmask_interrupts!();
    }
    while SYNC_IPI_OWNER
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        // Keep interrupts enabled while waiting so this LP can acknowledge
        // the current owner's rendezvous.
        core::hint::spin_loop();
    }
    mask_interrupts!();
    let self_id = get_lp_id();
    SYNC_IPI_BARRIER.store((lp_count - 1) as u64, Ordering::SeqCst);
    if let Err(lp) = send_to_remote_lps(lp_count, self_id, |lp| {
        LocalIntCtlr::send_unicast_ipi(lp, SYNC_IPI_VECTOR).is_ok()
    }) {
        // Never turn failed delivery into an acknowledgement. Keep ownership
        // and the barrier latched: no caller may return and reuse backing.
        // This is fail-stop, not recovery; a later acknowledged/epoch-fenced
        // retry protocol must precede any recoverable shootdown API.
        crate::early_logln!(
            "FATAL: TLB-shootdown delivery failed to LP{}; backing reuse blocked",
            lp
        );
        loop {
            unsafe {
                core::arch::asm!("cli", "hlt", options(nomem, nostack));
            }
        }
    }
    crate::cpu::isa::x86_64::memory::tlb::flush_current_non_global();
    while SYNC_IPI_BARRIER.load(Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }
    SYNC_IPI_OWNER.store(false, Ordering::Release);
    if interrupts_were_enabled {
        unmask_interrupts!();
    }
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

/// Single-LP boot fixture: fake delivery never invokes hardware handlers.
pub(crate) fn test_failed_delivery() {
    assert!(!SYNC_SHOOTDOWN_READY.load(Ordering::Acquire));
    SYNC_IPI_BARRIER.store(3, Ordering::SeqCst);
    let mut seen = 0;
    assert_eq!(
        send_to_remote_lps(4, 0, |lp| {
            seen += 1;
            lp != 2
        }),
        Err(2)
    );
    assert_eq!(seen, 2);
    assert_eq!(SYNC_IPI_BARRIER.load(Ordering::SeqCst), 3);
    SYNC_IPI_BARRIER.store(0, Ordering::SeqCst);
    assert_eq!(send_to_remote_lps(4, 2, |lp| lp != 2), Ok(()));
    crate::logln!("[ipi] delivery failure does not acknowledge invalidation; self excluded");
}
