//! Virtual scratch extents use prepared nodes. Reserve/release only relink;
//! explicit disposal runs after the caller leaves its namespace guard.
#![cfg_attr(test, feature(allocator_api))]

#[cfg(not(test))]
use crate::klib::collections::retirement_list;
// This same file remains a standalone host test crate. Reuse production nodes;
// the collection's crate-root feature attribute is redundant in this module.
#[cfg(test)]
#[allow(unused_attributes)]
#[path = "../../klib/collections/retirement_list.rs"]
mod retirement_list;
use retirement_list::{
    AdmittedMap,
    PreparedEntry,
    RetiredEntry,
};

const PAGE_SIZE: usize = 4096;
pub(crate) type PreparedExtent<C = ()> = PreparedEntry<(usize, usize), C>;
pub(crate) type RetiredExtent<C = ()> = RetiredEntry<(usize, usize), C>;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidRange,
    OutOfSpace,
    AllocationFailed,
}

#[derive(Debug)]
pub(crate) struct ScratchWindow<C = ()> {
    bytes: usize,
    /// Sorted, disjoint live reservations. Gaps are implicit; neither first
    /// fit nor exact removal allocates or needs a coalescing node.
    occupied: AdmittedMap<usize, usize, C>,
}

impl<C> ScratchWindow<C> {
    pub(crate) const fn new(bytes: usize) -> Self {
        Self {
            bytes,
            occupied: AdmittedMap::new(),
        }
    }

    pub(crate) fn first_fit(&self, bytes: usize) -> Result<usize, Error> {
        if bytes == 0 || !bytes.is_multiple_of(PAGE_SIZE) {
            return Err(Error::InvalidRange);
        }
        let mut offset = 0usize;
        for (&first, &length) in self.occupied.iter() {
            if first - offset >= bytes {
                break;
            }
            offset = first + length;
        }
        if offset.checked_add(bytes).is_none_or(|end| end > self.bytes) {
            return Err(Error::OutOfSpace);
        }
        Ok(offset)
    }

    pub(crate) fn reserve(
        &mut self,
        bytes: usize,
        storage: &mut Option<PreparedExtent<C>>,
    ) -> Result<usize, Error> {
        let offset = self.first_fit(bytes)?;
        let node = storage.take().ok_or(Error::AllocationFailed)?;
        self.occupied.insert(node, offset, bytes);
        Ok(offset)
    }

    /// Detach only. Callers retain the original node until post-guard disposal.
    pub(crate) fn release(
        &mut self,
        offset: usize,
        bytes: usize,
    ) -> Result<RetiredExtent<C>, Error> {
        if self.occupied.get(&offset) != Some(&bytes) {
            return Err(Error::InvalidRange);
        }
        Ok(self.occupied.take(&offset).expect("validated scratch extent missing"))
    }

    pub(crate) fn extent_count(&self) -> usize {
        self.occupied.iter().count()
    }

