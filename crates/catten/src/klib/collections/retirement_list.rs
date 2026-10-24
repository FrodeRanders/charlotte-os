//! Preallocated retirement nodes. Linking, detaching and filtering never grow
//! storage. Abandoned published nodes/lists quarantine their payloads.

#![cfg_attr(test, feature(allocator_api))]

use alloc::boxed::Box;
use core::{
    alloc::AllocError,
    fmt,
    mem::{
        ManuallyDrop,
        MaybeUninit,
    },
    ops::Index,
};

struct Node<T, C = ()> {
    value: Option<T>,
    charge: MaybeUninit<C>,
    next: Option<Box<Node<T, C>>>,
}

/// Empty storage allocated before its payload becomes externally reachable.
#[must_use]
pub(crate) struct PreparedEntry<T, C = ()>(Option<Box<Node<T, C>>>);

impl<T, C> fmt::Debug for PreparedEntry<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedEntry").finish_non_exhaustive()
    }
}

impl<T> PreparedEntry<T> {
    pub(crate) fn try_new() -> Result<Self, AllocError> {
        Self::try_new_with(Box::try_new)
    }

    fn try_new_with(
        allocate: impl FnOnce(Node<T>) -> Result<Box<Node<T>>, AllocError>,
    ) -> Result<Self, AllocError> {
        Self::try_new_charged_with((), allocate)
    }
}

impl<T, C> PreparedEntry<T, Option<C>> {
    /// Bind admission to already-prepared storage before payload publication.
    /// The original charge then follows node deallocation, including rejection.
    pub(crate) fn bind_charge(&mut self, charge: C) {
        // SAFETY: every prepared node initializes its charge before exposure.
        let slot = unsafe { self.0.as_mut().unwrap().charge.assume_init_mut() };
        assert!(slot.is_none(), "prepared node admission rebound");
        *slot = Some(charge);
    }
}

impl<T, C> PreparedEntry<T, C> {
    /// Exact requested node bytes, including the custody field. Allocator
    /// bookkeeping and independent registry capacity are separate admission.
    pub(crate) const fn allocation_bytes() -> usize {
        core::mem::size_of::<Node<T, C>>()
    }

    pub(crate) fn try_new_charged(charge: C) -> Result<Self, AllocError> {
        Self::try_new_charged_with(charge, Box::try_new)
    }

    fn try_new_charged_with(
        charge: C,
        allocate: impl FnOnce(Node<T, C>) -> Result<Box<Node<T, C>>, AllocError>,
    ) -> Result<Self, AllocError> {
        // Keep admission outside the value passed to the fallible allocator.
        // A rejected allocation drops its still-unstarted reservation normally.
        let mut node = allocate(Node {
            value: None,
            charge: MaybeUninit::uninit(),
            next: None,
        })?;
        node.charge.write(charge);
        Ok(Self(Some(node)))
    }

    pub(crate) fn publish(mut self, value: T) -> RetiredEntry<T, C> {
        let mut node = self.0.take().unwrap();
        node.value = Some(value);
        RetiredEntry(Some(node))
    }
}

/// Charge release follows both payload disposal and actual node deallocation.
/// Interruption retains the charge; no destructor may turn it into a refund.
fn release_node<T, C>(node: Box<Node<T, C>>) {
    // SAFETY: preparation initialized admission before exposing this sole node
    // owner. No field destructor reads it; it is consumed exactly once here.
    let charge = ManuallyDrop::new(unsafe { node.charge.assume_init_read() });
    drop(node);
    drop(ManuallyDrop::into_inner(charge));
}

impl<T, C> Drop for PreparedEntry<T, C> {
    fn drop(&mut self) {
        if let Some(node) = self.0.take() {
            release_node(node);
        }
    }
}

/// A detached node. Only explicit release destroys its payload. Drop retains
/// the node/backing rather than running physical cleanup in an unknown context.
#[must_use]
pub(crate) struct RetiredEntry<T, C = ()>(Option<Box<Node<T, C>>>);

impl<T, C> RetiredEntry<T, C> {
    pub(crate) fn value(&self) -> &T {
        self.0.as_ref().unwrap().value.as_ref().unwrap()
    }

    pub(crate) fn value_mut(&mut self) -> &mut T {
        self.0.as_mut().unwrap().value.as_mut().unwrap()
    }

    fn into_node(mut self) -> Box<Node<T, C>> {
        self.0.take().unwrap()
    }

    pub(crate) fn release(self) {
        release_node(self.into_node());
    }

    /// Transfer the complete typed payload into its explicit disposal while
    /// retaining this node until that disposal returns. A charge covering any
    /// payload allocation requires the callback to confirm its disposal too.
    /// On interruption the
    /// node remains retained; the payload's owner defines its own fallback.
    pub(crate) fn release_with(mut self, dispose: impl FnOnce(T)) {
        let value = self.0.as_mut().unwrap().value.take().unwrap();
        dispose(value);
        release_node(self.into_node());
    }
}

