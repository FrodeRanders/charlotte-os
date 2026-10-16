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
    mem::{
        ManuallyDrop,
        MaybeUninit,
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

/// Keep the original charge outside empty holder storage until initialization.
/// Abandonment retains both fields without refund, allocation or locks.
#[must_use]
struct PreparingAllocation<C>(ManuallyDrop<AllocationResources<C>>);

struct AllocationResources<C> {
    charge: Option<C>,
    storage: Option<Arc<MaybeUninit<Allocation<C>>>>,
}

impl<C> PreparingAllocation<C> {
    fn new(charge: C) -> Self {
        Self(ManuallyDrop::new(AllocationResources {
            charge: Some(charge),
            storage: None,
        }))
    }

    fn allocate(
        &mut self,
        allocate: impl FnOnce() -> Result<Arc<MaybeUninit<Allocation<C>>>, AllocError>,
    ) -> Result<(), AllocError> {
        assert!(self.0.storage.is_none(), "charge holder storage already allocated");
        self.0.storage = Some(allocate()?);
        Ok(())
    }

    fn complete(mut self) -> ChargedAllocator<C> {
        let resources = &mut *self.0;
        let slot = Arc::get_mut(resources.storage.as_mut().expect("charge holder needs storage"))
            .expect("private charge holder storage was aliased");
        slot.write(Allocation {
            charge: resources.charge.take().unwrap(),
            started: AtomicBool::new(false),
        });
        let storage = resources.storage.take().unwrap();
        // SAFETY: This unique, unexposed Arc has just been initialized exactly
        // once above. No Weak or bare holder Arc escapes the allocator adapter.
        let charge = unsafe { storage.assume_init() };
        ChargedAllocator {
            charge: Some(charge),
        }
    }

    fn cancel(self) {
        self.cancel_with(|| {});
    }

    fn cancel_with(self, mut before_dispose: impl FnMut()) {
        let AllocationResources {
            charge,
            storage,
        } = ManuallyDrop::into_inner(self.0);
        before_dispose();
        drop(storage);
        before_dispose();
        drop(charge);
    }
}

impl<C> ChargedAllocator<C> {
    pub(crate) fn try_new(charge: C) -> Result<Self, AllocError> {
        Self::try_new_with(charge, Arc::<Allocation<C>>::try_new_uninit)
    }

    /// Borrow a containing transaction's reservation until holder allocation
    /// succeeds. Failure leaves that exact charge with the parent for ordered
    /// payload disposal; no original account is reconstructed or refunded here.
    pub(crate) fn try_new_retaining(charge: &mut Option<C>) -> Result<Self, AllocError> {
        Self::try_new_retaining_with(charge, Arc::<Allocation<C>>::try_new_uninit)
    }

    fn try_new_retaining_with(
        charge: &mut Option<C>,
        allocate: impl FnOnce() -> Result<Arc<MaybeUninit<Allocation<C>>>, AllocError>,
    ) -> Result<Self, AllocError> {
        assert!(charge.is_some(), "missing parent admission");
        let mut preparing = PreparingAllocation(ManuallyDrop::new(AllocationResources {
            charge: None,
            storage: None,
        }));
        if let Err(error) = preparing.allocate(allocate) {
            preparing.cancel();
            return Err(error);
        }
        preparing.0.charge = charge.take();
        Ok(preparing.complete())
    }

    fn try_new_with(
        charge: C,
        allocate: impl FnOnce() -> Result<Arc<MaybeUninit<Allocation<C>>>, AllocError>,
    ) -> Result<Self, AllocError> {
        let mut preparing = PreparingAllocation::new(charge);
        if let Err(error) = preparing.allocate(allocate) {
            preparing.cancel();
            return Err(error);
        }
        Ok(preparing.complete())
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
