use core::cell::Cell;

use crate::{
    calibration,
    measure,
};

#[test]
fn elapsed_countdown_before_timestamp_is_still_measured() {
    let allocations = measure(|| {
        let clock = Cell::new(100u64);
        let sample = calibration::sample(
            || clock.get(),
            |ticks| {
                assert_eq!(ticks, calibration::SAMPLE_TICKS);
                // The countdown expires before the programming call returns.
                // Starting the TSC interval after arm would lose all its time.
                clock.set(clock.get() + u64::from(ticks));
            },
            || (true, 0),
            1000,
        );
        assert_eq!(sample.resolution(), Ok(1000), "complete countdown escaped measured interval");
        assert_eq!(sample.tsc_start, 100);
        assert_eq!(sample.tsc_end, 10100);
    });
    assert_eq!(allocations, 0);
}

#[test]
fn raw_samples_reject_incomplete_zero_reversed_and_overflow() {
    use calibration::{
        Error,
        Sample,
    };
    let valid = Sample {
        tsc_start: 100,
        tsc_end: 10100,
        tsc_period_ps: 1000,
        final_count: 0,
        completed: true,
    };
    assert_eq!(valid.resolution(), Ok(1000));
    for sample in [
        Sample {
            completed: false,
            ..valid
        },
        Sample {
            final_count: 1,
            ..valid
        },
    ] {
        assert_eq!(sample.resolution(), Err(Error::Incomplete));
    }
    assert_eq!(
        Sample {
            tsc_end: 99,
            ..valid
        }
        .resolution(),
        Err(Error::NonMonotonic)
    );
    for sample in [
        Sample {
            tsc_end: 100,
            ..valid
        },
        Sample {
            tsc_period_ps: 0,
            ..valid
        },
        Sample {
            tsc_end: 101,
            tsc_period_ps: 1,
            ..valid
        },
    ] {
        assert_eq!(sample.resolution(), Err(Error::Zero));
    }
    assert_eq!(
        Sample {
            tsc_period_ps: u128::MAX,
            ..valid
        }
        .resolution(),
        Err(Error::Overflow)
    );
}

#[test]
fn every_sample_error_stops_without_retry_or_later_sampling() {
    use calibration::Error;
    for failure in 0..calibration::NUM_SAMPLES {
        for error in [Error::Incomplete, Error::Zero, Error::NonMonotonic, Error::Overflow] {
            let mut calls = 0;
            assert_eq!(
                calibration::resolution(|| {
                    let index = calls;
                    calls += 1;
                    if index == failure {
                        Err(error)
                    } else {
                        Ok(1000)
                    }
                }),
                Err(error)
            );
            assert_eq!(calls, failure + 1);
        }
    }
}
