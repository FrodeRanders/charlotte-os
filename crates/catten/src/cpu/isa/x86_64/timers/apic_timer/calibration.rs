//! Fixed-size boot calibration: incomplete/zero samples cannot publish a timer.
pub(super) const NUM_SAMPLES: usize = 10;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    Incomplete,
    Zero,
    Overflow,
}

pub(super) fn resolution(mut sample: impl FnMut() -> Option<u128>) -> Result<u128, Error> {
    let mut sum = 0u128;
    for _ in 0..NUM_SAMPLES {
        let value = sample().ok_or(Error::Incomplete)?;
        if value == 0 {
            return Err(Error::Zero);
        }
        sum = sum.checked_add(value).ok_or(Error::Overflow)?;
    }
    Ok(sum / NUM_SAMPLES as u128)
}
