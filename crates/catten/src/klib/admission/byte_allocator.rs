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

/// Captured backing sponsor, independent of reusable IDs and current workers.
/// Implementations must not allocate. Refund requires actual deallocation or
/// rejection before backing existed, as enforced by this allocator.
pub(crate) trait ByteSponsor: Clone + core::fmt::Debug {
    fn reserve(&self, bytes: u64) -> Result<(), AllocError>;
    fn release(&self, bytes: u64);
}
impl ByteSponsor for () {
    fn reserve(&self, _: u64) -> Result<(), AllocError> {
        Ok(())
    }

    fn release(&self, _: u64) {}
}

/// Linear byte funding for a separately owned fixed buffer. The containing
/// owner must retain this charge until actual deallocation; wrap it with that
/// buffer in inert storage when abandonment cannot qualify disposal.
#[derive(Debug)]
#[must_use]
pub(crate) struct ByteCharge<S: ByteSponsor> {
    sponsor: S,
    bytes: u64,
}
impl<S: ByteSponsor> ByteCharge<S> {
    pub(crate) fn try_new(sponsor: S, bytes: u64) -> Result<Self, AllocError> {
        if bytes != 0 {
            sponsor.reserve(bytes)?;
        }
        Ok(Self {
            sponsor,
            bytes,
        })
    }
}
impl<S: ByteSponsor> Drop for ByteCharge<S> {
    fn drop(&mut self) {
        if self.bytes != 0 {
            self.sponsor.release(self.bytes);
        }
    }
}

/// Requested-layout funding for backing already bounded by a separate typed
/// family owner, including the actual Arc payload and private holder layouts.
/// Clones copy policy only; actual allocation/free owns the byte reservation.
#[derive(Clone, Debug)]
pub(crate) struct SponsoredAllocator<S: ByteSponsor>(S);
impl<S: ByteSponsor> SponsoredAllocator<S> {
    pub(crate) const fn new(sponsor: S) -> Self {
        Self(sponsor)
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
        self.0.reserve(bytes)?;
        match allocate(layout) {
            Ok(block) => Ok(block),
            Err(error) => {
                self.0.release(bytes);
                Err(error)
            }
        }
    }
}
// SAFETY: The unchanged pointer/layout belongs to Global. Funding covers the
// requested size, and only actual deallocation (or failed allocation) refunds.
unsafe impl<S: ByteSponsor> Allocator for SponsoredAllocator<S> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.allocate_with(layout, |layout| {
            let block = Global.allocate(layout)?;
            Ok(NonNull::slice_from_raw_parts(block.cast(), layout.size()))
        })
    }

    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        unsafe { Global.deallocate(pointer, layout) };
        if layout.size() != 0 {
            self.0.release(layout.size() as u64);
        }
    }
}

#[derive(Debug)]
pub(crate) struct ByteAllocator<F: Family<2>, S: ByteSponsor = ()> {
    platform: bool,
    sponsor: S,
    family: PhantomData<fn() -> F>,
}
impl<F: Family<2>> ByteAllocator<F> {
    pub(crate) const fn new(platform: bool) -> Self {
        Self::with_sponsor(platform, ())
    }
}
impl<F: Family<2>, S: ByteSponsor> ByteAllocator<F, S> {
    pub(crate) const fn with_sponsor(platform: bool, sponsor: S) -> Self {
        Self {
            platform,
            sponsor,
            family: PhantomData,
        }
    }

    pub(crate) fn platform(&self) -> bool {
        self.platform
    }

    pub(crate) fn sponsor(&self) -> &S {
        &self.sponsor
    }

    fn allocate_with(
        &self,
        layout: Layout,
        allocate: impl FnOnce(Layout) -> Result<NonNull<[u8]>, AllocError>,
    ) -> Result<NonNull<[u8]>, AllocError> {
        SponsoredAllocator::new(FamilySponsor(self)).allocate_with(layout, allocate)
    }

    #[cfg(test)]
    pub(crate) fn test_allocation_rejection(
        &self,
        layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        self.allocate_with(layout, |_| Err(AllocError))
    }
}
impl<F: Family<2>, S: ByteSponsor> Clone for ByteAllocator<F, S> {
    fn clone(&self) -> Self {
        Self::with_sponsor(self.platform, self.sponsor.clone())
    }
}
impl<F: Family<2>> Default for ByteAllocator<F> {
    fn default() -> Self {
        Self::new(false)
    }
}

#[derive(Debug)]
struct FamilySponsor<'a, F: Family<2>, S: ByteSponsor>(&'a ByteAllocator<F, S>);
impl<F: Family<2>, S: ByteSponsor> Clone for FamilySponsor<'_, F, S> {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}
impl<F: Family<2>, S: ByteSponsor> ByteSponsor for FamilySponsor<'_, F, S> {
    fn reserve(&self, bytes: u64) -> Result<(), AllocError> {
        self.0.sponsor.reserve(bytes)?;
        if let Err(error) = reserve::<F, 2>(self.0.platform, [bytes, 0]) {
            self.0.sponsor.release(bytes);
            return Err(error);
        }
        Ok(())
    }

    fn release(&self, bytes: u64) {
        release::<F, 2>(self.0.platform, [bytes, 0]);
        self.0.sponsor.release(bytes);
    }
}
// SAFETY: The shared backing adapter passes the unchanged Global pointer/layout.
// Its borrowed policy exists only during this allocation/free call.
unsafe impl<F: Family<2>, S: ByteSponsor> Allocator for ByteAllocator<F, S> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.allocate_with(layout, |layout| {
            let block = Global.allocate(layout)?;
            Ok(NonNull::slice_from_raw_parts(block.cast(), layout.size()))
        })
    }

    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        unsafe { SponsoredAllocator::new(FamilySponsor(self)).deallocate(pointer, layout) };
    }
    // Default grow funds the whole candidate while retaining the old capacity.
}
