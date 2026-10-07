//! Preallocated retirement nodes. Linking, detaching and filtering never grow
//! storage. Abandoned published nodes/lists quarantine their payloads.

#![cfg_attr(test, feature(allocator_api))]

use alloc::boxed::Box;
use core::{
    alloc::AllocError,
    fmt,
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

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
#[path = "retirement_list/tests.rs"]
mod tests;
