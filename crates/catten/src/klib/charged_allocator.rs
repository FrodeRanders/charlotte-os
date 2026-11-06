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
    marker::PhantomData,
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
struct Allocation<C, P = PreAdmitted> {
    charge: C,
    started: AtomicBool,
    policy: PhantomData<fn() -> P>,
}

/// Optional admission of the allocator's actual payload layout, before Global
/// allocation. Rejection changes no charge; rollback undoes an accepted layout
/// only when Global failed and therefore created no backing. Successful backing
/// keeps admission with C through final weak/allocator release.
pub(crate) trait PayloadAdmission<C>: Send + Sync {
    fn reserve(_charge: &C, _layout: Layout) -> Result<(), AllocError> {
        Ok(())
    }
    fn rollback(_charge: &C, _layout: Layout) {}
}

/// Existing families already admit their payload before constructing this
/// allocator. The zero-sized default preserves their representation and policy.
#[derive(Debug)]
pub(crate) struct PreAdmitted;
impl<C> PayloadAdmission<C> for PreAdmitted {}

#[derive(Debug)]
pub(crate) struct ChargedAllocator<C, P = PreAdmitted, A: Allocator + Clone = Global> {
    charge: Option<Arc<Allocation<C, P>, A>>,
}

/// Keep the original charge outside empty holder storage until initialization.
/// Abandonment retains both fields without refund, allocation or locks.
#[must_use]
struct PreparingAllocation<C, P = PreAdmitted, A: Allocator + Clone = Global>(
    ManuallyDrop<AllocationResources<C, P, A>>,
);

struct AllocationResources<C, P = PreAdmitted, A: Allocator + Clone = Global> {
    charge: Option<C>,
    storage: Option<Arc<MaybeUninit<Allocation<C, P>>, A>>,
}

/// Retain the complete payload and original admission across empty allocation.
/// Abandonment performs no field destruction, allocator work or refund.
#[must_use]
struct PreparingArc<T, C, P: PayloadAdmission<C> = PreAdmitted, A: Allocator + Clone = Global>(
    ManuallyDrop<ArcResources<T, C, P, A>>,
);

struct ArcResources<T, C, P: PayloadAdmission<C> = PreAdmitted, A: Allocator + Clone = Global> {
    value: Option<T>,
    storage: Option<Arc<MaybeUninit<T>, ChargedAllocator<C, P, A>>>,
    allocator: ChargedAllocator<C, P, A>,
}

impl<T, C, P: PayloadAdmission<C>, A: Allocator + Clone> PreparingArc<T, C, P, A> {
    fn new(value: T, allocator: ChargedAllocator<C, P, A>) -> Self {
        Self(ManuallyDrop::new(ArcResources {
            value: Some(value),
            storage: None,
            allocator,
        }))
    }

    fn allocate(
        &mut self,
        allocate: impl FnOnce(
            ChargedAllocator<C, P, A>,
        )
            -> Result<Arc<MaybeUninit<T>, ChargedAllocator<C, P, A>>, AllocError>,
    ) -> Result<(), AllocError> {
        assert!(self.0.storage.is_none(), "payload storage already allocated");
        self.0.storage = Some(allocate(self.0.allocator.clone())?);
        Ok(())
    }

    fn complete(mut self) -> Arc<T, ChargedAllocator<C, P, A>> {
        let resources = &mut *self.0;
        let slot = Arc::get_mut(resources.storage.as_mut().expect("payload needs storage"))
            .expect("private payload storage was aliased");
        slot.write(resources.value.take().unwrap());
        let storage = resources.storage.take().unwrap();
        // SAFETY: This unique, unexposed Arc was initialized exactly once above.
        // No strong or Weak alias can observe the uninitialized payload.
        let value = unsafe { storage.assume_init() };
        // The published allocation retains the same admission. Release only
        // the preparation's extra allocator reference, never its payload.
        let resources = ManuallyDrop::into_inner(self.0);
        drop(resources);
        value
    }

    fn cancel(self) {
        self.cancel_with(|| {});
    }

    fn cancel_with(self, mut before_dispose: impl FnMut()) {
        let ArcResources {
            value,
            storage,
            allocator,
        } = ManuallyDrop::into_inner(self.0);
        before_dispose();
        drop(value);
        before_dispose();
        drop(storage);
        before_dispose();
        drop(allocator);
    }
}

impl<C, P, A: Allocator + Clone> PreparingAllocation<C, P, A> {
    fn new(charge: C) -> Self {
        Self(ManuallyDrop::new(AllocationResources {
            charge: Some(charge),
            storage: None,
        }))
    }

