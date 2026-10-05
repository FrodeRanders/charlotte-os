//! Virtual scratch extents. Admission prepares metadata before publication;
//! release removes one exact live reservation without allocating.

use alloc::vec::Vec;

const PAGE_SIZE: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidRange,
    OutOfSpace,
    AllocationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Extent {
    offset: usize,
    bytes: usize,
}

#[derive(Debug)]
pub(crate) struct ScratchWindow {
    pub(crate) generation: usize,
    bytes: usize,
    /// Sorted, disjoint live reservations. Gaps are implicit, so releasing a
    /// range never needs a new free-extent node or a coalescing allocation.
    occupied: Vec<Extent>,
}

impl ScratchWindow {
    pub(crate) fn new(generation: usize, bytes: usize) -> Self {
        Self {
            generation,
            bytes,
            occupied: Vec::new(),
        }
    }

    pub(crate) fn reserve(&mut self, bytes: usize) -> Result<usize, Error> {
        self.reserve_with(bytes, |occupied| {
            occupied.try_reserve(1).map_err(|_| Error::AllocationFailed)
        })
    }

    fn reserve_with(
        &mut self,
        bytes: usize,
        prepare: impl FnOnce(&mut Vec<Extent>) -> Result<(), Error>,
    ) -> Result<usize, Error> {
        if bytes == 0 || !bytes.is_multiple_of(PAGE_SIZE) {
            return Err(Error::InvalidRange);
        }
        let mut offset = 0usize;
        let mut index = 0;
        for extent in &self.occupied {
            if extent.offset - offset >= bytes {
                break;
            }
            offset = extent.offset + extent.bytes;
            index += 1;
        }
        if offset.checked_add(bytes).is_none_or(|end| end > self.bytes) {
            return Err(Error::OutOfSpace);
        }
        // No logical mutation follows failed preparation. The production
        // callback reserves space for this one insertion, before publication.
        prepare(&mut self.occupied)?;
        assert!(self.occupied.len() < self.occupied.capacity(), "scratch metadata not prepared");
        self.occupied.insert(
            index,
            Extent {
                offset,
                bytes,
            },
        );
        Ok(offset)
    }

    pub(crate) fn release(&mut self, offset: usize, bytes: usize) -> Result<(), Error> {
        let index = self
            .occupied
            .binary_search_by_key(&offset, |extent| extent.offset)
            .map_err(|_| Error::InvalidRange)?;
        if self.occupied[index].bytes != bytes {
            return Err(Error::InvalidRange);
        }
        self.occupied.remove(index);
        Ok(())
    }
}

