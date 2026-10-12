//! Total operation deadlines: retrying a short wait must not reset admission.

pub struct Deadline {
    start: u64,
    frequency_hz: u64,
    budget_ms: u64,
}

impl Deadline {
    pub fn after(budget_ms: u64) -> Self {
        let (start, frequency_hz) = catten_syscall::monotonic_clock();
        Self::at(start, frequency_hz, budget_ms)
    }

    pub const fn at(start: u64, frequency_hz: u64, budget_ms: u64) -> Self {
        Self {
            start,
            frequency_hz,
            budget_ms,
        }
    }

    pub fn expired(&self) -> bool {
        self.expired_at(catten_syscall::monotonic_clock().0)
    }

    /// Remaining total budget, rounded up to the kernel's millisecond wait.
    pub fn remaining_ms(&self) -> u64 {
        self.remaining_ms_at(catten_syscall::monotonic_clock().0)
    }

    pub fn remaining_ms_at(&self, now: u64) -> u64 {
        if self.frequency_hz == 0 || now < self.start {
            return 0;
        }
        let budget = u128::from(self.budget_ms) * u128::from(self.frequency_hz);
        let elapsed = u128::from(now - self.start) * 1000;
        let remaining = budget.saturating_sub(elapsed).div_ceil(u128::from(self.frequency_hz));
        remaining as u64 // Cannot exceed the original u64 budget.
    }

    pub fn expired_at(&self, now: u64) -> bool {
        self.frequency_hz == 0
            || now < self.start
            || u128::from(now - self.start) * 1000
                >= u128::from(self.budget_ms) * u128::from(self.frequency_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_do_not_restart_the_deadline() {
        let deadline = Deadline::at(100, 1000, 50);
        assert!(!deadline.expired_at(100));
        assert!(!deadline.expired_at(149));
        assert!(deadline.expired_at(150));
        assert!(deadline.expired_at(151));
    }

    #[test]
    fn invalid_clock_fails_closed_without_arithmetic_overflow() {
        assert!(Deadline::at(100, 1000, 50).expired_at(99));
        assert!(Deadline::at(0, 0, 50).expired_at(0));
        assert!(Deadline::at(0, 1, 1).expired_at(u64::MAX));
        assert!(!Deadline::at(0, u64::MAX, u64::MAX).expired_at(u64::MAX));
    }
    #[test]
    fn remaining_budget_rounds_up_without_restart_or_overflow() {
        let deadline = Deadline::at(10, 3000, 2);
        assert_eq!(deadline.remaining_ms_at(10), 2);
        assert_eq!(deadline.remaining_ms_at(11), 2);
        assert_eq!(deadline.remaining_ms_at(13), 1);
        assert_eq!(deadline.remaining_ms_at(16), 0);
        assert_eq!(deadline.remaining_ms_at(17), 0);
        assert_eq!(deadline.remaining_ms_at(9), 0);
        assert_eq!(Deadline::at(0, 0, 10).remaining_ms_at(0), 0);
        assert_eq!(Deadline::at(0, u64::MAX, u64::MAX).remaining_ms_at(0), u64::MAX);
        assert_eq!(Deadline::at(0, 1, 1).remaining_ms_at(u64::MAX), 0);
    }
}