    /// Whole namespace disposal requires its containing root's completion proof.
    pub(crate) fn release_all(&mut self) {
        while let Some((&offset, _)) = self.occupied.first_key_value() {
            self.occupied.take(&offset).unwrap().release();
        }
    }
}

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    fn reserve(window: &mut ScratchWindow, bytes: usize) -> Result<usize, Error> {
        let mut storage = Some(PreparedExtent::try_new().unwrap());
        window.reserve(bytes, &mut storage)
    }
    fn release(window: &mut ScratchWindow, offset: usize, bytes: usize) -> Result<(), Error> {
        window.release(offset, bytes)?.release();
        Ok(())
    }
    fn snapshot(window: &ScratchWindow) -> Vec<(usize, usize)> {
        window.occupied.iter().map(|(&offset, &bytes)| (offset, bytes)).collect()
    }

    #[test]
    fn first_fit_reuses_and_combines_implicit_gaps_without_release_allocation() {
        let mut window = ScratchWindow::new(PAGE_SIZE * 8);
        let a = reserve(&mut window, PAGE_SIZE * 2).unwrap();
        let b = reserve(&mut window, PAGE_SIZE).unwrap();
        let c = reserve(&mut window, PAGE_SIZE * 2).unwrap();
        release(&mut window, b, PAGE_SIZE).unwrap();
        release(&mut window, a, PAGE_SIZE * 2).unwrap();
        assert_eq!(reserve(&mut window, PAGE_SIZE * 3), Ok(0));
        assert_eq!(reserve(&mut window, PAGE_SIZE), Ok(PAGE_SIZE * 5));
        release(&mut window, c, PAGE_SIZE * 2).unwrap();
        assert_eq!(reserve(&mut window, PAGE_SIZE * 2), Ok(PAGE_SIZE * 3));
        window.release_all();
    }

    #[test]
    fn release_requires_one_exact_live_extent_and_leaves_rejections_unchanged() {
        let mut window = ScratchWindow::new(PAGE_SIZE * 8);
        reserve(&mut window, PAGE_SIZE * 2).unwrap();
        reserve(&mut window, PAGE_SIZE).unwrap();
        let expected = snapshot(&window);
        for (offset, bytes) in [
            (0, 0),
            (0, PAGE_SIZE),
            (PAGE_SIZE, PAGE_SIZE),
            (0, PAGE_SIZE * 3),
            (PAGE_SIZE * 8, PAGE_SIZE),
            (usize::MAX, PAGE_SIZE),
        ] {
            assert_eq!(release(&mut window, offset, bytes), Err(Error::InvalidRange));
            assert_eq!(snapshot(&window), expected);
        }
        release(&mut window, 0, PAGE_SIZE * 2).unwrap();
        assert_eq!(release(&mut window, 0, PAGE_SIZE * 2), Err(Error::InvalidRange));
        assert_eq!(window.extent_count(), 1);
        assert_eq!(reserve(&mut window, PAGE_SIZE * 2), Ok(0));
        window.release_all();
    }

    #[test]
    fn missing_preparation_does_not_publish_or_consume_virtual_space() {
        let mut window = ScratchWindow::new(PAGE_SIZE * 4);
        assert_eq!(window.reserve(PAGE_SIZE, &mut None), Err(Error::AllocationFailed));
        assert!(window.occupied.first_key_value().is_none());
        assert_eq!(reserve(&mut window, PAGE_SIZE), Ok(0));
        let expected = snapshot(&window);
        assert_eq!(window.reserve(PAGE_SIZE, &mut None), Err(Error::AllocationFailed));
        assert_eq!(snapshot(&window), expected);
        assert_eq!(reserve(&mut window, PAGE_SIZE), Ok(PAGE_SIZE));
        window.release_all();
    }

    #[test]
    fn invalid_exhausted_and_overflow_requests_reject_before_storage_consumption() {
        let mut window = ScratchWindow::new(PAGE_SIZE);
        for bytes in [0, 1, PAGE_SIZE - 1] {
            assert_eq!(window.reserve(bytes, &mut None), Err(Error::InvalidRange));
        }
        assert_eq!(reserve(&mut window, PAGE_SIZE), Ok(0));
        assert_eq!(window.reserve(PAGE_SIZE, &mut None), Err(Error::OutOfSpace));
        let largest = usize::MAX - (PAGE_SIZE - 1);
        let mut large = ScratchWindow::new(largest);
        assert_eq!(reserve(&mut large, largest), Ok(0));
        assert_eq!(reserve(&mut large, PAGE_SIZE), Err(Error::OutOfSpace));
        release(&mut large, 0, largest).unwrap();
        assert_eq!(reserve(&mut large, PAGE_SIZE), Ok(0));
        window.release_all();
        large.release_all();
    }

    #[test]
    fn independent_lifetimes_keep_independent_windows() {
        let mut old = ScratchWindow::new(PAGE_SIZE * 2);
        let mut new = ScratchWindow::new(PAGE_SIZE * 2);
        assert_eq!(reserve(&mut old, PAGE_SIZE), Ok(0));
        assert_eq!(reserve(&mut new, PAGE_SIZE), Ok(0));
        release(&mut old, 0, PAGE_SIZE).unwrap();
        assert_eq!(new.extent_count(), 1);
        new.release_all();
    }

    #[test]
    fn fragmented_trace_matches_first_fit_bitmap_oracle() {
        let mut window = ScratchWindow::new(PAGE_SIZE * 32);
        let mut bitmap = [false; 32];
        let mut live = Vec::new();
        let mut random = 17u32;
        for _ in 0..6000 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            if random.is_multiple_of(3) && !live.is_empty() {
                let index = random as usize % live.len();
                let (first, count) = live.remove(index);
                release(&mut window, first * PAGE_SIZE, count * PAGE_SIZE).unwrap();
                bitmap[first..first + count].fill(false);
            } else {
                let count = random as usize % 5 + 1;
                let expected = bitmap.windows(count).position(|range| range.iter().all(|bit| !bit));
                assert_eq!(
                    reserve(&mut window, count * PAGE_SIZE),
                    expected.map(|first| first * PAGE_SIZE).ok_or(Error::OutOfSpace)
                );
                if let Some(first) = expected {
                    bitmap[first..first + count].fill(true);
                    live.push((first, count));
                }
            }
            let mut actual = [false; 32];
            for (&offset, &bytes) in window.occupied.iter() {
                let first = offset / PAGE_SIZE;
                let count = bytes / PAGE_SIZE;
                assert!(actual[first..first + count].iter().all(|bit| !bit));
                actual[first..first + count].fill(true);
            }
            assert_eq!(actual, bitmap);
        }
        window.release_all();
    }
}
