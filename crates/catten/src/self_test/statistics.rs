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
