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

/// Free frames below one sixteenth of usable RAM damp history-based growth and
/// refuse in-life stack growth, so a pressured node fails closed.
pub const STACK_GROWTH_RESERVE_DIVISOR: u64 = 16;

/// Choose the user-stack page count for the next generation of a service from
/// the previous generation's touched high-water mark.
///
/// One page of headroom is added to the observed high-water mark, and the
/// result is clamped to `[default_pages, max_pages]`. A zero high-water mark —
/// no recorded history, for example a cold boot — selects the default, so the
/// first generation of every service always gets the same policy.
pub fn adaptive_stack_pages(
    previous_high_water_pages: u64,
    default_pages: usize,
    max_pages: usize,
) -> usize {
    if previous_high_water_pages == 0 {
        return default_pages;
    }
    let pages = usize::try_from(previous_high_water_pages).unwrap_or(usize::MAX).saturating_add(1);
    pages.clamp(default_pages, max_pages)
}

/// Choose the heap capacity for the next generation of a service from the
/// previous generation's peak allocation.
///
/// Twice the observed peak plus 256 KiB of slack is rounded up to a page and
/// clamped to `[min_bytes, max_bytes]`. A zero peak — no recorded history, for
/// example a cold boot — selects `default_bytes`, so the first generation of
/// every service keeps the full default capacity.
pub fn adaptive_heap_bytes(
    previous_peak_bytes: u64,
    default_bytes: usize,
    min_bytes: usize,
    max_bytes: usize,
) -> usize {
    if previous_peak_bytes == 0 {
        return default_bytes;
    }
    let peak = usize::try_from(previous_peak_bytes).unwrap_or(usize::MAX);
    let with_slack = peak.saturating_mul(2).saturating_add(256 * 1024);
    let rounded = with_slack.checked_next_multiple_of(4096).unwrap_or(usize::MAX);
    rounded.clamp(min_bytes, max_bytes)
}

/// Damp stack growth while physical memory is scarce.
///
/// Returns `desired_pages` when free frames are at or above `reserve_frames`,
/// and falls back to `floor_pages` below the reserve. The floor is the ordinary
/// default, so pressure only withholds growth justified by history; it never
/// pushes a service below the size its first generation would receive.
pub fn damp_stack_growth(
    desired_pages: usize,
    floor_pages: usize,
    free_frames: u64,
    reserve_frames: u64,
) -> usize {
    if free_frames < reserve_frames {
        desired_pages.min(floor_pages)
    } else {
        desired_pages.max(floor_pages)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        JoinDisposition,
        ThreadIdentity,
        TimedWaitOutcome,
        adaptive_stack_pages,
        claim_generation,
        classify_join,
        classify_timed_wait,
        damp_stack_growth,
    };

    #[test]
    fn adaptive_heap_bytes_sizes_from_peak_or_default() {
        use super::adaptive_heap_bytes;
        assert_eq!(adaptive_heap_bytes(0, 4 << 20, 1 << 20, 5 << 20), 4 << 20);
        assert_eq!(adaptive_heap_bytes(1, 4 << 20, 1 << 20, 5 << 20), 1 << 20);
        assert_eq!(adaptive_heap_bytes(1 << 20, 4 << 20, 1 << 20, 5 << 20), 2 << 20 | 1 << 18);
        assert_eq!(adaptive_heap_bytes(u64::MAX, 4 << 20, 1 << 20, 5 << 20), 5 << 20);
        assert_eq!(adaptive_heap_bytes(1, 4 << 20, 4 << 20, 5 << 20), 4 << 20);
    }

    #[test]
    fn damp_stack_growth_only_withholds_history_based_growth() {
        assert_eq!(damp_stack_growth(8, 4, 100, 50), 8);
        assert_eq!(damp_stack_growth(8, 4, 50, 50), 8);
        assert_eq!(damp_stack_growth(8, 4, 49, 50), 4);
        assert_eq!(damp_stack_growth(4, 4, 0, 50), 4);
        assert_eq!(damp_stack_growth(2, 4, 100, 50), 4);
    }

    #[test]
    fn adaptive_stack_pages_adds_headroom_and_clamps() {
        assert_eq!(adaptive_stack_pages(0, 4, 64), 4);
        assert_eq!(adaptive_stack_pages(1, 4, 64), 4);
        assert_eq!(adaptive_stack_pages(4, 4, 64), 5);
        assert_eq!(adaptive_stack_pages(63, 4, 64), 64);
        assert_eq!(adaptive_stack_pages(64, 4, 64), 64);
        assert_eq!(adaptive_stack_pages(u64::MAX, 4, 64), 64);
    }

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
