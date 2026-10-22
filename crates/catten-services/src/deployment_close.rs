//! Bounded, CQ-bound logical socket close for deployment client phases.
use catten_rt::owned::IpcError;

use crate::{
    deadline::Deadline,
    deployment_input::MAX_CLIENTS,
    socket::{
        ClosingSocket,
        OwnedSocket,
        SocketError,
    },
};

pub const CLOSE_MS: u64 = 5_000;
/// Queue-full admission has no producer readiness source. Accepted calls use CQ.
pub const ADMISSION_BACKOFF_MS: u64 = 50;

#[derive(Debug)]
pub enum Error {
    Deadline,
    Submission(IpcError),
    Binding(IpcError),
    Observation(SocketError),
}

trait Calls {
    type Socket;
    type Owner;
    fn now(&self) -> (u64, u64);
    fn prepare(socket: Self::Socket) -> Self::Owner;
    fn submit(&mut self, owner: &mut Self::Owner) -> Result<(), IpcError>;
    fn bind(&mut self, owner: &mut Self::Owner, cq: u32) -> Result<(), IpcError>;
    fn poll(&mut self, owner: &mut Self::Owner) -> Result<bool, SocketError>;
    fn abort(&mut self, owner: &Self::Owner, error: Error) -> !;
}
struct Close<O> {
    owner: O,
    deadline: Deadline,
    retry: Option<Deadline>,
}
struct Machine<C: Calls> {
    closes: [Option<Close<C::Owner>>; MAX_CLIENTS],
    calls: C,
    cursor: usize,
    cq: u32,
}
impl<C: Calls> Machine<C> {
    fn new(calls: C, cq: u32) -> Self {
        Self {
            closes: core::array::from_fn(|_| None),
            calls,
            cursor: 0,
            cq,
        }
    }

    fn start(&mut self, socket: C::Socket) -> Result<(), C::Socket> {
        let Some(index) = self.closes.iter().position(Option::is_none) else {
            return Err(socket);
        };
        let (now, frequency) = self.calls.now();
        self.closes[index] = Some(Close {
            owner: C::prepare(socket),
            deadline: Deadline::at(now, frequency, CLOSE_MS),
            retry: None,
        });
        self.arm(index);
        Ok(())
    }

    fn arm(&mut self, index: usize) {
        let (now, frequency) = self.calls.now();
        let close = self.closes[index].as_mut().unwrap();
        if close.deadline.expired_at(now) {
            self.calls.abort(&close.owner, Error::Deadline);
        }
        match self.calls.submit(&mut close.owner) {
            Ok(()) => {
                close.retry = None;
                if let Err(error) = self.calls.bind(&mut close.owner, self.cq) {
                    self.calls.abort(&close.owner, Error::Binding(error));
                }
            }
            Err(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL)) => {
                // Rejection proves this submission did not enter IPC. Keep its
                // original deadline and exact owner through admission backoff.
                close.retry = Some(Deadline::at(now, frequency, ADMISSION_BACKOFF_MS));
            }
            Err(error) => self.calls.abort(&close.owner, Error::Submission(error)),
        }
    }

    fn advance(&mut self) -> bool {
        let (now, _) = self.calls.now();
        for offset in 0..MAX_CLIENTS {
            let index = (self.cursor + offset) % MAX_CLIENTS;
            let Some(close) = self.closes[index].as_mut() else {
                continue;
            };
            if close.deadline.expired_at(now) {
                self.calls.abort(&close.owner, Error::Deadline);
            }
            if let Some(retry) = close.retry {
                if !retry.expired_at(now) {
                    continue;
                }
                self.cursor = (index + 1) % MAX_CLIENTS;
                self.arm(index);
                return true;
            }
            match self.calls.poll(&mut close.owner) {
                Ok(false) => continue,
                Ok(true) => {
                    self.cursor = (index + 1) % MAX_CLIENTS;
                    drop(self.closes[index].take());
                    return true;
                }
                Err(error) => self.calls.abort(&close.owner, Error::Observation(error)),
            }
        }
        false
    }

    fn occupied_slots(&self) -> usize {
        self.closes.iter().flatten().count()
    }

    fn remaining_ms(&self) -> Option<u64> {
        let (now, _) = self.calls.now();
        self.closes
            .iter()
            .flatten()
            .map(|close| {
                let remaining = close.deadline.remaining_ms_at(now);
                close.retry.map_or(remaining, |retry| remaining.min(retry.remaining_ms_at(now)))
            })
            .min()
    }
}
struct Runtime<'a>(core::marker::PhantomData<&'a ()>);
impl<'a> Calls for Runtime<'a> {
    type Owner = ClosingSocket<'a>;
    type Socket = OwnedSocket<'a>;

    fn now(&self) -> (u64, u64) {
        catten_syscall::monotonic_clock()
    }

    fn prepare(socket: Self::Socket) -> Self::Owner {
        socket.into_closing()
    }

