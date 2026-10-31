//! Inline shutdown-plan storage. Abandonment retains every payload without
//! invoking its destructor beneath an unknown caller guard. Explicit removal
//! transfers the one owner after its phase-specific completion proof.

use core::{
    mem::ManuallyDrop,
    ops::{
        Index,
        IndexMut,
    },
};

pub(crate) struct RetainedArray<T, const N: usize> {
    entries: ManuallyDrop<[Option<T>; N]>,
    len: usize,
}

impl<T, const N: usize> Drop for RetainedArray<T, N> {
    fn drop(&mut self) {
        // ManuallyDrop retains all initialized payloads, including nested
        // pending close owners. Abandonment never acts on their authority.
    }
}

impl<T, const N: usize> RetainedArray<T, N> {
    pub(crate) fn from_options(entries: [Option<T>; N]) -> Self {
        let mut result = Self::empty();
        for entry in entries.into_iter().flatten() {
            result.entries[result.len] = Some(entry);
            result.len += 1;
        }
        result
    }

    const fn empty() -> Self {
        Self {
            entries: ManuallyDrop::new([const { None }; N]),
            len: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn first_mut(&mut self) -> Option<&mut T> {
        self.entries[0..self.len].first_mut().and_then(Option::as_mut)
    }

    pub(crate) fn remove(&mut self, index: usize) -> T {
        assert!(index < self.len);
        let value = self.entries[index].take().unwrap();
        for next in index + 1..self.len {
            self.entries[next - 1] = self.entries[next].take();
        }
        self.len -= 1;
        value
    }

    pub(crate) fn swap_remove(&mut self, index: usize) -> T {
        assert!(index < self.len);
        let value = self.entries[index].take().unwrap();
        self.len -= 1;
        if index != self.len {
            self.entries[index] = self.entries[self.len].take();
        }
        value
    }
}

impl<T, const N: usize, const M: usize> From<[T; M]> for RetainedArray<T, N> {
    fn from(entries: [T; M]) -> Self {
        // Shutdown's phase/device maxima are structural, checked at compile
        // time for fixture arrays rather than discovered after taking owners.
        const { assert!(M <= N) };
        let mut result = Self::empty();
        for entry in entries {
            result.entries[result.len] = Some(entry);
            result.len += 1;
        }
        result
    }
}

impl<T, const N: usize> Default for RetainedArray<T, N> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T, const N: usize> Index<usize> for RetainedArray<T, N> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        assert!(index < self.len);
        self.entries[index].as_ref().unwrap()
    }
}

impl<T, const N: usize> IndexMut<usize> for RetainedArray<T, N> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        assert!(index < self.len);
        self.entries[index].as_mut().unwrap()
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