impl<T, C> Drop for RetiredEntry<T, C> {
    fn drop(&mut self) {
        if let Some(node) = self.0.take() {
            core::mem::forget(node);
        }
    }
}

/// Prepend/pop owning nodes. Reverse detached batches to retain insertion order.
pub(crate) struct RetirementList<T, C = ()> {
    head: Option<Box<Node<T, C>>>,
}

impl<T, C> RetirementList<T, C> {
    pub(crate) const fn new() -> Self {
        Self {
            head: None,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    pub(crate) fn push(&mut self, entry: RetiredEntry<T, C>) {
        let mut node = entry.into_node();
        node.next = self.head.take();
        self.head = Some(node);
    }

    pub(crate) fn pop(&mut self) -> Option<RetiredEntry<T, C>> {
        let mut node = self.head.take()?;
        self.head = node.next.take();
        Some(RetiredEntry(Some(node)))
    }

    pub(crate) fn reverse(&mut self) {
        let mut reversed = Self::new();
        while let Some(entry) = self.pop() {
            reversed.push(entry);
        }
        core::mem::swap(self, &mut reversed);
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        let mut next = self.head.as_deref();
        core::iter::from_fn(move || {
            let node = next?;
            next = node.next.as_deref();
            node.value.as_ref()
        })
    }

    /// Relink admitted nodes in caller-defined order without allocating.
    pub(crate) fn insert_before(
        &mut self,
        entry: RetiredEntry<T, C>,
        mut before: impl FnMut(&T) -> bool,
    ) {
        let mut link = &mut self.head;
        while link.as_ref().is_some_and(|node| !before(node.value.as_ref().unwrap())) {
            link = &mut link.as_mut().unwrap().next;
        }
        let mut node = entry.into_node();
        node.next = link.take();
        *link = Some(node);
    }

    /// Detach the owning node; its payload and allocation survive until an
    /// explicit post-guard release. Rejection leaves the list unchanged.
    pub(crate) fn take_first(
        &mut self,
        mut matches: impl FnMut(&T) -> bool,
    ) -> Option<RetiredEntry<T, C>> {
        let mut link = &mut self.head;
        while let Some(node) = link.as_ref() {
            if matches(node.value.as_ref().unwrap()) {
                let mut node = link.take().unwrap();
                *link = node.next.take();
                return Some(RetiredEntry(Some(node)));
            }
            link = &mut link.as_mut().unwrap().next;
        }
        None
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        IterMut(self.head.as_deref_mut())
    }
}

struct IterMut<'a, T, C>(Option<&'a mut Node<T, C>>);
impl<'a, T, C> Iterator for IterMut<'a, T, C> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.0.take()?;
        self.0 = node.next.as_deref_mut();
        node.value.as_mut()
    }
}

impl<T, C> Default for RetirementList<T, C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, C> Drop for RetirementList<T, C> {
    fn drop(&mut self) {
        if let Some(head) = self.head.take() {
            // O(1) abandonment: no recursive chain destruction or T::drop.
            core::mem::forget(head);
        }
    }
}

/// Ordered owning registry storage. Prepare before publication; detach before
/// explicit disposal. Lookup and sorted insertion are linear scans.
pub(crate) struct AdmittedMap<K, V, C = ()>(RetirementList<(K, V), C>);
impl<K: Ord + Copy, V, C> AdmittedMap<K, V, C> {
    pub(crate) const fn new() -> Self {
        Self(RetirementList::new())
    }

    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(crate) fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    /// Mutate payloads while preserving immutable keys and sorted node order.
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.0.iter_mut().map(|(k, v)| (&*k, v))
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter().map(|(_, v)| v)
    }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.0.iter_mut().map(|(_, v)| v)
    }

    pub(crate) fn first_key_value(&self) -> Option<(&K, &V)> {
        self.iter().next()
    }

    pub(crate) fn insert(&mut self, entry: PreparedEntry<(K, V), C>, key: K, value: V) {
        assert!(!self.contains_key(&key), "admitted key replaced");
        let entry = entry.publish((key, value));
        self.0.insert_before(entry, |(other, _)| *other > key);
    }

    pub(crate) fn take(&mut self, key: &K) -> Option<RetiredEntry<(K, V), C>> {
        self.0.take_first(|(k, _)| k == key)
    }
}
impl<K: Ord + Copy, V, C> Default for AdmittedMap<K, V, C> {
    fn default() -> Self {
        Self::new()
    }
}
impl<K: Ord + Copy, V, C> Index<&K> for AdmittedMap<K, V, C> {
    type Output = V;

    fn index(&self, key: &K) -> &V {
        self.get(key).expect("admitted registry key absent")
    }
}
impl<K: Ord + Copy + fmt::Debug, V: fmt::Debug, C> fmt::Debug for AdmittedMap<K, V, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
#[path = "retirement_list/tests.rs"]
mod tests;