    fn submit(&mut self, owner: &mut Self::Owner) -> Result<(), IpcError> {
        owner.submit()
    }

    fn bind(&mut self, owner: &mut Self::Owner, cq: u32) -> Result<(), IpcError> {
        owner.bind_cq(cq)
    }

    fn poll(&mut self, owner: &mut Self::Owner) -> Result<bool, SocketError> {
        owner.poll().map(|receipt| receipt.is_some())
    }

    fn abort(&mut self, _owner: &Self::Owner, error: Error) -> ! {
        catten_rt::logln!("[deployd] logical close failed with owner retained: {error:?}");
        catten_rt::domain_abort()
    }
}
/// Closing consumes one of the same four client positions. Only a confirmed
/// logical acknowledgement releases it; uncertain close aborts with the pool live.
pub struct Closes<'a>(Machine<Runtime<'a>>);
impl<'a> Closes<'a> {
    pub fn new(cq: u32) -> Self {
        Self(Machine::new(Runtime(core::marker::PhantomData), cq))
    }

    pub fn start(&mut self, socket: OwnedSocket<'a>) {
        if let Err(_socket) = self.0.start(socket) {
            // A full close pool here would violate the containing four-phase
            // admission accounting. Retain the complete arriving socket too.
            catten_rt::domain_abort();
        }
    }

    pub fn advance(&mut self) -> bool {
        self.0.advance()
    }

    pub fn occupied_slots(&self) -> usize {
        self.0.occupied_slots()
    }

