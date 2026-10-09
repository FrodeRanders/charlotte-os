//! Preallocated retirement nodes. Linking, detaching and filtering never grow
//! storage. Abandoned published nodes/lists quarantine their payloads.

#![cfg_attr(test, feature(allocator_api))]

use alloc::boxed::Box;
use core::{
    alloc::AllocError,
    fmt,
    ops::Index,
};

struct Node<T> {
    value: Option<T>,
    next: Option<Box<Node<T>>>,
}

/// Empty storage allocated before its payload becomes externally reachable.
#[must_use]
pub(crate) struct PreparedEntry<T>(Box<Node<T>>);

impl<T> fmt::Debug for PreparedEntry<T> {
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
        allocate(Node {
            value: None,
            next: None,
        })
        .map(Self)
    }

    pub(crate) fn publish(mut self, value: T) -> RetiredEntry<T> {
        self.0.value = Some(value);
        RetiredEntry(Some(self.0))
    }
}

/// A detached node. Only explicit release destroys its payload. Drop retains
/// the node/backing rather than running physical cleanup in an unknown context.
#[must_use]
pub(crate) struct RetiredEntry<T>(Option<Box<Node<T>>>);

impl<T> RetiredEntry<T> {
    pub(crate) fn value(&self) -> &T {
        self.0.as_ref().unwrap().value.as_ref().unwrap()
    }

    pub(crate) fn value_mut(&mut self) -> &mut T {
        self.0.as_mut().unwrap().value.as_mut().unwrap()
    }

    fn into_node(mut self) -> Box<Node<T>> {
        self.0.take().unwrap()
    }

    pub(crate) fn release(self) {
        drop(self.into_node());
    }
}

impl<T> Drop for RetiredEntry<T> {
    fn drop(&mut self) {
        if let Some(node) = self.0.take() {
            core::mem::forget(node);
        }
    }
}

/// Prepend/pop owning nodes. Reverse detached batches to retain insertion order.
pub(crate) struct RetirementList<T> {
    head: Option<Box<Node<T>>>,
}

impl<T> RetirementList<T> {
    pub(crate) const fn new() -> Self {
        Self {
            head: None,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    pub(crate) fn push(&mut self, entry: RetiredEntry<T>) {
        let mut node = entry.into_node();
        node.next = self.head.take();
        self.head = Some(node);
    }

    pub(crate) fn pop(&mut self) -> Option<RetiredEntry<T>> {
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
        entry: RetiredEntry<T>,
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
    ) -> Option<RetiredEntry<T>> {
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

struct IterMut<'a, T>(Option<&'a mut Node<T>>);
impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.0.take()?;
        self.0 = node.next.as_deref_mut();
        node.value.as_mut()
    }
}

impl<T> Default for RetirementList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for RetirementList<T> {
    fn drop(&mut self) {
        if let Some(head) = self.head.take() {
            // O(1) abandonment: no recursive chain destruction or T::drop.
            core::mem::forget(head);
        }
    }
}

/// Ordered owning registry storage. Prepare before publication; detach before
/// explicit disposal. Lookup and sorted insertion are linear scans.
pub(crate) struct AdmittedMap<K, V>(RetirementList<(K, V)>);
impl<K: Ord + Copy, V> AdmittedMap<K, V> {
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

    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter().map(|(_, v)| v)
    }

    pub(crate) fn first_key_value(&self) -> Option<(&K, &V)> {
        self.iter().next()
    }

    pub(crate) fn insert(&mut self, entry: PreparedEntry<(K, V)>, key: K, value: V) {
        assert!(!self.contains_key(&key), "admitted key replaced");
        let entry = entry.publish((key, value));
        self.0.insert_before(entry, |(other, _)| *other > key);
    }

    pub(crate) fn take(&mut self, key: &K) -> Option<RetiredEntry<(K, V)>> {
        self.0.take_first(|(k, _)| k == key)
    }
}
impl<K: Ord + Copy, V> Default for AdmittedMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}
impl<K: Ord + Copy, V> Index<&K> for AdmittedMap<K, V> {
    type Output = V;

    fn index(&self, key: &K) -> &V {
        self.get(key).expect("admitted registry key absent")
    }
}
impl<K: Ord + Copy + fmt::Debug, V: fmt::Debug> fmt::Debug for AdmittedMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
#[path = "retirement_list/tests.rs"]
mod tests;
