pub mod round_robin;
use alloc::{
    fmt::Debug,
    sync::Arc,
};
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::{
    cpu::{
        isa::{
            lp::LpId,
            memory::paging::HwAsid,
        },
        scheduler::threads::{
            ThreadCount,
            ThreadGeneration,
            ThreadId,
        },
    },
    klib::observer::Observer,
    memory::AddressSpaceId,
};

pub trait LpScheduler: Debug + Send {
    fn get_lp_id(&self) -> LpId;
    fn get_tid(&self) -> Option<ThreadId>;
    fn is_ctx_switch_pending(&self) -> bool;
    /* The following two functions should use interior mutability to access an internal atomic
     * for safe lock free operaton. */
    fn set_ctx_switch_pending(&self);
    /* This should clear the pending context switch field and when appropriate create and submit
     * a new TimerEvent to the local TimerQueue so the pending context switch will get set to
     * true again when the event notifies */
    fn clear_ctx_switch_pending(&self);
    fn next(&mut self) -> Result<ThreadId, Error>;
    fn add_thread(
        &mut self,
        tid: ThreadId,
        expected_generation: Option<ThreadGeneration>,
    ) -> Result<(), Error>;
    fn remove_thread(&mut self, tid: ThreadId) -> Result<(), Error>;
    fn is_idle(&self) -> bool;
    fn start(&mut self);
    fn stop(&mut self);
    fn asid_to_hwasid(&self, asid: AddressSpaceId) -> Option<HwAsid>;
    fn thread_count(&self) -> ThreadCount;
}

#[derive(Debug)]
pub enum Error {
    NoRunnableThreads,
    ThreadAlreadyAssignedToLp,
    ThreadNotAssignedToThisLp,
    InvalidThread,
}

/// Tracks the per-LP quantum timer state.
///
/// `pending` is the "a context switch has been requested" flag: it is set both
/// when the quantum timer fires and when a thread calls `yield_lp`. `armed`
/// tracks whether a quantum `TimerEvent` is currently in flight in the timer
/// queue, so that exactly one is ever queued at a time — otherwise every manual
/// yield would enqueue another quantum event and the timer queue would grow
/// without bound.
#[derive(Debug)]
struct TimerEventObserver {
    pending: AtomicBool,
    armed: AtomicBool,
}

impl TimerEventObserver {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            armed: AtomicBool::new(false),
        }
    }
}

impl Observer for TimerEventObserver {
    fn notify(self: Arc<Self>) {
        // The quantum event fired: it is no longer in flight, and a context
        // switch is now due.
        self.armed.store(false, Ordering::Release);
        self.pending.store(true, Ordering::Release);
    }
}
