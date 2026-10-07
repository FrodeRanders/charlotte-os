//! Fallible, one-shot registrations. Token destruction enters only this
//! independent list lock, never its source subsystem or a callback.

use alloc::{
    boxed::Box,
    sync::{
        Arc,
        Weak,
    },
};

use super::{
    Observer,
    list_budget,
};
use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::charged_allocator::ChargedAllocator,
};

#[derive(Debug)]
pub(crate) struct ListRef<C>(Arc<ObserverList<C>, ChargedAllocator<list_budget::Charge>>);

impl<C> Clone for ListRef<C> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C> core::ops::Deref for ListRef<C> {
    type Target = ObserverList<C>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationError {
    Closed,
    ResourceLimit,
    AllocationFailed,
}

#[derive(Debug)]
struct Entry<C> {
    node: Box<EntryNode<C>>,
    // Outside the allocation, and last: free it before returning admission.
    _charge: C,
}

#[derive(Debug)]
struct EntryNode<C> {
    id: u64,
    observer: Weak<dyn Observer>,
    next: Option<Entry<C>>,
}

#[derive(Debug)]
struct State<C> {
    head: Option<Entry<C>>,
    count: usize,
    next_id: u64,
    closed: bool,
}

#[derive(Debug)]
pub(crate) struct ObserverList<C> {
    state: Mutex<State<C>>,
    capacity: usize,
}

impl<C> ObserverList<C> {
    pub(crate) fn try_new(
        capacity: usize,
        platform: bool,
    ) -> Result<ListRef<C>, RegistrationError> {
        Self::try_new_with(capacity, platform, |value, allocator| allocator.try_arc(value))
    }

    fn try_new_with(
        capacity: usize,
        platform: bool,
        allocate: impl FnOnce(
            Self,
            ChargedAllocator<list_budget::Charge>,
        ) -> Result<
            Arc<Self, ChargedAllocator<list_budget::Charge>>,
            core::alloc::AllocError,
        >,
    ) -> Result<ListRef<C>, RegistrationError> {
        let allocator = ChargedAllocator::try_new(list_budget::reserve(platform)?)
            .map_err(|_| RegistrationError::AllocationFailed)?;
        allocate(
            Self {
                state: Mutex::new(State {
                    head: None,
                    count: 0,
                    next_id: 1,
                    closed: false,
                }),
                capacity,
            },
            allocator,
        )
        .map(ListRef)
        .map_err(|_| RegistrationError::AllocationFailed)
    }
}

impl<C> ListRef<C> {
    pub(crate) fn register(
        &self,
        observer: Weak<dyn Observer>,
        charge: C,
    ) -> Result<Registration<C>, RegistrationError> {
        self.register_with(observer, charge, |node| {
            Box::try_new(node).map_err(|_| RegistrationError::AllocationFailed)
        })
    }

    fn register_with(
        &self,
        observer: Weak<dyn Observer>,
        charge: C,
        allocate: impl FnOnce(EntryNode<C>) -> Result<Box<EntryNode<C>>, RegistrationError>,
    ) -> Result<Registration<C>, RegistrationError> {
        // Allocate before taking the list lock. Rejection drops the staged
        // entry/charge after the guard; no uncharged spare capacity remains.
        let mut entry = Entry {
            node: allocate(EntryNode {
                id: 0,
                observer,
                next: None,
            })?,
            _charge: charge,
        };
        let mut state = self.state.lock();
        if state.closed {
            return Err(RegistrationError::Closed);
        }
        if state.count >= self.capacity {
            return Err(RegistrationError::ResourceLimit);
        }
        let next_id = state.next_id.checked_add(1).ok_or(RegistrationError::ResourceLimit)?;
        let id = state.next_id;
        state.next_id = next_id;
        entry.node.id = id;
        entry.node.next = state.head.take();
        state.head = Some(entry);
        state.count += 1;
        Ok(Registration {
            list: self.clone(),
            id,
        })
    }
}

impl<C> ObserverList<C> {
    fn remove(&self, id: u64) {
        let mut state = self.state.lock();
        let mut link = &mut state.head;
        while let Some(entry) = link.as_ref() {
            if entry.node.id == id {
                let mut removed = link.take().unwrap();
                *link = removed.node.next.take();
                state.count -= 1;
                drop(state);
                drop(removed);
                return;
            }
            link = &mut link.as_mut().unwrap().node.next;
        }
    }

