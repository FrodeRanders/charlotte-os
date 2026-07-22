use alloc::sync::Arc;
use core::sync::atomic::{
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

#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct Waker(ThreadId, ThreadGeneration);

impl Waker {
    pub fn new(tid: ThreadId, generation: ThreadGeneration) -> Self {
        Self(tid, generation)
    }
}

impl crate::klib::observer::Observer for Waker {
    fn notify(self: Arc<Self>) {
        WAKER_DIAGNOSTICS[0].fetch_add(1, Ordering::Relaxed);
        crate::debug_trace::trace(crate::debug_trace::TAG_WAKER_NOTIFY, self.0 as u64, self.1, 0);
        sched_trace!("[sched] waker notify TID={} gen={}", self.0, self.1);
        // A notification may race thread exit and table-slot reuse. Treat a
        // generation mismatch as a stale wake, never as authority to wake the
        // new occupant of the same numeric id.
        if SYSTEM_SCHEDULER.read().submit_woken_thread(self.0, self.1).is_ok() {
            WAKER_DIAGNOSTICS[1].fetch_add(1, Ordering::Relaxed);
        } else {
            WAKER_DIAGNOSTICS[2].fetch_add(1, Ordering::Relaxed);
        }
    }
}
