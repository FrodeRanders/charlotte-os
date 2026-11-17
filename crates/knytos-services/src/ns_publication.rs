//! One registry publication owns its connection and exact endpoint-death watch.
//! CQ entries are hints; only the watch's confirmed endpoint-close result
//! permits logical unpublication. Neither grants physical cleanup authority.

use catten_rt::owned::{
    Completion,
    CompletionError,
    Connection,
    ConnectionRef,
};
use catten_syscall::CqEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    Unrelated,
    Pending,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchError {
    Completion(CompletionError),
    UnexpectedResult(i64),
}

pub trait CloseWatch {
    fn observe(&mut self, entry: &CqEntry) -> Result<Observation, WatchError>;
}

impl CloseWatch for Completion {
    fn observe(&mut self, entry: &CqEntry) -> Result<Observation, WatchError> {
        if !self.matches_cq(entry) {
            return Ok(Observation::Unrelated);
        }
        match self.poll().map_err(WatchError::Completion)? {
            None => Ok(Observation::Pending),
            Some(catten_syscall::IPC_REPLY_ENDPOINT_CLOSED) => Ok(Observation::Closed),
            Some(result) => Err(WatchError::UnexpectedResult(result)),
        }
    }
}

/// Field order cancels/disposes the watch before closing its borrowed source.
pub struct Publication<C = Connection, W = Completion> {
    watch: W,
    connection: C,
}

impl<C, W: CloseWatch> Publication<C, W> {
    /// Prepare before policy/registry mutation. Rejection returns the complete
    /// source owner; it is not an accepted publication or a retry obligation.
    pub fn prepare_with<E>(
        connection: C,
        watch: impl FnOnce(&C) -> Result<W, E>,
    ) -> Result<Self, (C, E)> {
        match watch(&connection) {
            Ok(watch) => Ok(Self {
                watch,
                connection,
            }),
            Err(error) => Err((connection, error)),
        }
    }

    pub fn observe(&mut self, entry: &CqEntry) -> Result<Observation, WatchError> {
        self.watch.observe(entry)
    }
}

impl Publication {
    pub fn prepare(connection: Connection) -> Result<Self, (Connection, CompletionError)> {
        Self::prepare_with(connection, Connection::watch_closed)
    }

    pub fn connection(&self) -> ConnectionRef<'_> {
        self.connection.as_ref()
    }
}

/// One slot is reserved at launch for preparing a live replacement. Tombstones
/// remain in the existing bounded policy store and need no live watch.
pub fn publication_admitted(live: usize, replacing_live: bool) -> bool {
    live < charlotte_protocol_ns::MAX_LIVE_PUBLICATIONS
        || (replacing_live && live == charlotte_protocol_ns::MAX_LIVE_PUBLICATIONS)
}

#[cfg(test)]
mod tests {
    use alloc::{
        rc::Rc,
        vec::Vec,
    };
    use core::cell::RefCell;

    use super::*;

    type Events = Rc<RefCell<Vec<&'static str>>>;
    struct Source(Events);
    impl Drop for Source {
        fn drop(&mut self) {
            self.0.borrow_mut().push("close source");
        }
    }
    struct Watch {
        events: Events,
        cookie: u64,
        result: Result<Observation, WatchError>,
    }
    impl CloseWatch for Watch {
        fn observe(&mut self, entry: &CqEntry) -> Result<Observation, WatchError> {
            if self.cookie != entry.cookie {
                return Ok(Observation::Unrelated);
            }
            self.events.borrow_mut().push("observe watch");
            self.result
        }
    }
    impl Drop for Watch {
        fn drop(&mut self) {
            self.events.borrow_mut().push("cancel watch");
        }
    }
    fn entry(cookie: u64) -> CqEntry {
        CqEntry {
            operation: 1,
            cookie,
            status: 0,
            flags: 0,
            result: -7,
        }
    }
    fn prepared(
        events: &Events,
        cookie: u64,
        result: Result<Observation, WatchError>,
    ) -> Publication<Source, Watch> {
        Publication::prepare_with(Source(events.clone()), |_| {
            Ok::<_, ()>(Watch {
                events: events.clone(),
                cookie,
                result,
            })
        })
        .unwrap_or_else(|_| panic!("preparation"))
    }
    #[test]
    fn rejected_preparation_returns_source_without_a_watch() {
        let events = Events::default();
        let result = Publication::<_, Watch>::prepare_with(Source(events.clone()), |_| Err("full"));
        let (source, error) = result.err().unwrap();
        assert_eq!(error, "full");
        assert!(events.borrow().is_empty());
        drop(source);
        assert_eq!(*events.borrow(), ["close source"]);
    }
    #[test]
    fn replacement_and_explicit_retirement_cancel_watch_before_source() {
        let events = Events::default();
        let old = prepared(&events, 11, Ok(Observation::Pending));
        let mut replacement = prepared(&events, 12, Ok(Observation::Closed));
        drop(old);
        assert_eq!(replacement.observe(&entry(11)), Ok(Observation::Unrelated));
        assert_eq!(*events.borrow(), ["cancel watch", "close source"]);
        assert_eq!(replacement.observe(&entry(12)), Ok(Observation::Closed));
        drop(replacement);
        assert_eq!(
            *events.borrow(),
            ["cancel watch", "close source", "observe watch", "cancel watch", "close source"]
        );
    }
    #[test]
    fn matching_hint_does_not_complete_a_pending_or_uncertain_watch() {
        let events = Events::default();
        let mut pending = prepared(&events, 11, Ok(Observation::Pending));
        assert_eq!(pending.observe(&entry(11)), Ok(Observation::Pending));
        let mut uncertain = prepared(&events, 12, Err(WatchError::UnexpectedResult(-3)));
        assert_eq!(uncertain.observe(&entry(12)), Err(WatchError::UnexpectedResult(-3)));
        assert_eq!(*events.borrow(), ["observe watch", "observe watch"]);
        drop(pending);
        drop(uncertain);
    }
    #[test]
    fn live_capacity_keeps_one_replacement_preparation_slot() {
        let max = charlotte_protocol_ns::MAX_LIVE_PUBLICATIONS;
        assert!(publication_admitted(max - 1, false));
        assert!(!publication_admitted(max, false));
        assert!(publication_admitted(max, true));
        assert!(!publication_admitted(max + 1, true));
    }
}
