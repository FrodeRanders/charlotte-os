//! Fixed-size boot calibration: incomplete/zero samples cannot publish a timer.
pub(super) const NUM_SAMPLES: usize = 10;
pub(super) const SAMPLE_TICKS: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Incomplete,
    Zero,
    NonMonotonic,
    Overflow,
}

/// Raw failed-sample evidence stays on the boot stack; no sample vector or
/// runtime diagnostic allocation is needed.
#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    pub tsc_start: u64,
    pub tsc_end: u64,
    pub tsc_period_ps: u128,
    pub final_count: u32,
    pub completed: bool,
}

impl Sample {
    pub fn resolution(self) -> Result<u128, Error> {
        if !self.completed || self.final_count != 0 {
            return Err(Error::Incomplete);
        }
        let cycles = self.tsc_end.checked_sub(self.tsc_start).ok_or(Error::NonMonotonic)?;
        let duration = (cycles as u128).checked_mul(self.tsc_period_ps).ok_or(Error::Overflow)?;
        let period = duration / SAMPLE_TICKS as u128;
        if period == 0 {
            return Err(Error::Zero);
        }
        Ok(period)
    }
}

/// The timestamp interval encloses programming and the complete countdown.
/// `timestamp` must order subsequent hardware programming after the timestamp;
/// `wait` supplies bounded completion and the last observed hardware count.
pub(super) fn sample(
    mut timestamp: impl FnMut() -> u64,
    arm: impl FnOnce(u32),
    wait: impl FnOnce() -> (bool, u32),
    tsc_period_ps: u128,
) -> Sample {
    let tsc_start = timestamp();
    arm(SAMPLE_TICKS);
    let (completed, final_count) = wait();
    let tsc_end = timestamp();
    Sample {
        tsc_start,
        tsc_end,
        tsc_period_ps,
        final_count,
        completed,
    }
}

pub(super) fn resolution(mut sample: impl FnMut() -> Result<u128, Error>) -> Result<u128, Error> {
    let mut sum = 0u128;
    for _ in 0..NUM_SAMPLES {
        let value = sample()?;
        if value == 0 {
            return Err(Error::Zero);
        }
        sum = sum.checked_add(value).ok_or(Error::Overflow)?;
    }
    Ok(sum / NUM_SAMPLES as u128)
}
