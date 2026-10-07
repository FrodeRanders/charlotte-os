//! Admission attached to allocation lifetime, including weak-only Arc backing.
//! All clones of the private charge holder consume it through `into_inner`;
//! the last one frees that holder before refunding its original reservation.

use alloc::{
    alloc::Global,
    sync::Arc,
};
use core::{
    alloc::{
        AllocError,
        Allocator,
        Layout,
    },
    ptr::NonNull,
    sync::atomic::{
        AtomicBool,
        Ordering,
    },
};

#[derive(Debug)]
struct Allocation<C> {
    charge: C,
    started: AtomicBool,
}

#[derive(Debug)]
pub(crate) struct ChargedAllocator<C> {
    charge: Option<Arc<Allocation<C>>>,
}

impl<C> ChargedAllocator<C> {
    pub(crate) fn try_new(charge: C) -> Result<Self, AllocError> {
        Self::try_new_with(charge, Arc::try_new)
    }

    fn try_new_with(
        charge: C,
        allocate: impl FnOnce(Allocation<C>) -> Result<Arc<Allocation<C>>, AllocError>,
    ) -> Result<Self, AllocError> {
        Ok(Self {
            charge: Some(allocate(Allocation {
                charge,
                started: AtomicBool::new(false),
            })?),
        })
    }

    pub(crate) fn try_arc<T>(self, value: T) -> Result<Arc<T, Self>, AllocError> {
        self.try_arc_with(value, Arc::try_new_in)
    }

    fn try_arc_with<T>(
        self,
        value: T,
        allocate: impl FnOnce(T, Self) -> Result<Arc<T, Self>, AllocError>,
    ) -> Result<Arc<T, Self>, AllocError> {
        // Arc allocation failure can drop its allocator before the payload.
        // Retain this preparation owner until that whole attempt has ended.
        allocate(value, self.clone())
    }
}

impl<C> Clone for ChargedAllocator<C> {
    fn clone(&self) -> Self {
        Self {
            charge: self.charge.clone(),
        }
    }
}

impl<C> Drop for ChargedAllocator<C> {
    fn drop(&mut self) {
        // No Weak or bare Arc to the charge holder escapes this adapter. Every
        // clone uses into_inner, including concurrent final allocator drops.
        // This returns exactly one reservation *after* holder deallocation.
        if let Some(allocation) = self.charge.take().and_then(Arc::into_inner) {
            drop(allocation.charge);
        }
    }
}

// SAFETY: Every allocation/deallocation uses Global with the caller's unchanged
// pointer/layout. Clones share the same Global allocation domain; none refund
// admission during allocation/deallocation or expose the private charge holder.
unsafe impl<C> Allocator for ChargedAllocator<C> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let allocation = self.charge.as_ref().unwrap();
        // One reservation permits one allocation, not one allocation per
        // clone. Even a freed allocation cannot be revived with its old charge.
        if allocation.started.swap(true, Ordering::AcqRel) {
            return Err(AllocError);
        }
        match Global.allocate(layout) {
            Ok(backing) => Ok(backing),
            Err(error) => {
                allocation.started.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { Global.deallocate(ptr, layout) };
    }
}

#[cfg(test)]
#[path = "charged_allocator/tests.rs"]
mod tests;
