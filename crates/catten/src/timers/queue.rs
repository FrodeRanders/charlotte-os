//! Sorted, owning anonymous nodes plus one embedded scheduler-quantum slot.
//! Anonymous nodes are allocated fallibly before any blocked/record publication.
//! No high-water VecDeque backing or allocation occurs during insertion.

use alloc::boxed::Box;

use super::TimerEvent;

#[derive(Debug)]
pub(super) struct Node {
    pub(super) event: TimerEvent,
    next: Option<Box<Node>>,
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
    head: Option<Box<Node>>,
    quantum: Option<TimerEvent>,
    anonymous: usize,
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

    pub(super) fn insert_prepared(&mut self, mut node: Box<Node>) {
        assert!(node.event.key.is_none() && node.next.is_none());
        let mut link = &mut self.head;
        while link.as_ref().is_some_and(|queued| queued.event.deadline <= node.event.deadline) {
            link = &mut link.as_mut().unwrap().next;
        }
        node.next = link.take();
        *link = Some(node);
        self.anonymous += 1;
    }

    pub(super) fn pop_front(&mut self) -> Option<TimerEvent> {
        if self.quantum.as_ref().is_some_and(|quantum| {
            self.head.as_ref().is_none_or(|head| quantum.deadline <= head.event.deadline)
        }) {
            return self.quantum.take();
        }
        let mut node = self.head.take()?;
        self.head = node.next.take();
        self.anonymous -= 1;
        Some(node.event)
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&TimerEvent) -> bool) {
        if self.quantum.as_ref().is_some_and(|event| !keep(event)) {
            self.quantum = None;
        }
        let mut link = &mut self.head;
        while let Some(node) = link.as_ref() {
            if keep(&node.event) {
                link = &mut link.as_mut().unwrap().next;
            } else {
                let mut removed = link.take().unwrap();
                *link = removed.next.take();
                self.anonymous -= 1;
                drop(removed);
            }
        }
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        // Kernel stacks must not recursively destroy a node-sized chain.
        while self.pop_front().is_some() {}
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
