//! Bounded owned call futures for the existing Sitas ShardExecutor.
//!
//! Wrap the selected CharlotteReactor in WakeReactor before giving it to the
//! executor. IPC reply readiness is a CQ wake, not necessarily a cookie record:
//! every backend wake rechecks the bounded set of registered pending owners.
//! This crate never consumes a CQ ring independently of the selected backend.
#![no_std]
#![feature(allocator_api)]
extern crate alloc;
use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::{
    future::Future,
    pin::Pin,
    task::{
        Context,
        Poll,
        Waker,
    },
    time::Duration,
};

use catten_rt::{
    call::CallObservation,
    deadline::Deadline,
    owned::{
        CallResult,
        IpcError,
        PendingCall,
    },
};
use sitas_core::{
    io,
    reactor_backend::ReactorBackend,
};
use spin::Mutex;

struct Slot {
    generation: u64,
    occupied: bool,
    waker: Option<Waker>,
    deadline: Option<Deadline>,
}
struct Registry {
    slots: Mutex<Vec<Slot>>,
    clock: fn() -> u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    Capacity,
    Binding(IpcError),
}

impl Registry {
    fn reserve(
        self: &Arc<Self>,
        deadline: Option<Deadline>,
    ) -> Result<Interest, RegistrationError> {
        let mut slots = self.slots.lock();
        for (index, slot) in slots.iter_mut().enumerate() {
            if !slot.occupied && slot.generation != u64::MAX {
                slot.generation += 1;
                slot.occupied = true;
                slot.deadline = deadline;
                return Ok(Interest {
                    registry: Arc::clone(self),
                    index,
                    generation: slot.generation,
                });
            }
        }
        Err(RegistrationError::Capacity)
    }

    fn wake_all(&self) {
        let count = self.slots.lock().len();
        for index in 0..count {
            let waker = self.slots.lock()[index].waker.take();
            // Callbacks and waker destruction always leave the registry guard.
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    fn remaining(&self) -> Option<Duration> {
        let count = self.slots.lock().len();
        let mut earliest = None;
        for index in 0..count {
            let deadline = self.slots.lock()[index].deadline;
            if let Some(deadline) = deadline {
                let remaining = Duration::from_millis(deadline.remaining_ms_at((self.clock)()));
                earliest = Some(earliest.map_or(remaining, |old: Duration| old.min(remaining)));
            }
        }
        earliest
    }
}

struct Interest {
    registry: Arc<Registry>,
    index: usize,
    generation: u64,
}
impl Interest {
    fn update(&self, waker: &Waker) {
        let new = waker.clone();
        let old = {
            let mut slots = self.registry.slots.lock();
            let slot = &mut slots[self.index];
            assert!(slot.occupied && slot.generation == self.generation);
            slot.waker.replace(new)
        };
        drop(old);
    }
}
impl Drop for Interest {
    fn drop(&mut self) {
        let old = {
            let mut slots = self.registry.slots.lock();
            let slot = &mut slots[self.index];
            assert!(slot.occupied && slot.generation == self.generation);
            slot.occupied = false;
            slot.deadline = None;
            slot.waker.take()
        };
        drop(old);
    }
}

/// One wrapper per shard/CQ. It owns no capability; the underlying backend
/// remains the sole CQ consumer. Prepare its fixed registration table first.
pub struct WakeReactor<R> {
    backend: R,
    registry: Arc<Registry>,
    cq: u32,
}
impl<R> WakeReactor<R> {
    pub fn new(backend: R, cq: u32, capacity: usize) -> Result<Self, RegistrationError> {
        Self::with_clock(backend, cq, capacity, || catten_rt::abi::monotonic_clock().0)
    }

    fn with_clock(
        backend: R,
        cq: u32,
        capacity: usize,
        clock: fn() -> u64,
    ) -> Result<Self, RegistrationError> {
        if capacity == 0 {
            return Err(RegistrationError::Capacity);
        }
        let mut slots = Vec::new();
        slots.try_reserve_exact(capacity).map_err(|_| RegistrationError::Capacity)?;
        for _ in 0..capacity {
            slots.push(Slot {
                generation: 0,
                occupied: false,
                waker: None,
                deadline: None,
            });
        }
        let registry = Arc::try_new(Registry {
            slots: Mutex::new(slots),
            clock,
        })
        .map_err(|_| RegistrationError::Capacity)?;
        Ok(Self {
            backend,
            registry,
            cq,
        })
    }

    pub fn handle(&self) -> Calls {
        Calls {
            registry: Arc::clone(&self.registry),
            cq: self.cq,
        }
    }
}
impl<R: ReactorBackend> ReactorBackend for WakeReactor<R> {
    type Event = R::Event;
    type Handle = R::Handle;
    type Waker = R::Waker;

    fn waker(&self) -> Self::Waker {
        self.backend.waker()
    }

    fn wait(
        &self,
        read: &[R::Handle],
        write: &[R::Handle],
        timeout: Option<Duration>,
    ) -> io::Result<R::Event> {
        let timeout = match (timeout, self.registry.remaining()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let event = match self.backend.wait(read, write, timeout) {
            Ok(event) => event,
            Err(error) => fail_wait(error),
        };
        self.registry.wake_all();
        Ok(event)
    }
}

// Older supported Sitas executors retry backend errors. A rejected wait must
// instead retain the executor's complete pending owners through terminal abort.
fn fail_wait(error: io::ErrorKind) -> ! {
    #[cfg(test)]
    panic!("injected reactor wait rejection: {error:?}");
    #[cfg(not(test))]
    {
        catten_rt::logln!("[app-sitas] backend wait rejected: {:?}", error);
        catten_rt::domain_abort()
    }
}

#[derive(Clone)]
pub struct Calls {
    registry: Arc<Registry>,
    cq: u32,
}
impl Calls {
    /// Registration/binding failure returns the exact accepted operation.
    /// The future retains its Rust memory loan through Pending and expiry.
    pub fn call<'memory>(
        &self,
        mut pending: PendingCall<'memory>,
        deadline: Option<Deadline>,
    ) -> Result<CallFuture<'memory>, (PendingCall<'memory>, RegistrationError)> {
        let interest = match self.registry.reserve(deadline) {
            Ok(interest) => interest,
            Err(error) => return Err((pending, error)),
        };
        if let Err(error) = pending.bind_cq(self.cq) {
            return Err((pending, RegistrationError::Binding(error)));
        }
        Ok(CallFuture {
            state: Waiting {
                pending: Some(pending),
                interest: Some(interest),
                deadline,
            },
        })
    }
}

trait Pending {
    type Reply;
    fn observe(&mut self) -> Result<Option<Self::Reply>, IpcError>;
}
impl Pending for PendingCall<'_> {
    type Reply = CallResult;