    pub fn remaining_ms(&self) -> Option<u64> {
        self.0.remaining_ms()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::{
        collections::VecDeque,
        rc::Rc,
        vec::Vec,
    };
    use core::cell::{
        Cell,
        RefCell,
    };

    use super::*;
    type Log = Rc<RefCell<Vec<(&'static str, usize)>>>;
    #[derive(Debug)]
    struct Socket(usize, Log);
    impl Drop for Socket {
        fn drop(&mut self) {
            self.1.borrow_mut().push(("socket", self.0));
        }
    }
    struct Pending(usize, Log);
    impl Drop for Pending {
        fn drop(&mut self) {
            self.1.borrow_mut().push(("pending", self.0));
        }
    }
    struct Owner {
        pending: Option<Pending>,
        socket: Socket,
        accepted: bool,
    }
    struct Fake {
        now: Rc<Cell<u64>>,
        log: Log,
        rejects: [usize; MAX_CLIENTS],
        attempts: [usize; MAX_CLIENTS],
        accepted: [usize; MAX_CLIENTS],
        fail_bind: bool,
        fail_submit: bool,
        replies: [VecDeque<Result<bool, SocketError>>; MAX_CLIENTS],
    }
    impl Calls for Fake {
        type Owner = Owner;
        type Socket = Socket;

        fn now(&self) -> (u64, u64) {
            (self.now.get(), 1000)
        }

        fn prepare(socket: Socket) -> Owner {
            Owner {
                pending: None,
                socket,
                accepted: false,
            }
        }

        fn submit(&mut self, owner: &mut Owner) -> Result<(), IpcError> {
            let id = owner.socket.0;
            assert!(!owner.accepted, "accepted close must never be resubmitted");
            self.attempts[id] += 1;
            if self.fail_submit {
                return Err(IpcError::Status(99));
            }
            if self.rejects[id] > 0 {
                self.rejects[id] -= 1;
                return Err(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL));
            }
            self.accepted[id] += 1;
            owner.accepted = true;
            owner.pending = Some(Pending(id, self.log.clone()));
            Ok(())
        }

        fn bind(&mut self, owner: &mut Owner, cq: u32) -> Result<(), IpcError> {
            assert_eq!(cq, 7);
            assert!(owner.accepted);
            if self.fail_bind {
                Err(IpcError::Status(98))
            } else {
                Ok(())
            }
        }

        fn poll(&mut self, owner: &mut Owner) -> Result<bool, SocketError> {
            let complete = self.replies[owner.socket.0].pop_front().unwrap_or(Ok(false))?;
            if complete {
                drop(owner.pending.take());
            }
            Ok(complete)
        }

        fn abort(&mut self, owner: &Owner, error: Error) -> ! {
            let id = owner.socket.0;
            assert!(!self.log.borrow().iter().any(|(_, dropped)| *dropped == id));
            assert_eq!(owner.pending.is_some(), owner.accepted);
            panic!("close rejected with owner live: {error:?}")
        }
    }
    fn setup() -> Machine<Fake> {
        Machine::new(
            Fake {
                now: Rc::new(Cell::new(0)),
                log: Rc::new(RefCell::new(Vec::new())),
                rejects: [0; MAX_CLIENTS],
                attempts: [0; MAX_CLIENTS],
                accepted: [0; MAX_CLIENTS],
                fail_bind: false,
                fail_submit: false,
                replies: core::array::from_fn(|_| VecDeque::new()),
            },
            7,
        )
    }
    fn start(m: &mut Machine<Fake>, id: usize) {
        m.start(Socket(id, m.calls.log.clone())).unwrap();
    }
    fn rejected(f: impl FnOnce()) {
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err());
    }
    #[test]
    fn held_close_keeps_its_position_while_another_close_completes() {
        let mut m = setup();
        start(&mut m, 0);
        start(&mut m, 1);
        for _ in 0..10 {
            assert!(!m.advance());
        }
        m.calls.now.set(1234);
        m.calls.replies[1].push_back(Ok(true));
        assert!(m.advance());
        assert_eq!(m.occupied_slots(), 1);
        assert_eq!(m.remaining_ms(), Some(CLOSE_MS - 1234));
        assert_eq!(&*m.calls.log.borrow(), &[("pending", 1), ("socket", 1)]);
        assert_eq!(m.calls.attempts[..2], [1, 1]);
    }
    #[test]
    fn queue_full_retries_only_unstarted_admission_without_budget_reset() {
        let mut m = setup();
        m.calls.rejects[0] = 2;
        start(&mut m, 0);
        assert_eq!(m.remaining_ms(), Some(ADMISSION_BACKOFF_MS));
        m.calls.now.set(49);
        assert!(!m.advance());
        assert_eq!(m.calls.attempts[0], 1);
        m.calls.now.set(50);
        assert!(m.advance());
        assert_eq!(m.calls.attempts[0], 2);
        m.calls.now.set(100);
        assert!(m.advance());
        assert_eq!(m.calls.attempts[0], 3);
        assert_eq!(m.calls.accepted[0], 1);
        assert_eq!(m.remaining_ms(), Some(CLOSE_MS - 100));
        for _ in 0..10 {
            assert!(!m.advance());
        }
        m.calls.replies[0].push_back(Ok(true));
        assert!(m.advance());
        assert_eq!(m.calls.attempts[0], 3);
        assert_eq!(m.occupied_slots(), 0);
    }
    #[test]
    fn queued_admission_does_not_hide_healthy_completion() {
        let mut m = setup();
        m.calls.rejects[0] = 100;
        start(&mut m, 0);
        start(&mut m, 1);
        m.calls.replies[1].push_back(Ok(true));
        assert!(m.advance());
        assert_eq!(m.calls.accepted[0], 0);
        assert_eq!(m.occupied_slots(), 1);
    }
    #[test]
    fn accepted_timeout_aborts_with_owner_live_without_replay() {
        let mut m = setup();
        start(&mut m, 0);
        m.calls.now.set(CLOSE_MS);
        rejected(|| {
            m.advance();
        });
        assert_eq!(m.calls.attempts[0], 1);
        assert_eq!(m.occupied_slots(), 1);
        assert!(m.calls.log.borrow().is_empty());
    }
    #[test]
    fn admission_backoff_deadline_is_terminal_with_unstarted_owner_retained() {
        let mut m = setup();
        m.calls.rejects[0] = usize::MAX;
        start(&mut m, 0);
        m.calls.now.set(CLOSE_MS);
        rejected(|| {
            m.advance();
        });
        assert_eq!(m.calls.attempts[0], 1);
        assert_eq!(m.calls.accepted[0], 0);
        assert_eq!(m.occupied_slots(), 1);
        assert!(m.calls.log.borrow().is_empty());
    }
    #[test]
    fn binding_and_non_queue_submission_rejection_keep_complete_owner() {
        for binding in [false, true] {
            let mut m = setup();
            m.calls.fail_bind = binding;
            m.calls.fail_submit = !binding;
            rejected(|| start(&mut m, 0));
            assert_eq!(m.occupied_slots(), 1);
            assert_eq!(m.calls.accepted[0], usize::from(binding));
            assert!(m.calls.log.borrow().is_empty());
        }
    }
    #[test]
    fn observation_and_malformed_result_failure_do_not_cancel_or_resubmit_close() {
        for error in [
            SocketError::Ipc(IpcError::Status(99)),
            SocketError::Service(-1),
            SocketError::InvalidReply,
        ] {
            let mut m = setup();
            start(&mut m, 0);
            m.calls.replies[0].push_back(Err(error));
            rejected(|| {
                m.advance();
            });
            assert_eq!(m.calls.accepted[0], 1);
            assert_eq!(m.occupied_slots(), 1);
            assert!(m.calls.log.borrow().is_empty());
        }
    }
    #[test]
    fn close_capacity_is_not_returned_before_confirmation() {
        let mut m = setup();
        for id in 0..MAX_CLIENTS {
            start(&mut m, id);
        }
        let socket = m.start(Socket(0, m.calls.log.clone())).unwrap_err();
        assert_eq!(m.occupied_slots(), MAX_CLIENTS);
        drop(socket);
        m.calls.replies[2].push_back(Ok(true));
        assert!(m.advance());
        assert_eq!(m.occupied_slots(), MAX_CLIENTS - 1);
    }
}
