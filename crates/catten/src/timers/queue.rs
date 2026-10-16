//! Sorted, owning anonymous nodes plus one embedded scheduler-quantum slot.
//! Anonymous nodes are allocated fallibly before any blocked/record publication.
//! No high-water VecDeque backing or allocation occurs during insertion.

use alloc::boxed::Box;
use core::ops::{
    Deref,
    DerefMut,
};

use super::{
    EventAdmission,
    TimerEvent,
};

#[derive(Debug)]
pub(super) struct OwnedNode {
    node: Box<Node>,
    // Field order frees the entire node before returning its reservation.
    _admission: EventAdmission,
}
impl OwnedNode {
    pub(super) fn new(node: Box<Node>, admission: EventAdmission) -> Self {
        Self {
            node,
            _admission: admission,
        }
    }
}
impl Deref for OwnedNode {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.node
    }
}
impl DerefMut for OwnedNode {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

#[derive(Debug)]
pub(super) struct Node {
    pub(super) event: TimerEvent,
    next: Option<OwnedNode>,
}
impl Node {
    pub(super) fn new(event: TimerEvent) -> Self {
        Self {
            event,
            next: None,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Events {
    head: Option<OwnedNode>,
    quantum: Option<TimerEvent>,
    anonymous: usize,
}

/// Detached complete queue payloads. Drop retains their original nodes,
/// sources, callback references and charges without entering an allocator.
#[must_use = "finish detached timer events after the local queue borrow leaves"]
#[derive(Debug, Default)]
pub(crate) struct RetiredEvents(Events);

impl RetiredEvents {
    pub(super) fn reverse_nodes(&mut self) {
        let mut head = None;
        while let Some(mut node) = self.0.head.take() {
            self.0.head = node.next.take();
            node.next = head.take();
            head = Some(node);
        }
        self.0.head = head;
    }

    pub(super) fn pop_front(&mut self) -> Option<Self> {
        self.0.pop_front()
    }

    pub(super) fn push_node(&mut self, mut node: OwnedNode) {
        assert!(node.next.is_none());
        node.next = self.0.head.take();
        self.0.head = Some(node);
        self.0.anonymous += 1;
    }

    pub(super) fn push_quantum(&mut self, event: TimerEvent) {
        assert!(self.0.quantum.is_none(), "two keyed events in one queue transaction");
        self.0.quantum = Some(event);
    }

    pub(super) fn front(&self) -> Option<&TimerEvent> {
        self.0.front()
    }

    pub(crate) fn finish(self) {
        self.finish_with(|_| {});
    }

    pub(super) fn finish_with(mut self, mut before_dispose: impl FnMut(&TimerEvent)) {
        // Detach each next link before ordinary destruction, avoiding recursive
        // cleanup. OwnedNode's outside admission outlives its Box deallocation.
        while let Some(mut node) = self.0.head.take() {
            self.0.head = node.next.take();
            before_dispose(&node.event);
            drop(node);
        }
        if let Some(event) = &self.0.quantum {
            before_dispose(event);
        }
        drop(self.0.quantum.take());
    }
}

impl Events {
    pub(super) fn len(&self) -> usize {
        self.anonymous + usize::from(self.quantum.is_some())
    }

    pub(super) fn front(&self) -> Option<&TimerEvent> {
        self.iter().next()
    }

    pub(super) fn iter(&self) -> Iter<'_> {
        Iter {
            next: self.head.as_deref(),
            quantum: self.quantum.as_ref(),
        }
    }

    pub(super) fn insert_quantum(&mut self, event: TimerEvent) {
        assert!(event.key.is_some() && self.quantum.is_none());
        self.quantum = Some(event);
    }

    pub(super) fn insert_prepared(&mut self, mut node: OwnedNode) {
        assert!(node.event.key.is_none() && node.next.is_none());
        let mut link = &mut self.head;
        while link.as_ref().is_some_and(|queued| queued.event.deadline <= node.event.deadline) {
            link = &mut link.as_mut().unwrap().next;
        }
        node.next = link.take();
        *link = Some(node);
        self.anonymous += 1;
    }

    pub(super) fn pop_front(&mut self) -> Option<RetiredEvents> {
        self.front()?;
        let mut retired = RetiredEvents::default();
        self.pop_front_into(&mut retired);
        Some(retired)
    }

    pub(super) fn pop_front_into(&mut self, retired: &mut RetiredEvents) {
        if self.quantum.as_ref().is_some_and(|quantum| {
            self.head.as_ref().is_none_or(|head| quantum.deadline <= head.event.deadline)
        }) {
            retired.push_quantum(self.quantum.take().unwrap());
            return;
        }
        let mut node = self.head.take().expect("empty timer queue removal");
        self.head = node.next.take();
        self.anonymous -= 1;
        retired.push_node(node);
    }

    pub(super) fn retain(
        &mut self,
        retired: &mut RetiredEvents,
        mut keep: impl FnMut(&TimerEvent) -> bool,
    ) {
        if self.quantum.as_ref().is_some_and(|event| !keep(event)) {
            retired.push_quantum(self.quantum.take().unwrap());
        }
        let mut link = &mut self.head;
        while let Some(node) = link.as_ref() {
            if keep(&node.event) {
                link = &mut link.as_mut().unwrap().next;
            } else {
                let mut removed = link.take().unwrap();
                *link = removed.next.take();
                self.anonymous -= 1;
                retired.push_node(removed);
            }
        }
    }

    pub(super) fn retire(self) -> RetiredEvents {
        RetiredEvents(self)
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        // Unknown queue/receipt abandonment retains the complete payloads,
        // including the inline keyed event. Explicit finish alone disposes.
        if let Some(head) = self.head.take() {
            core::mem::forget(head);
        }
        if let Some(quantum) = self.quantum.take() {
            core::mem::forget(quantum);
        }
    }
}
pub(super) struct Iter<'a> {
    next: Option<&'a Node>,
    quantum: Option<&'a TimerEvent>,
}
impl<'a> Iterator for Iter<'a> {
    type Item = &'a TimerEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.quantum.is_some_and(|quantum| {
            self.next.is_none_or(|node| quantum.deadline <= node.event.deadline)
        }) {
            return self.quantum.take();
        }
        let node = self.next?;
        self.next = node.next.as_deref();
        Some(&node.event)
    }
}
