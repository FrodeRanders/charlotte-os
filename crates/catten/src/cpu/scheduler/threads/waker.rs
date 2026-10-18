//! Thread waker — the observer that re-admits a blocked thread.
//!
//! A [`Waker`] identifies a thread by `(ThreadId, ThreadGeneration)` and holds
//! its wait registration and early-notification state. It is registered on an
//! observable event (timer, completion, endpoint, CQ).
//! When the event fires, [`Waker::notify`] calls `submit_woken_wait`,
//! which transitions the thread from `Blocked` to `Ready` and re-admits
//! it to its affinity LP. Re-admission validates both the thread generation
//! and this installed waker's identity, rejecting slot reuse and hints from
//! an earlier park. Early notifications are retained before publication.

use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::{
            ThreadGeneration,
            ThreadId,
        },
    },
    logln,
};

const SCHED_TRACE: bool = false;

/// External-debugger counters: notification attempts, successful admissions,
/// and rejected/stale admissions.
#[unsafe(no_mangle)]
pub static WAKER_DIAGNOSTICS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

macro_rules! sched_trace {
    ($($arg:tt)*) => {
        if SCHED_TRACE {
            logln!($($arg)*);
        }
    };
}

#[derive(Debug)]
pub struct Waker(
    ThreadId,
    ThreadGeneration,
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<crate::klib::observer::WaitRegistration>>,
    AtomicBool,
);

impl Waker {
    pub fn new(tid: ThreadId, generation: ThreadGeneration) -> Self {
        Self(
            tid,
            generation,
            crate::cpu::multiprocessor::spin::mutex::Mutex::new(None),
            AtomicBool::new(false),
        )
    }

    pub(crate) fn was_notified(&self) -> bool {
        self.3.load(Ordering::Acquire)
    }

    pub(crate) fn set_registration(&self, registration: crate::klib::observer::WaitRegistration) {
        *self.2.lock() = Some(registration);
    }

    pub(crate) fn cancel_registration(&self) {
        let registration = self.2.lock().take();
        drop(registration);
    }
}

impl crate::klib::observer::Observer for Waker {
    fn notify(self: Arc<Self>) {
        // Registration precedes Blocked publication. Remember an early hint,
        // even when the thread does not yet contain this particular waker.
        // Publication checks this flag under the same table serialization as
        // matching wake admission, closing both sides of that race.
        self.3.store(true, Ordering::Release);
        WAKER_DIAGNOSTICS[0].fetch_add(1, Ordering::Relaxed);
        crate::debug_trace::trace(crate::debug_trace::TAG_WAKER_NOTIFY, self.0 as u64, self.1, 0);
        sched_trace!("[sched] waker notify TID={} gen={}", self.0, self.1);
        // A notification may race thread exit and table-slot reuse. Treat a
        // generation mismatch as a stale wake, never as authority to wake the
        // new occupant of the same numeric id.
        if SYSTEM_SCHEDULER.read().submit_woken_wait(self.0, self.1, &self).is_ok() {
            WAKER_DIAGNOSTICS[1].fetch_add(1, Ordering::Relaxed);
        } else {
            WAKER_DIAGNOSTICS[2].fetch_add(1, Ordering::Relaxed);
        }
    }
}