    fn observe(&mut self) -> Result<Option<CallResult>, IpcError> {
        self.poll()
    }
}
enum Outcome<P: Pending> {
    Reply(P::Reply),
    Deadline(P),
    Failed(IpcError),
}
struct Waiting<P> {
    pending: Option<P>,
    interest: Option<Interest>,
    deadline: Option<Deadline>,
}
impl<P: Pending> Waiting<P> {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Outcome<P>> {
        let interest = self.interest.as_ref().expect("completed call future");
        interest.update(cx.waker());
        // Register before rechecking producer state: no callback adopts a reply.
        match self.pending.as_mut().expect("live call").observe() {
            Ok(Some(reply)) => {
                self.pending = None;
                self.interest = None;
                Poll::Ready(Outcome::Reply(reply))
            }
            Err(error) => {
                self.pending = None;
                self.interest = None;
                Poll::Ready(Outcome::Failed(error))
            }
            Ok(None) => {
                if self.deadline.is_some_and(|d| d.expired_at((interest.registry.clock)())) {
                    let pending = self.pending.take().expect("live call");
                    self.interest = None;
                    Poll::Ready(Outcome::Deadline(pending))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

/// Polling is nonblocking. Drop uses PendingCall's existing cancellation policy,
/// which may block or abort: use into_pending for explicitly managed cleanup.
#[must_use]
pub struct CallFuture<'memory> {
    state: Waiting<PendingCall<'memory>>,
}
impl<'memory> CallFuture<'memory> {
    pub fn into_pending(mut self) -> PendingCall<'memory> {
        self.state.interest = None;
        self.state.pending.take().expect("live call")
    }
}
impl<'memory> Future for CallFuture<'memory> {
    type Output = CallObservation<'memory>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().state.poll(cx).map(|outcome| match outcome {
            Outcome::Reply(reply) => CallObservation::Reply(reply),
            Outcome::Deadline(pending) => CallObservation::Deadline(pending),
            Outcome::Failed(error) => CallObservation::Failed(error),
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::{
        rc::Rc,
        task::Wake,
    };
    use core::{
        cell::Cell,
        sync::atomic::{
            AtomicU64,
            AtomicUsize,
            Ordering,
        },
    };

    use super::*;
    static NOW: AtomicU64 = AtomicU64::new(0);
    struct Count(AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    struct Fake {
        ready: Rc<Cell<bool>>,
        dropped: Rc<Cell<usize>>,
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.set(self.dropped.get() + 1);
        }
    }
    impl Pending for Fake {
        type Reply = u64;

        fn observe(&mut self) -> Result<Option<u64>, IpcError> {
            Ok(self.ready.get().then_some(7))
        }
    }
    fn registry(capacity: usize) -> Arc<Registry> {
        WakeReactor::with_clock((), 0, capacity, || NOW.load(Ordering::Relaxed)).unwrap().registry
    }
    #[test]
    fn cancellation_releases_registration_capacity_and_stale_waker() {
        let registry = registry(1);
        let interest = registry.reserve(None).unwrap();
        assert!(registry.reserve(None).is_err());
        let count = Arc::new(Count(AtomicUsize::new(0)));
        interest.update(&Waker::from(Arc::clone(&count)));
        drop(interest);
        registry.wake_all();
        assert_eq!(count.0.load(Ordering::Relaxed), 0);
        assert!(registry.reserve(None).is_ok());
    }
    #[test]
    fn producer_ready_before_poll_is_observed_after_registration() {
        let registry = registry(1);
        let dropped = Rc::new(Cell::new(0));
        let mut state = Waiting {
            pending: Some(Fake {
                ready: Rc::new(Cell::new(true)),
                dropped: dropped.clone(),
            }),
            interest: Some(registry.reserve(None).unwrap()),
            deadline: None,
        };
        let waker = Waker::from(Arc::new(Count(AtomicUsize::new(0))));
        assert!(matches!(
            state.poll(&mut Context::from_waker(&waker)),
            Poll::Ready(Outcome::Reply(7))
        ));
        assert_eq!(dropped.get(), 1);
        assert!(registry.reserve(None).is_ok());
    }
    #[test]
    fn wake_rechecks_owner_and_deadline_returns_complete_pending_owner() {
        NOW.store(0, Ordering::Relaxed);
        let registry = registry(1);
        let dropped = Rc::new(Cell::new(0));
        let deadline = Deadline::at(0, 1000, 10);
        let mut state = Waiting {
            pending: Some(Fake {
                ready: Rc::new(Cell::new(false)),
                dropped: dropped.clone(),
            }),
            interest: Some(registry.reserve(Some(deadline)).unwrap()),
            deadline: Some(deadline),
        };
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&count));
        assert!(state.poll(&mut Context::from_waker(&waker)).is_pending());
        registry.wake_all();
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        assert!(state.poll(&mut Context::from_waker(&waker)).is_pending());
        NOW.store(10, Ordering::Relaxed);
        assert_eq!(registry.remaining(), Some(Duration::ZERO));
        let Poll::Ready(Outcome::Deadline(owner)) = state.poll(&mut Context::from_waker(&waker))
        else {
            panic!("deadline")
        };
        assert_eq!(dropped.get(), 0);
        drop(owner);
        assert_eq!(dropped.get(), 1);
    }