    fn allocate(
        &mut self,
        allocate: impl FnOnce() -> Result<Arc<MaybeUninit<Allocation<C, P>>, A>, AllocError>,
    ) -> Result<(), AllocError> {
        assert!(self.0.storage.is_none(), "charge holder storage already allocated");
        self.0.storage = Some(allocate()?);
        Ok(())
    }

    fn complete(mut self) -> ChargedAllocator<C, P, A> {
        let resources = &mut *self.0;
        let slot = Arc::get_mut(resources.storage.as_mut().expect("charge holder needs storage"))
            .expect("private charge holder storage was aliased");
        slot.write(Allocation {
            charge: resources.charge.take().unwrap(),
            started: AtomicBool::new(false),
            policy: PhantomData,
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
}

impl<C, P: PayloadAdmission<C>> ChargedAllocator<C, P> {
    pub(crate) fn try_new_with_policy(charge: C) -> Result<Self, AllocError> {
        Self::try_new_with_backing(charge, Global)
    }
}

impl<C, P: PayloadAdmission<C>, A: Allocator + Clone> ChargedAllocator<C, P, A> {
    /// Borrow original admission without exposing its private Arc/Weak holder.
    pub(crate) fn charge(&self) -> &C {
        &self.charge.as_ref().unwrap().charge
    }

    pub(crate) fn try_new_with_backing(charge: C, backing: A) -> Result<Self, AllocError> {
        let mut preparing = PreparingAllocation::<C, P, A>::new(charge);
        if let Err(error) =
            preparing.allocate(|| Arc::<Allocation<C, P>, A>::try_new_uninit_in(backing))
        {
            preparing.cancel();
            return Err(error);
        }
        Ok(preparing.complete())
    }

    fn allocate_payload(
        &self,
        layout: Layout,
        allocate: impl FnOnce(Layout) -> Result<NonNull<[u8]>, AllocError>,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let allocation = self.charge.as_ref().unwrap();
        if allocation.started.swap(true, Ordering::AcqRel) {
            return Err(AllocError);
        }
        if let Err(error) = P::reserve(&allocation.charge, layout) {
            allocation.started.store(false, Ordering::Release);
            return Err(error);
        }
        match allocate(layout) {
            Ok(backing) => Ok(backing),
            Err(error) => {
                P::rollback(&allocation.charge, layout);
                allocation.started.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    pub(crate) fn try_arc<T>(self, value: T) -> Result<Arc<T, Self>, AllocError> {
        self.try_arc_with(value, Arc::<T, Self>::try_new_uninit_in)
    }

    fn try_arc_with<T>(
        self,
        value: T,
        allocate: impl FnOnce(Self) -> Result<Arc<MaybeUninit<T>, Self>, AllocError>,
    ) -> Result<Arc<T, Self>, AllocError> {
        let mut preparing = PreparingArc::new(value, self);
        if let Err(error) = preparing.allocate(allocate) {
            preparing.cancel();
            return Err(error);
        }
        Ok(preparing.complete())
    }
}

impl<C, P, A: Allocator + Clone> Clone for ChargedAllocator<C, P, A> {
    fn clone(&self) -> Self {
        Self {
            charge: self.charge.clone(),
        }
    }
}

impl<C, P, A: Allocator + Clone> Drop for ChargedAllocator<C, P, A> {
    fn drop(&mut self) {
        // No Weak or bare Arc to the charge holder escapes this adapter. Every
        // clone uses into_inner, including concurrent final allocator drops.
        // This returns exactly one reservation *after* holder deallocation.
        if let Some(allocation) = self.charge.take().and_then(Arc::into_inner) {
            drop(allocation.charge);
        }
    }
}

// SAFETY: Payload and holder allocations use the captured backing allocator A.
// Clones share its allocation domain and pass unchanged pointer/layout on free.
// A may fund actual backing bytes; C remains held through final holder release.
// No bare Arc/Weak to the private charge holder escapes this adapter.
unsafe impl<C, P: PayloadAdmission<C>, A: Allocator + Clone> Allocator
    for ChargedAllocator<C, P, A>
{
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // All clones share permission for a single allocation. Freeing that
        // allocation does not let a clone reuse its old charge.
        self.allocate_payload(layout, |layout| {
            Arc::allocator(self.charge.as_ref().unwrap()).allocate(layout)
        })
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { Arc::allocator(self.charge.as_ref().unwrap()).deallocate(ptr, layout) };
    }
}

#[cfg(test)]
#[path = "charged_allocator/tests.rs"]
mod tests;
