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
}
