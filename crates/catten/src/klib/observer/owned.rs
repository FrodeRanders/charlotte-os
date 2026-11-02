//! Strong/weak observer handles keep allocation admission behind the interface.
//! Type erasure consumes the original handle without allocation or pointer adoption.
use alloc::sync::{
    Arc,
    Weak,
};
use core::{
    alloc::AllocError,
    fmt,
    ops::Deref,
};

use super::backing;

/// Dispatchers retain a strong handle while borrowing a notification target.
/// A notification is a hint; it conveys no completion or cleanup authority.
pub trait Observer: Send + Sync {
    fn notify(&self);
}

pub struct ObserverRef<T: ?Sized = dyn Observer>(Arc<T, backing::Allocator>);
pub struct ObserverWeak<T: ?Sized = dyn Observer>(Option<Weak<T, backing::Allocator>>);

impl<T: Observer + 'static> ObserverRef<T> {
    pub(crate) fn try_new(value: T, platform: bool) -> Result<Self, AllocError> {
        backing::allocator(platform)?.try_arc(value).map(Self)
    }

    pub fn erase(self) -> ObserverRef {
        ObserverRef(self.0)
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn new_for_test(value: T) -> Self {
        Self::try_new(value, false).expect("mandatory fixture observer allocation failed")
    }
}
impl<T: ?Sized> ObserverRef<T> {
    pub fn downgrade(&self) -> ObserverWeak<T> {
        ObserverWeak(Some(Arc::downgrade(&self.0)))
    }

    pub fn ptr_eq(left: &Self, right: &Self) -> bool {
        Arc::ptr_eq(&left.0, &right.0)
    }

    pub fn strong_count(this: &Self) -> usize {
        Arc::strong_count(&this.0)
    }

    pub fn weak_count(this: &Self) -> usize {
        Arc::weak_count(&this.0)
    }
}
impl<T: ?Sized> Clone for ObserverRef<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: ?Sized> Deref for ObserverRef<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: ?Sized> AsRef<T> for ObserverRef<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}
impl<T: ?Sized + fmt::Debug> fmt::Debug for ObserverRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}
impl<T: Observer + 'static> ObserverWeak<T> {
    pub fn erase(self) -> ObserverWeak {
        ObserverWeak(self.0.map(|weak| -> Weak<dyn Observer, backing::Allocator> { weak }))
    }
}
impl<T: ?Sized> ObserverWeak<T> {
    /// Empty interest owns no allocation or admission.
    pub const fn new() -> Self {
        Self(None)
    }

    pub fn upgrade(&self) -> Option<ObserverRef<T>> {
        self.0.as_ref().and_then(Weak::upgrade).map(ObserverRef)
    }

    pub fn strong_count(&self) -> usize {
        self.0.as_ref().map_or(0, Weak::strong_count)
    }

    pub fn weak_count(&self) -> usize {
        self.0.as_ref().map_or(0, Weak::weak_count)
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(left), Some(right)) => Weak::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }
}
impl<T: ?Sized> Default for ObserverWeak<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: ?Sized> Clone for ObserverWeak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: ?Sized> fmt::Debug for ObserverWeak<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObserverWeak").finish_non_exhaustive()
    }
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_observer<T: Observer + 'static>(value: T) -> ObserverRef {
    ObserverRef::new_for_test(value).erase()
}

#[cfg(test)]
#[path = "owned/tests.rs"]
mod tests;