#[cfg(test)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_fit_reuses_and_combines_implicit_gaps_without_release_allocation() {
        let mut window = ScratchWindow::new(7, PAGE_SIZE * 8);
        assert_eq!(window.generation, 7);
        let a = window.reserve(PAGE_SIZE * 2).unwrap();
        let b = window.reserve(PAGE_SIZE).unwrap();
        let c = window.reserve(PAGE_SIZE * 2).unwrap();
        let capacity = window.occupied.capacity();
        window.release(b, PAGE_SIZE).unwrap();
        window.release(a, PAGE_SIZE * 2).unwrap();
        assert_eq!(window.occupied.capacity(), capacity);
        assert_eq!(window.reserve(PAGE_SIZE * 3), Ok(0));
        assert_eq!(window.reserve(PAGE_SIZE), Ok(PAGE_SIZE * 5));
        window.release(c, PAGE_SIZE * 2).unwrap();
        assert_eq!(window.reserve(PAGE_SIZE * 2), Ok(PAGE_SIZE * 3));
    }

    #[test]
    fn release_requires_one_exact_live_extent_and_leaves_rejections_unchanged() {
        let mut window = ScratchWindow::new(1, PAGE_SIZE * 8);
        window.reserve(PAGE_SIZE * 2).unwrap();
        window.reserve(PAGE_SIZE).unwrap();
        let expected = window.occupied.clone();
        for (offset, bytes) in [
            (0, 0),
            (0, PAGE_SIZE),
            (PAGE_SIZE, PAGE_SIZE),
            (0, PAGE_SIZE * 3),
            (PAGE_SIZE * 8, PAGE_SIZE),
            (usize::MAX, PAGE_SIZE),
        ] {
            assert_eq!(window.release(offset, bytes), Err(Error::InvalidRange));
            assert_eq!(window.occupied, expected);
        }
        window.release(0, PAGE_SIZE * 2).unwrap();
        assert_eq!(window.release(0, PAGE_SIZE * 2), Err(Error::InvalidRange));
        assert_eq!(window.occupied.len(), 1);
        assert_eq!(window.reserve(PAGE_SIZE * 2), Ok(0));
    }

    #[test]
    fn failed_metadata_preflight_does_not_publish_or_consume_virtual_space() {
        let mut window = ScratchWindow::new(1, PAGE_SIZE * 4);
        assert_eq!(
            window.reserve_with(PAGE_SIZE, |_| Err(Error::AllocationFailed)),
            Err(Error::AllocationFailed)
        );
        assert!(window.occupied.is_empty());
        assert_eq!(window.reserve(PAGE_SIZE), Ok(0));
        let expected = window.occupied.clone();
        assert_eq!(
            window.reserve_with(PAGE_SIZE, |_| Err(Error::AllocationFailed)),
            Err(Error::AllocationFailed)
        );
        assert_eq!(window.occupied, expected);
        assert_eq!(window.reserve(PAGE_SIZE), Ok(PAGE_SIZE));
    }

    #[test]
    fn invalid_exhausted_and_overflow_requests_never_reach_metadata_preflight() {
        let mut window = ScratchWindow::new(1, PAGE_SIZE);
        for bytes in [0, 1, PAGE_SIZE - 1] {
            assert_eq!(
                window.reserve_with(bytes, |_| panic!("invalid request admitted")),
                Err(Error::InvalidRange)
            );
        }
        assert_eq!(window.reserve(PAGE_SIZE), Ok(0));
        assert_eq!(
            window.reserve_with(PAGE_SIZE, |_| panic!("exhausted request admitted")),
            Err(Error::OutOfSpace)
        );
        let largest = usize::MAX - (PAGE_SIZE - 1);
        let mut large = ScratchWindow::new(2, largest);
        assert_eq!(large.reserve(largest), Ok(0));
        assert_eq!(large.reserve(PAGE_SIZE), Err(Error::OutOfSpace));
        large.release(0, largest).unwrap();
        assert_eq!(large.reserve(PAGE_SIZE), Ok(0));
    }

    #[test]
    fn independent_lifetimes_start_with_independent_windows() {
        let mut old = ScratchWindow::new(1, PAGE_SIZE * 2);
        let mut new = ScratchWindow::new(2, PAGE_SIZE * 2);
        assert_eq!(old.reserve(PAGE_SIZE), Ok(0));
        assert_eq!(new.reserve(PAGE_SIZE), Ok(0));
        old.release(0, PAGE_SIZE).unwrap();
        assert_eq!(new.occupied.len(), 1);
        assert_ne!(old.generation, new.generation);
    }

    #[test]
    fn fragmented_trace_matches_first_fit_bitmap_oracle() {
        let mut window = ScratchWindow::new(1, PAGE_SIZE * 32);
        let mut bitmap = [false; 32];
        let mut live = Vec::new();
        let mut random = 17u32;
        for _ in 0..6000 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            if random.is_multiple_of(3) && !live.is_empty() {
                let index = random as usize % live.len();
                let (first, count) = live.remove(index);
                window.release(first * PAGE_SIZE, count * PAGE_SIZE).unwrap();
                bitmap[first..first + count].fill(false);
            } else {
                let count = random as usize % 5 + 1;
                let expected = bitmap.windows(count).position(|range| range.iter().all(|bit| !bit));
                let result = window.reserve(count * PAGE_SIZE);
                assert_eq!(
                    result,
                    expected.map(|first| first * PAGE_SIZE).ok_or(Error::OutOfSpace)
                );
                if let Some(first) = expected {
                    bitmap[first..first + count].fill(true);
                    live.push((first, count));
                }
            }
            let mut actual = [false; 32];
            for extent in &window.occupied {
                let first = extent.offset / PAGE_SIZE;
                let count = extent.bytes / PAGE_SIZE;
                assert!(actual[first..first + count].iter().all(|bit| !bit));
                actual[first..first + count].fill(true);
            }
            assert_eq!(actual, bitmap);
        }
    }
}