    /// Detach without allocation or callbacks. The caller must notify the
    /// batch only after releasing its source registry/other subsystem locks.
    pub(crate) fn close(&self) -> NotificationBatch<C> {
        let mut state = self.state.lock();
        state.closed = true;
        state.count = 0;
        NotificationBatch(state.head.take())
    }

    /// Reusable event sources detach their current batch without closing.
    pub(crate) fn drain(&self) -> NotificationBatch<C> {
        let mut state = self.state.lock();
        state.count = 0;
        NotificationBatch(state.head.take())
    }

    pub(crate) fn registered(&self) -> usize {
        self.state.lock().count
    }
}

/// Kernel fixture: inject entry allocation failure after admission. The
/// production path still uses Box::try_new; no physical allocator is exhausted.
pub(crate) fn test_entry_allocation_rollback() {
    use crate::{
        completion::watch_budget as budget,
        klib::observer::CallOnNotify,
    };
    let account = budget::DomainBudget::new(1);
    let list = ObserverList::try_new(1, false).unwrap();
    let callback: Arc<dyn Observer> = CallOnNotify::new(|| panic!("failed entry notified"));
    assert_eq!(
        list.register_with(
            Arc::downgrade(&callback),
            budget::reserve(&account, false).unwrap(),
            |_| Err(RegistrationError::AllocationFailed),
        )
        .unwrap_err(),
        RegistrationError::AllocationFailed
    );
    assert_eq!(list.registered(), 0);
    assert_eq!(account.used(), 0);
    assert_eq!(Arc::weak_count(&callback), 0);
    let token = list
        .register(Arc::downgrade(&callback), budget::reserve(&account, false).unwrap())
        .unwrap();
    assert_eq!(account.used(), 1);
    assert_eq!(Arc::weak_count(&callback), 1);
    drop(token);
    assert_eq!(account.used(), 0);
    assert_eq!(Arc::weak_count(&callback), 0);
    list_tests::test_list_backing_admission();
}

impl<C> Drop for ObserverList<C> {
    fn drop(&mut self) {
        // Avoid recursive linked-list destruction on a small kernel stack.
        drop(NotificationBatch(self.state.get_mut().head.take()));
    }
}

#[must_use]
#[derive(Debug)]
pub(crate) struct Registration<C> {
    list: ListRef<C>,
    id: u64,
}

impl<C> Drop for Registration<C> {
    fn drop(&mut self) {
        self.list.remove(self.id);
    }
}

pub(crate) struct NotificationBatch<C>(Option<Entry<C>>);

impl<C> NotificationBatch<C> {
    pub(crate) const fn empty() -> Self {
        Self(None)
    }

    /// Combine detached batches without allocation. Traversal is bounded by
    /// the appended source batch; notification order is not an IPC contract.
    pub(crate) fn append(&mut self, mut other: Self) {
        let Some(mut head) = other.0.take() else {
            return;
        };
        let mut tail = &mut head;
        while tail.node.next.is_some() {
            tail = tail.node.next.as_mut().unwrap();
        }
        tail.node.next = self.0.take();
        self.0 = Some(head);
    }

    pub(crate) fn notify(mut self) {
        while let Some(mut entry) = self.0.take() {
            self.0 = entry.node.next.take();
            let observer = entry.node.observer.upgrade();
            // Release the list entry before invoking a potentially reentrant
            // callback. Its strong observer reference remains live separately.
            drop(entry);
            if let Some(observer) = observer {
                observer.notify();
            }
        }
    }
}

impl<C> Drop for NotificationBatch<C> {
    fn drop(&mut self) {
        while let Some(mut entry) = self.0.take() {
            self.0 = entry.node.next.take();
        }
    }
}

#[path = "registration/list_tests.rs"]
mod list_tests;
