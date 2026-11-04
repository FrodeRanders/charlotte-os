//! Ordinary local byte storage: erase the full capacity before allocator disposal.
use alloc::vec::Vec;
use core::{
    alloc::Allocator,
    ops::{
        Deref,
        DerefMut,
    },
};

use zeroize::Zeroize;

pub(crate) struct SensitiveBuffer<A: Allocator>(Vec<u8, A>);

impl<A: Allocator> SensitiveBuffer<A> {
    pub(crate) fn new_in(allocator: A) -> Self {
        Self(Vec::new_in(allocator))
    }

    /// Prepare once, before secret material is written. Reallocation of an
    /// initialized secret could leave an unwiped copy in the old allocation.
    pub(crate) fn try_initialize_zeroed(
        &mut self,
        len: usize,
    ) -> Result<(), core::alloc::AllocError> {
        if self.0.capacity() != 0 {
            return Err(core::alloc::AllocError);
        }
        self.0.try_reserve_exact(len).map_err(|_| core::alloc::AllocError)?;
        self.0.resize(len, 0);
        Ok(())
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.0.capacity()
    }
}

impl<A: Allocator> Deref for SensitiveBuffer<A> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl<A: Allocator> DerefMut for SensitiveBuffer<A> {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}
impl<A: Allocator> Drop for SensitiveBuffer<A> {
    fn drop(&mut self) {
        self.0.as_mut_slice().zeroize();
        self.0.spare_capacity_mut().zeroize();
        // Vec subsequently deallocates through its original allocator. No
        // refund or release can precede erasure, including truncated contents.
    }
}

#[cfg(test)]
#[path = "sensitive_buffer/tests.rs"]
mod tests;
