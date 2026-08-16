//! Pure, host-testable lifecycle decisions used at kernel/userspace boundaries.
#![no_std]

/// Stable identity for an occupant of a recyclable numeric thread slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadIdentity {
    tid: u64,
    generation: u64,
}

impl ThreadIdentity {
    pub const fn new(tid: u64, generation: u64) -> Self {
        Self {
            tid,
            generation,
        }
    }

    pub const fn tid(self) -> u64 {
        self.tid
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinDisposition {
    ObserveCurrent,
    AlreadyExited,
}

/// Decide whether a captured thread handle may observe the current slot.
/// Missing slots and replacement generations both mean the captured thread
/// has already exited.
pub const fn classify_join(
    captured: ThreadIdentity,
    current: Option<ThreadIdentity>,
) -> JoinDisposition {
    match current {
        Some(current)
            if current.tid == captured.tid && current.generation == captured.generation =>
        {
            JoinDisposition::ObserveCurrent
        }
        _ => JoinDisposition::AlreadyExited,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimedWaitOutcome {
    Work,
    Timeout,
}

/// Claim a non-zero monotonically increasing generation from a counter that
/// stores the next value to allocate.
///
/// Returning `None` at zero or `u64::MAX` makes exhaustion fail closed: zero
/// remains available as an ABI sentinel and no previously issued generation
/// can be reused after integer wraparound.
pub const fn claim_generation(next: u64) -> Option<(u64, u64)> {
    if next == 0 || next == u64::MAX {
        None
    } else {
        Some((next, next + 1))
    }
}

/// Classify a timed wait after wakeup. Any observed generation change wins
/// over the watchdog because publication happened after the waiter captured
/// `registered_generation`.
pub const fn classify_timed_wait(
    registered_generation: u64,
    current_generation: u64,
) -> TimedWaitOutcome {
    if current_generation == registered_generation {
        TimedWaitOutcome::Timeout
    } else {
        TimedWaitOutcome::Work
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        JoinDisposition,
        ThreadIdentity,
        TimedWaitOutcome,
        claim_generation,
        classify_join,
        classify_timed_wait,
    };

    #[test]
    fn generation_claims_fail_closed_before_wrap_or_zero() {
        assert_eq!(claim_generation(0), None);
        assert_eq!(claim_generation(1), Some((1, 2)));
        assert_eq!(claim_generation(u64::MAX - 1), Some((u64::MAX - 1, u64::MAX)));
        assert_eq!(claim_generation(u64::MAX), None);
    }

    #[test]
    fn join_exhaustively_rejects_other_slots_and_generations() {
        let values = [0, 1, 2, u64::MAX];
        for captured_tid in values {
            for captured_generation in values {
                let captured = ThreadIdentity::new(captured_tid, captured_generation);
                assert_eq!(classify_join(captured, None), JoinDisposition::AlreadyExited);
                for current_tid in values {
                    for current_generation in values {
                        let current = ThreadIdentity::new(current_tid, current_generation);
                        let expected = if current == captured {
                            JoinDisposition::ObserveCurrent
                        } else {
                            JoinDisposition::AlreadyExited
                        };
                        assert_eq!(classify_join(captured, Some(current)), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn timed_wait_exhaustively_prefers_every_generation_change() {
        let values = [0, 1, 2, u64::MAX];
        for registered in values {
            for current in values {
                let expected = if current == registered {
                    TimedWaitOutcome::Timeout
                } else {
                    TimedWaitOutcome::Work
                };
                assert_eq!(classify_timed_wait(registered, current), expected);
            }
        }
    }
}
