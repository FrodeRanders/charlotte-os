//! Self-test: the running-mean/variance accumulator used by scheduler and
//! device statistics.
//!
//! Why: [`RunningStatistics`] computes count/min/max/mean/sample-variance
//! incrementally (no full-sample storage) and supports merging two
//! accumulators — properties the scheduler and device layers rely on. This
//! test pins those math guarantees so a future rewrite cannot silently change
//! the numbers.
//!
//! What it does:
//! - feeds the canonical 8-sample set `[2,4,4,4,5,5,7,9]` and checks count/min/max/total/mean and
//!   the sample variance (`32/7`, stored as a reduced ratio);
//! - merges the set split into two halves (`left.merge(right)`) and checks the result equals the
//!   single-pass snapshot;
//! - resets and checks the accumulator is empty (count 0, no variance).
//!
//! Expected outcome: all assertions hold; the merge is exactly equivalent to
//! one pass, and a reset returns to the empty state. Logs
//! `Running-statistics tests passed.`

use crate::{
    klib::statistics::RunningStatistics,
    logln,
};

pub fn test_running_statistics() {
    let mut statistics = RunningStatistics::new();
    for sample in [2, 4, 4, 4, 5, 5, 7, 9] {
        statistics.add_sample(sample);
    }
    let snapshot = statistics.snapshot();
    assert_eq!(snapshot.count, 8);
    assert_eq!(snapshot.min, Some(2));
    assert_eq!(snapshot.max, Some(9));
    assert_eq!(snapshot.total, 40);
    assert_eq!(snapshot.mean_ratio(), Some((40, 8)));
    // This canonical sample has sample variance 32 / 7.
    assert_eq!(snapshot.sample_variance_ratio(), Some((256, 56)));

    let mut left = RunningStatistics::new();
    let mut right = RunningStatistics::new();
    for sample in [2, 4, 4, 4] {
        left.add_sample(sample);
    }
    for sample in [5, 5, 7, 9] {
        right.add_sample(sample);
    }
    left.merge(right);
    assert_eq!(left.snapshot(), snapshot);

    left.reset();
    assert_eq!(left.snapshot().count, 0);
    assert_eq!(left.snapshot().sample_variance_ratio(), None);
    logln!("Running-statistics tests passed.");
}
