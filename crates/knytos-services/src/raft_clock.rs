//! Kernel-monotonic time for the two Raft service reactors.

use catten_graft::{
    clock::MonotonicClock,
    node::RaftNode,
};

pub fn start() -> MonotonicClock {
    let (counter, frequency) = catten_syscall::monotonic_clock();
    MonotonicClock::new(counter, frequency).unwrap_or_else(|error| {
        catten_rt::logln!("[raft] invalid monotonic clock: {:?}", error);
        catten_rt::domain_abort()
    })
}

/// Sample after waits and synchronous work, before using protocol time.
/// An invalid clock cannot authorize reads, elections or a fresh retry budget.
/// Abort with existing pending owners live; do not manufacture a tick.
pub fn refresh(node: &mut RaftNode, clock: &mut MonotonicClock) {
    let (counter, frequency) = catten_syscall::monotonic_clock();
    let millis = clock.sample(counter, frequency).unwrap_or_else(|error| {
        catten_rt::logln!("[raft] monotonic clock rejected: {:?}", error);
        catten_rt::domain_abort()
    });
    node.set_millis(millis);
}
