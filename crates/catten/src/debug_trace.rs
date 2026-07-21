//! Stub — debug trace is on the sched/cq-generation-counter branch.
//! The trace calls in completion/mod.rs compile to no-ops on dev.

pub fn trace(_tag: u64, _a: u64, _b: u64, _c: u64) {}

pub const TAG_CQ_WAIT_ENTER: u64 = 0;
pub const TAG_CQ_WAIT_RESUME: u64 = 0;
pub const TAG_CQ_WAIT_FAST: u64 = 0;
pub const TAG_CQ_WAIT_GUARD: u64 = 0;
pub const TAG_COMPLETE: u64 = 0;
pub const TAG_COMPLETE_DETACHED: u64 = 0;
pub const TAG_WAKE: u64 = 0;
pub const TAG_SIGNAL_CQ: u64 = 0;
pub const TAG_WAKER_NOTIFY: u64 = 0;
pub const TAG_SUBMIT_TIMER_OK: u64 = 0;
