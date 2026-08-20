use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use lock_api::RawMutex;

use crate::cpu::isa::lp::ops::{
    get_int_state,
    mask_interrupts,
    unmask_interrupts,
};

pub type Mutex<T> = lock_api::Mutex<MutexCore, T>;

/// # A spinlock-based mutex that disables interrupts on the calling processor while locked.
/// This lock is suitable for providing mutual exclusion during critical sections but it should be
/// used with caution to avoid deadlocks between LPs. It prevents self deadlocks by
/// masking maskable interrupts for the complete ownership interval.
///
/// It guards the frame allocator, the address-space table, the kernel address
/// space, domain-authority and lifecycle state, the scratch-window cursor, and
/// the global (talc) allocator. The interrupt masking is essential: these
/// locks are taken from both preemptible kernel threads and synchronous EL0
/// exception paths, so a timer-preempted owner could otherwise be starved by
/// every other LP spinning for the lock with IRQs masked.
#[derive(Debug)]
pub struct MutexCore {
    state: AtomicBool,
    saved_interrupt_flag: AtomicBool,
}

impl MutexCore {
    pub const fn new() -> Self {
        Self {
            state: AtomicBool::new(false),
            saved_interrupt_flag: AtomicBool::new(false),
        }
    }
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
        } else if int_state {
            unmask_interrupts!();
        }
        acquired
    }
}
