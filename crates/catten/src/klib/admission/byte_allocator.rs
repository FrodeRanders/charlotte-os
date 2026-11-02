//! Bytes follow the requested allocation, including complete old/candidate
//! overlap during allocator-default growth. The containing Vec owns cleanup.
use alloc::alloc::Global;
use core::{
    alloc::{
        AllocError,
        Allocator,
        Layout,
    },
    marker::PhantomData,
    ptr::NonNull,
};

use super::{
    Family,
    release,
    reserve,
};

#[derive(Debug)]
pub(crate) struct ByteAllocator<F: Family<2>> {
    platform: bool,
    family: PhantomData<fn() -> F>,
}
impl<F: Family<2>> ByteAllocator<F> {
    pub(crate) const fn new(platform: bool) -> Self {
        Self {
            platform,
            family: PhantomData,
        }
    }

    pub(crate) fn platform(&self) -> bool {
        self.platform
    }

    fn allocate_with(
        &self,
        layout: Layout,
        allocate: impl FnOnce(Layout) -> Result<NonNull<[u8]>, AllocError>,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let bytes = u64::try_from(layout.size()).map_err(|_| AllocError)?;
        if bytes == 0 {
            return allocate(layout);
        }
        reserve::<F, 2>(self.platform, [bytes, 0])?;
        match allocate(layout) {
            Ok(block) => Ok(block),
            Err(error) => {
                release::<F, 2>(self.platform, [bytes, 0]);
                Err(error)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn test_allocation_rejection(
        &self,
        layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        self.allocate_with(layout, |_| Err(AllocError))
    }
}
impl<F: Family<2>> Clone for ByteAllocator<F> {
    fn clone(&self) -> Self {
        Self::new(self.platform)
    }
}
impl<F: Family<2>> Default for ByteAllocator<F> {
    fn default() -> Self {
        Self::new(false)
    }
}

// SAFETY: Global receives the unchanged pointer/layout. Returned blocks report
// the requested size, so a fitting deallocation layout has that same byte size.
// Captured class moves/clones unchanged; no allocator Drop refunds backing.
unsafe impl<F: Family<2>> Allocator for ByteAllocator<F> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.allocate_with(layout, |layout| {
            let block = Global.allocate(layout)?;
            Ok(NonNull::slice_from_raw_parts(block.cast(), layout.size()))
        })
    }

    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        unsafe { Global.deallocate(pointer, layout) };
        if layout.size() != 0 {
            release::<F, 2>(self.platform, [layout.size() as u64, 0]);
        }
    }
    // Allocator's default grow allocates the full candidate before copying and
    // deallocating the old buffer. Never use incremental-only realloc admission.
}
