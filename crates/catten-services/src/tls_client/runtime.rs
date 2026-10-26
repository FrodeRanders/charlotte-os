//! Domain-owned entropy checkout and nonblocking handshake admission.
//!
//! Only owner transfers run under the slot mutex. Neither entropy RPCs nor
//! handshakes retain that guard. Reentrant/concurrent operations reject.

use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};

use spin::Mutex;

enum State<Source> {
    Uninstalled,
    Available(Source),
    Borrowed,
}

pub(super) struct EntropyContext<Source> {
    state: Mutex<State<Source>>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum Unavailable {
    Uninstalled,
    Busy,
}

impl<Source> EntropyContext<Source> {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(State::Uninstalled),
        }
    }

    /// On rejection return the new owner; never replace the installed source.
    pub(super) fn install(&self, source: Source) -> Result<(), Source> {
        let mut state = self.state.lock();
        if !matches!(*state, State::Uninstalled) {
            return Err(source);
        }
        *state = State::Available(source);
        Ok(())
    }

    pub(super) fn checkout(&self) -> Result<EntropyLease<'_, Source>, Unavailable> {
        let mut state = self.state.lock();
        match core::mem::replace(&mut *state, State::Borrowed) {
            State::Available(source) => Ok(EntropyLease {
                context: self,
                source: Some(source),
            }),
            State::Uninstalled => {
                *state = State::Uninstalled;
                Err(Unavailable::Uninstalled)
            }
            State::Borrowed => Err(Unavailable::Busy),
        }
    }
}

pub(super) struct EntropyLease<'context, Source> {
    context: &'context EntropyContext<Source>,
    source: Option<Source>,
}

impl<Source> EntropyLease<'_, Source> {
    pub(super) fn source(&self) -> &Source {
        self.source.as_ref().expect("source retained through checkout")
    }
}

impl<Source> Drop for EntropyLease<'_, Source> {
    fn drop(&mut self) {
        let mut state = self.context.state.lock();
        debug_assert!(matches!(*state, State::Borrowed));
        *state = State::Available(self.source.take().expect("one checkout returns its source"));
    }
}

pub(super) struct HandshakeClock {
    active: AtomicBool,
    seconds: AtomicU64,
}

impl HandshakeClock {
    pub(super) const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            seconds: AtomicU64::new(0),
        }
    }

    pub(super) fn begin(&self, seconds: u64) -> Option<HandshakeGuard<'_>> {
        if seconds == 0
            || self
                .active
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
        {
            return None;
        }
        self.seconds.store(seconds, Ordering::Release);
        Some(HandshakeGuard(self))
    }

    pub(super) fn now(&self) -> Option<u64> {
        let seconds = self.seconds.load(Ordering::Acquire);
        (seconds != 0).then_some(seconds)
    }
}

pub(super) struct HandshakeGuard<'clock>(&'clock HandshakeClock);

impl Drop for HandshakeGuard<'_> {
    fn drop(&mut self) {
        self.0.seconds.store(0, Ordering::Release);
        self.0.active.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::sync::{
        Arc,
        atomic::AtomicUsize,
    };

    use super::*;

    struct Source(Arc<AtomicUsize>);
    impl Drop for Source {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn admission_and_duplicate_installation_preserve_owners() {
        let drops = Arc::new(AtomicUsize::new(0));
        let context = EntropyContext::new();
        assert!(matches!(context.checkout(), Err(Unavailable::Uninstalled)));
        assert!(context.install(Some(Source(drops.clone()))).is_ok());
        let rejected = context.install(Some(Source(drops.clone()))).err().unwrap();
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        drop(rejected);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        let lease = context.checkout().unwrap();
        assert!(lease.source().is_some());
        // The metadata lock is released even while the source is checked out.
        assert!(context.state.try_lock().is_some());
        assert!(matches!(context.checkout(), Err(Unavailable::Busy)));
        assert!(context.install(None).is_err());
        drop(lease);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        drop(context.checkout().unwrap());
        drop(context);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn failed_operation_and_unwind_return_the_same_source() {
        let drops = Arc::new(AtomicUsize::new(0));
        let context = EntropyContext::new();
        assert!(context.install(Some(Source(drops.clone()))).is_ok());
        for stage in ["submission", "reply", "mapping", "copy"] {
            let rejected = || -> Result<(), &str> {
                let _lease = context.checkout().unwrap();
                Err(stage)
            };
            assert_eq!(rejected(), Err(stage));
            assert_eq!(drops.load(Ordering::Relaxed), 0);
            drop(context.checkout().unwrap());
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _lease = context.checkout().unwrap();
                panic!("fixture cancellation");
            }))
            .is_err()
        );
        drop(context.checkout().unwrap());
        drop(context);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn architectural_only_installation_is_explicit_and_exclusive() {
        let context = EntropyContext::<Option<()>>::new();
        assert!(context.install(None).is_ok());
        let lease = context.checkout().unwrap();
        assert!(lease.source().is_none());
        assert!(matches!(context.checkout(), Err(Unavailable::Busy)));
        drop(lease);
        assert!(context.checkout().is_ok());
    }

    #[test]
    fn checked_out_source_rejects_another_thread_without_waiting() {
        let context = EntropyContext::<Option<()>>::new();
        context.install(None).unwrap();
        let lease = context.checkout().unwrap();
        std::thread::scope(|scope| {
            scope
                .spawn(|| assert!(matches!(context.checkout(), Err(Unavailable::Busy))))
                .join()
                .unwrap();
        });
        drop(lease);
        assert!(context.checkout().is_ok());
    }

    #[test]
    fn handshake_clock_rejects_overlap_and_clears_on_drop() {
        let clock = HandshakeClock::new();
        assert!(clock.begin(0).is_none());
        assert_eq!(clock.now(), None);
        let first = clock.begin(42).unwrap();
        assert!(clock.begin(99).is_none());
        std::thread::scope(|scope| {
            scope.spawn(|| assert!(clock.begin(101).is_none())).join().unwrap();
        });
        assert_eq!(clock.now(), Some(42));
        drop(first);
        assert_eq!(clock.now(), None);
        drop(clock.begin(99).unwrap());
        assert_eq!(clock.now(), None);
    }
}
