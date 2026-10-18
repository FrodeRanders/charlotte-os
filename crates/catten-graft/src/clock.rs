//! Convert a stable monotonic counter to elapsed Raft milliseconds.
//!
//! Wake counts cannot measure elapsed time: one notification may cover a long
//! reactor pause. Keep the original counter epoch so submillisecond samples do
//! not discard fractions or accumulate conversion drift.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockError {
    InvalidFrequency,
    FrequencyChanged,
    CounterRegressed,
    MillisecondsExhausted,
}

pub struct MonotonicClock {
    origin: u64,
    last: u64,
    frequency_hz: u64,
}

impl MonotonicClock {
    pub fn new(counter: u64, frequency_hz: u64) -> Result<Self, ClockError> {
        if frequency_hz == 0 {
            return Err(ClockError::InvalidFrequency);
        }
        Ok(Self {
            origin: counter,
            last: counter,
            frequency_hz,
        })
    }

    /// Reject an invalid sample without changing the captured clock epoch.
    pub fn sample(&mut self, counter: u64, frequency_hz: u64) -> Result<u64, ClockError> {
        if frequency_hz != self.frequency_hz {
            return Err(ClockError::FrequencyChanged);
        }
        if counter < self.last {
            return Err(ClockError::CounterRegressed);
        }
        let millis = u128::from(counter - self.origin) * 1000 / u128::from(frequency_hz);
        let millis = u64::try_from(millis).map_err(|_| ClockError::MillisecondsExhausted)?;
        self.last = counter;
        Ok(millis)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClockError,
        MonotonicClock,
    };

    #[test]
    fn elapsed_time_includes_the_whole_reactor_pause() {
        let mut clock = MonotonicClock::new(1_000, 1_000).unwrap();
        assert_eq!(clock.sample(1_202, 1_000), Ok(202));
        assert_eq!(clock.sample(6_202, 1_000), Ok(5_202));
        assert_eq!(clock.sample(6_202, 1_000), Ok(5_202));
    }

    #[test]
    fn frequent_samples_preserve_counter_fractions() {
        let mut clock = MonotonicClock::new(100, 3_000).unwrap();
        for delta in 0..=30 {
            assert_eq!(clock.sample(100 + delta, 3_000), Ok(delta / 3));
        }
    }

    #[test]
    fn invalid_samples_do_not_reset_the_epoch_or_last_valid_counter() {
        assert!(matches!(MonotonicClock::new(0, 0), Err(ClockError::InvalidFrequency)));
        let mut clock = MonotonicClock::new(100, 1_000).unwrap();
        assert_eq!(clock.sample(200, 1_000), Ok(100));
        assert_eq!(clock.sample(199, 1_000), Err(ClockError::CounterRegressed));
        assert_eq!(clock.sample(300, 0), Err(ClockError::FrequencyChanged));
        assert_eq!(clock.sample(300, 2_000), Err(ClockError::FrequencyChanged));
        assert_eq!(clock.sample(250, 1_000), Ok(150));
    }

    #[test]
    fn conversion_checks_its_millisecond_range() {
        let mut clock = MonotonicClock::new(0, 1).unwrap();
        assert_eq!(clock.sample(u64::MAX, 1), Err(ClockError::MillisecondsExhausted));
        assert_eq!(clock.sample(1, 1), Ok(1_000));
        let mut clock = MonotonicClock::new(0, u64::MAX).unwrap();
        assert_eq!(clock.sample(u64::MAX, u64::MAX), Ok(1_000));
    }
}