    #[derive(Clone)]
    struct BackendWake;
    impl sitas_core::reactor_backend::ReactorWaker for BackendWake {
        fn wake(&self) -> io::Result<()> {
            Ok(())
        }
    }
    struct BackendEvent;
    impl sitas_core::reactor_backend::ReactorEvent for BackendEvent {
        type Handle = u64;

        fn woke(&self) -> bool {
            true
        }

        fn readable(&self) -> &[u64] {
            &[]
        }

        fn writable(&self) -> &[u64] {
            &[]
        }
    }
    struct Backend {
        reject: bool,
        timeout: Cell<Option<Duration>>,
    }
    impl ReactorBackend for Backend {
        type Event = BackendEvent;
        type Handle = u64;
        type Waker = BackendWake;

        fn waker(&self) -> BackendWake {
            BackendWake
        }

        fn wait(
            &self,
            _: &[u64],
            _: &[u64],
            timeout: Option<Duration>,
        ) -> io::Result<BackendEvent> {
            self.timeout.set(timeout);
            if self.reject {
                Err(io::ErrorKind::Other)
            } else {
                Ok(BackendEvent)
            }
        }
    }
    #[test]
    fn reactor_clips_waits_without_restarting_deadlines_and_dispatches_wakes() {
        let reactor = WakeReactor::with_clock(
            Backend {
                reject: false,
                timeout: Cell::new(None),
            },
            0,
            1,
            || 5,
        )
        .unwrap();
        let interest = reactor.registry.reserve(Some(Deadline::at(0, 1000, 10))).unwrap();
        let count = Arc::new(Count(AtomicUsize::new(0)));
        interest.update(&Waker::from(Arc::clone(&count)));
        reactor.wait(&[], &[], Some(Duration::from_secs(1))).unwrap();
        assert_eq!(reactor.backend.timeout.get(), Some(Duration::from_millis(5)));
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        drop(interest);
        reactor.wait(&[], &[], Some(Duration::from_secs(1))).unwrap();
        assert_eq!(reactor.backend.timeout.get(), Some(Duration::from_secs(1)));
    }
    #[test]
    fn backend_rejection_is_terminal_with_interests_retained_not_a_retry_hint() {
        let reactor = WakeReactor::with_clock(
            Backend {
                reject: true,
                timeout: Cell::new(None),
            },
            0,
            1,
            || 0,
        )
        .unwrap();
        let interest = reactor.registry.reserve(None).unwrap();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reactor.wait(&[], &[], None)));
        assert!(result.is_err());
        assert!(reactor.registry.reserve(None).is_err());
        drop(interest);
        assert!(reactor.registry.reserve(None).is_ok());
    }
}
