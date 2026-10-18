use core::sync::atomic::{
    AtomicBool,
    AtomicU32,
    Ordering,
};

use lock_api::RawMutex;

use crate::cpu::isa::lp::ops::{
    get_int_state,
    get_lp_id,
    mask_interrupts,
    unmask_interrupts,
};

pub type Mutex<T> = lock_api::Mutex<MutexCore, T>;

/// Spinlock that masks interrupts on the owning logical processor.
///
/// Masking prevents interrupt handlers from reentering a lock held on the same
/// LP. Callers must still follow lock ordering to avoid deadlocks between LPs.
///
/// It guards the frame allocator, the address-space table, the kernel address
/// space, domain-authority and lifecycle state, the scratch-window allocator,
/// and the global (talc) allocator. The interrupt masking is essential: these
/// locks are taken from both preemptible kernel threads and synchronous EL0
/// exception paths, so a timer-preempted owner could otherwise be starved by
/// every other LP spinning for the lock with IRQs masked.
#[derive(Debug)]
pub struct MutexCore {
    state: AtomicBool,
    saved_interrupt_flag: AtomicBool,
    /// Observation only: owning guards mask IRQs and cannot migrate. This
    /// identifies a caller's enclosing hold without mistaking remote-LP
    /// contention for a recursive lock. It grants no access to guarded data.
    owner_lp: AtomicU32,
}

impl MutexCore {
    pub const fn new() -> Self {
        Self {
            state: AtomicBool::new(false),
            saved_interrupt_flag: AtomicBool::new(false),
            owner_lp: AtomicU32::new(u32::MAX),
        }
    }

    pub(crate) fn owned_by_current_lp(&self) -> bool {
        let _pin = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        self.owner_lp.load(Ordering::Acquire) == get_lp_id()
    }
}

pub(crate) fn owned_by_current_lp<T>(mutex: &Mutex<T>) -> bool {
    // Only read the raw lock's atomic observation; do not change its state or
    // access payload. lock_api keeps the raw reference valid for this borrow.
    unsafe { mutex.raw() }.owned_by_current_lp()
}

impl Default for MutexCore {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl RawMutex for MutexCore {
    type GuardMarker = lock_api::GuardNoSend;

    const INIT: Self = Self::new();

    fn lock(&self) {
        loop {
            let int_state = get_int_state();
            mask_interrupts!();
            if self
                .state
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                self.saved_interrupt_flag.store(int_state, Ordering::Release);
                self.owner_lp.store(get_lp_id(), Ordering::Release);
                return;
            }

            // A lock owner may be waiting for this LP to acknowledge a TLB
            // shootdown. Keep maskable interrupts live while contending.
            if int_state {
                unmask_interrupts!();
            }
            while self.state.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }

    unsafe fn unlock(&self) {
        let restore = self.saved_interrupt_flag.swap(false, Ordering::Relaxed);
        self.owner_lp.store(u32::MAX, Ordering::Release);
        self.state.store(false, Ordering::Release);
        if restore {
            unmask_interrupts!();
        }
    }

    fn try_lock(&self) -> bool {
        let int_state = get_int_state();
        mask_interrupts!();
        let acquired =
            self.state.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok();
        if acquired {
            self.saved_interrupt_flag.store(int_state, core::sync::atomic::Ordering::Release);
            self.owner_lp.store(get_lp_id(), Ordering::Release);
        } else if int_state {
            unmask_interrupts!();
        }
        acquired
    }
}
