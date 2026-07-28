//! Integer running statistics for low-overhead kernel instrumentation.
//!
//! Kernel code deliberately avoids floating-point arithmetic: using FP/SIMD
//! registers in privileged code would require saving additional architectural
//! state on every transition. Samples are therefore accumulated exactly as
//! integers. Snapshots expose the rational components needed to calculate
//! mean, sample variance, standard deviation, and coefficient of variation in
//! userspace.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatisticsSnapshot {
    pub count: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub total: u128,
    pub sum_of_squares: u128,
    pub saturated: bool,
}

impl StatisticsSnapshot {
    /// Components of the exact arithmetic mean: `total / count`.
    pub const fn mean_ratio(self) -> Option<(u128, u64)> {
        if self.count == 0 {
            None
        } else {
            Some((self.total, self.count))
        }
    }

    /// Components of sample variance:
    /// `(count * sum(x²) - sum(x)²) / (count * (count - 1))`.
    pub fn sample_variance_ratio(self) -> Option<(u128, u128)> {
        if self.count < 2 || self.saturated {
            return None;
        }
        let count = u128::from(self.count);
        let numerator = count
            .checked_mul(self.sum_of_squares)?
            .checked_sub(self.total.checked_mul(self.total)?)?;
        Some((numerator, count * (count - 1)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningStatistics {
    count: u64,
    min: u64,
    max: u64,
    total: u128,
    sum_of_squares: u128,
    saturated: bool,
}

impl RunningStatistics {
    pub const fn new() -> Self {
        Self {
            count: 0,
            min: u64::MAX,
            max: 0,
            total: 0,
            sum_of_squares: 0,
            saturated: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn add_sample(&mut self, sample: u64) {
        self.min = self.min.min(sample);
        self.max = self.max.max(sample);
        self.count = match self.count.checked_add(1) {
            Some(count) => count,
            None => {
                self.saturated = true;
                u64::MAX
            }
        };
        self.total = match self.total.checked_add(u128::from(sample)) {
            Some(total) => total,
            None => {
                self.saturated = true;
                u128::MAX
            }
        };
        let square = u128::from(sample) * u128::from(sample);
        self.sum_of_squares = match self.sum_of_squares.checked_add(square) {
            Some(sum) => sum,
            None => {
                self.saturated = true;
                u128::MAX
            }
        };
    }

    pub fn merge(&mut self, other: Self) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = other;
            return;
        }
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        let count = self.count.checked_add(other.count);
        let total = self.total.checked_add(other.total);
        let sum_of_squares = self.sum_of_squares.checked_add(other.sum_of_squares);
        self.saturated = self.saturated
            || other.saturated
            || count.is_none()
            || total.is_none()
            || sum_of_squares.is_none();
        self.count = count.unwrap_or(u64::MAX);
        self.total = total.unwrap_or(u128::MAX);
        self.sum_of_squares = sum_of_squares.unwrap_or(u128::MAX);
    }

    pub const fn snapshot(self) -> StatisticsSnapshot {
        StatisticsSnapshot {
            count: self.count,
            min: if self.count == 0 {
                None
            } else {
                Some(self.min)
            },
            max: if self.count == 0 {
                None
            } else {
                Some(self.max)
            },
            total: self.total,
            sum_of_squares: self.sum_of_squares,
            saturated: self.saturated,
        }
    }
}

impl Default for RunningStatistics {
    fn default() -> Self {
        Self::new()
    }
}
