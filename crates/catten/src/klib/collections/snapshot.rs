//! Fallible, bounded scratch for diagnostic snapshots. Prepare outside source
//! guards; capture never grows storage and rejects a source that outgrew it.

use alloc::vec::Vec;

/// Per diagnostic request, across capture buffers and encoded scratch. This
/// does not replace general heap or aggregate principal admission.
pub(crate) const MAX_WORKING_BYTES: usize =
    crate::resource_policy::POLICY.local.telemetry_working_bytes;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Capacity,
    Allocation,
    SourceChanged,
}

pub(crate) struct Budget {
    remaining: usize,
}

impl Budget {
    pub(crate) fn new(bytes: usize) -> Self {
        Self {
            remaining: bytes,
        }
    }

    pub(crate) fn prepare<T: Copy>(&mut self, count: usize) -> Result<Buffer<T>, Error> {
        self.prepare_with(count, |values, count| {
            values.try_reserve_exact(count).map_err(|_| Error::Allocation)
        })
    }

    fn prepare_with<T: Copy>(
        &mut self,
        count: usize,
        allocate: impl FnOnce(&mut Vec<T>, usize) -> Result<(), Error>,
    ) -> Result<Buffer<T>, Error> {
        let bytes = count.checked_mul(core::mem::size_of::<T>()).ok_or(Error::Capacity)?;
        if bytes > self.remaining {
            return Err(Error::Capacity);
        }
        let mut values = Vec::new();
        allocate(&mut values, count)?;
        // Charge actual capacity too, rather than trusting a reserve request.
        let actual =
            values.capacity().checked_mul(core::mem::size_of::<T>()).ok_or(Error::Capacity)?;
        if actual > self.remaining || values.capacity() < count {
            return Err(Error::Capacity);
        }
        self.remaining -= actual;
        Ok(Buffer {
            values,
            limit: count,
        })
    }
}

pub(crate) struct Buffer<T: Copy> {
    values: Vec<T>,
    limit: usize,
}

impl<T: Copy> Buffer<T> {
    pub(crate) fn push(&mut self, value: T) -> Result<(), Error> {
        if self.values.len() == self.limit {
            return Err(Error::SourceChanged);
        }
        self.values.push(value);
        Ok(())
    }

    pub(crate) fn into_vec(self) -> Vec<T> {
        self.values
    }
}

#[cfg(test)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_rejects_before_allocation_and_checks_arithmetic() {
        let mut budget = Budget::new(7);
        assert!(matches!(
            budget.prepare_with::<u64>(1, |_, _| panic!("allocated")),
            Err(Error::Capacity)
        ));
        assert!(matches!(budget.prepare::<u64>(usize::MAX), Err(Error::Capacity)));
        assert_eq!(budget.remaining, 7);
    }

    #[test]
    fn allocation_rejection_preserves_budget() {
        let mut budget = Budget::new(16);
        assert!(matches!(
            budget.prepare_with::<u64>(2, |_, _| Err(Error::Allocation)),
            Err(Error::Allocation)
        ));
        assert_eq!(budget.remaining, 16);
        let mut buffer = budget.prepare::<u64>(2).unwrap();
        buffer.push(11).unwrap();
        buffer.push(22).unwrap();
        assert_eq!(buffer.into_vec(), [11, 22]);
        assert_eq!(budget.remaining, 0);
    }

    #[test]
    fn growth_rejects_without_reallocating_or_overwriting_prefix() {
        let mut budget = Budget::new(16);
        let mut buffer = budget.prepare::<u64>(2).unwrap();
        let allocation = buffer.values.as_ptr();
        buffer.push(1).unwrap();
        buffer.push(2).unwrap();
        assert_eq!(buffer.push(3), Err(Error::SourceChanged));
        assert_eq!(buffer.values.as_ptr(), allocation);
        assert_eq!(buffer.into_vec(), [1, 2]);
    }

    #[test]
    fn independent_buffers_share_one_working_set_limit() {
        let mut budget = Budget::new(24);
        let first = budget.prepare::<u64>(2).unwrap();
        assert!(matches!(budget.prepare::<u64>(2), Err(Error::Capacity)));
        let _second = budget.prepare::<u8>(8).unwrap();
        // Disposing an earlier buffer does not restart this request's budget.
        drop(first);
        assert!(matches!(budget.prepare::<u8>(1), Err(Error::Capacity)));
    }

    #[test]
    fn shorter_and_empty_sources_are_complete_snapshots() {
        let mut budget = Budget::new(16);
        let mut shorter = budget.prepare::<u64>(2).unwrap();
        shorter.push(7).unwrap();
        assert_eq!(shorter.into_vec(), [7]);
        let mut empty = budget.prepare::<u64>(0).unwrap();
        assert_eq!(empty.push(1), Err(Error::SourceChanged));
        assert!(empty.into_vec().is_empty());
    }
}
