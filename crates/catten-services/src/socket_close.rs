//! Synchronous driver for the same consuming socket-close operation as reactors.
use catten_rt::owned::IpcError;

use crate::{
    deadline::Deadline,
    socket::{
        ClosingSocket,
        SocketError,
    },
};

pub(crate) const CLOSE_MS: u64 = 5_000;
const ADMISSION_MS: u64 = 1_000;
pub(crate) const ADMISSION_BACKOFF_MS: u64 = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    Deadline,
    Observation(SocketError),
    Reentry,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Unsubmitted,
    Accepted,
    Confirmed,
    Retained,
}
trait Calls {
    type Owner;
    fn now(&self) -> (u64, u64);
    fn submit(&mut self, owner: &mut Self::Owner) -> Result<(), IpcError>;
    fn pause(&mut self, ms: u64);
    fn wait(&mut self, owner: &mut Self::Owner, remaining_ms: u64) -> Result<bool, SocketError>;
    fn abort(&mut self, owner: &Self::Owner, failure: Failure) -> !;
}
struct Blocking<C: Calls> {
    owner: C::Owner,
    deadline: Deadline,
    admission: Deadline,
    calls: C,
    phase: Phase,
}
impl<C: Calls> Blocking<C> {
    fn new(calls: C, owner: C::Owner) -> Self {
        let (now, frequency) = calls.now();
        Self {
            owner,
            deadline: Deadline::at(now, frequency, CLOSE_MS),
            admission: Deadline::at(now, frequency, ADMISSION_MS),
            calls,
            phase: Phase::Unsubmitted,
        }
    }

    fn abort(&mut self, failure: Failure) -> ! {
        self.phase = Phase::Retained;
        self.calls.abort(&self.owner, failure)
    }

    fn run(&mut self) -> Result<(), IpcError> {
        if self.phase != Phase::Unsubmitted {
            self.abort(Failure::Reentry);
        }
        loop {
            let (now, _) = self.calls.now();
            if self.deadline.expired_at(now) {
                self.abort(Failure::Deadline);
            }
            match self.calls.submit(&mut self.owner) {
                Ok(()) => {
                    self.phase = Phase::Accepted;
                    break;
                }
                Err(error) => {
                    let (now, _) = self.calls.now();
                    if error != IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL)
                        || self.admission.expired_at(now)
                        || self.deadline.expired_at(now)
                    {
                        self.phase = Phase::Retained;
                        return Err(error);
                    }
                    // There is no queue-admission readiness source. Only an
                    // explicitly rejected submission takes this bounded backoff.
                    let delay = ADMISSION_BACKOFF_MS
                        .min(self.admission.remaining_ms_at(now))
                        .min(self.deadline.remaining_ms_at(now));
                    self.calls.pause(delay);
                }
            }
        }
        let remaining = self.deadline.remaining_ms_at(self.calls.now().0);
        if remaining == 0 {
            self.abort(Failure::Deadline);
        }
        match self.calls.wait(&mut self.owner, remaining) {
            Ok(true) => {
                self.phase = Phase::Confirmed;
                Ok(())
            }
            Ok(false) => self.abort(Failure::Deadline),
            Err(error) => self.abort(Failure::Observation(error)),
        }
    }
}
struct Runtime;
impl<'a> Calls for (Runtime, core::marker::PhantomData<&'a ()>) {
    type Owner = ClosingSocket<'a>;

    fn now(&self) -> (u64, u64) {
        catten_syscall::monotonic_clock()
    }

    fn submit(&mut self, owner: &mut Self::Owner) -> Result<(), IpcError> {
        owner.submit()
    }

    fn pause(&mut self, ms: u64) {
        crate::sleep_ms(ms);
    }

    fn wait(&mut self, owner: &mut Self::Owner, remaining_ms: u64) -> Result<bool, SocketError> {
        owner.wait_timeout(remaining_ms).map(|receipt| receipt.is_some())
    }

    fn abort(&mut self, _owner: &Self::Owner, failure: Failure) -> ! {
        match failure {
            Failure::Observation(error) => {
                catten_rt::logln!("[socket] close observation retained through abort: {error:?}")
            }
            other => catten_rt::logln!("[socket] close retained through abort: {other:?}"),
        }
        catten_rt::domain_abort()
    }
}
/// Both explicit close and ordinary socket Drop consume their lease here once.
/// No field destructor can issue a second call after this driver reports error.
pub(crate) fn finish(owner: ClosingSocket<'_>) -> Result<(), IpcError> {
    finish_transaction(Blocking::new((Runtime, core::marker::PhantomData), owner))
}

fn finish_transaction<C: Calls>(mut transaction: Blocking<C>) -> Result<(), IpcError> {
    let result = transaction.run();
    if result.is_err() {
        // run returns only before admission. Keep the complete terminal owner
        // inert instead of invoking incomplete-close Drop or recreating its ID.
        // TCP/IP retains any remote record and its original accounting; this
        // is no custody/retry owner, cleanup proof, or local/remote charge refund.
        core::mem::forget(transaction);
    }
    result
}

#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::{
        collections::VecDeque,
        rc::Rc,
        vec,
        vec::Vec,
    };
    use core::cell::{
        Cell,
        RefCell,
    };

    use super::*;
    type Log = Rc<RefCell<Vec<&'static str>>>;
    struct Socket(Log);
    impl Drop for Socket {
        fn drop(&mut self) {
            self.0.borrow_mut().push("socket");
        }
    }
    struct Pending(Log);
    impl Drop for Pending {
        fn drop(&mut self) {
            self.0.borrow_mut().push("pending");
        }
    }
    struct Owner {
        pending: Option<Pending>,
        socket: Option<Socket>,
    }
    struct Fake {
        now: Rc<Cell<u64>>,
        frequency: u64,
        log: Log,
        attempts: Rc<Cell<usize>>,
        accepted: usize,
        rejections: VecDeque<IpcError>,
        submit_ms: u64,
        pauses: Vec<u64>,
        waits: Vec<u64>,
        outcome: Result<bool, SocketError>,
        failures: Vec<Failure>,
    }
    impl Calls for Fake {
        type Owner = Owner;

        fn now(&self) -> (u64, u64) {
            (self.now.get(), self.frequency)
        }

        fn submit(&mut self, owner: &mut Owner) -> Result<(), IpcError> {
            assert!(owner.socket.is_some());
            assert!(owner.pending.is_none());
            assert_eq!(self.accepted, 0, "an accepted close cannot be resubmitted");
            self.attempts.set(self.attempts.get() + 1);
            self.now.set(self.now.get() + self.submit_ms);
            if let Some(error) = self.rejections.pop_front() {
                return Err(error);
            }
            self.accepted += 1;
            owner.pending = Some(Pending(self.log.clone()));
            Ok(())
        }

        fn pause(&mut self, ms: u64) {
            assert!(ms > 0);
            self.pauses.push(ms);
            self.now.set(self.now.get() + ms);
        }

        fn wait(&mut self, owner: &mut Owner, ms: u64) -> Result<bool, SocketError> {
            assert!(owner.pending.is_some());
            self.waits.push(ms);
            if self.outcome == Ok(true) {
                drop(owner.pending.take());
                drop(owner.socket.take());
            }
            self.outcome
        }

        fn abort(&mut self, owner: &Owner, failure: Failure) -> ! {
            assert!(owner.socket.is_some());
            assert!(self.log.borrow().is_empty());
            assert_eq!(owner.pending.is_some(), self.accepted == 1);
            self.failures.push(failure);
            panic!("close abort with complete owner: {failure:?}")
        }
    }
    fn setup() -> Blocking<Fake> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let owner = Owner {
            pending: None,
            socket: Some(Socket(log.clone())),
        };
        Blocking::new(
            Fake {
                now: Rc::new(Cell::new(0)),
                frequency: 1000,
                log,
                attempts: Rc::new(Cell::new(0)),
                accepted: 0,
                rejections: VecDeque::new(),
                submit_ms: 0,
                pauses: Vec::new(),
                waits: Vec::new(),
                outcome: Ok(true),
                failures: Vec::new(),
            },
            owner,
        )
    }
    fn rejected(f: impl FnOnce()) {
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err());
    }
    fn queue_full() -> IpcError {
        IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL)
    }
    #[test]
    fn success_submits_once_and_disposes_pending_before_remote_lease() {
        let mut transaction = setup();
        assert_eq!(transaction.run(), Ok(()));
        assert_eq!(transaction.calls.attempts.get(), 1);
        assert_eq!(transaction.calls.waits, vec![CLOSE_MS]);
        assert_eq!(&*transaction.calls.log.borrow(), &["pending", "socket"]);
        assert_eq!(transaction.phase, Phase::Confirmed);
    }
    #[test]
    fn queue_backoff_carries_remaining_budget_into_one_owned_wait() {
        let mut transaction = setup();
        transaction.calls.rejections.extend([queue_full(); 2]);
        assert_eq!(transaction.run(), Ok(()));
        assert_eq!(transaction.calls.attempts.get(), 3);
        assert_eq!(transaction.calls.pauses, vec![50, 50]);
        assert_eq!(transaction.calls.waits, vec![CLOSE_MS - 100]);
    }
    #[test]
    fn admission_window_stops_without_a_second_destructor_attempt() {
        let mut transaction = setup();
        transaction.calls.rejections.extend([queue_full(); 100]);
        let attempts = transaction.calls.attempts.clone();
        let log = transaction.calls.log.clone();
        assert_eq!(finish_transaction(transaction), Err(queue_full()));
        assert_eq!(attempts.get(), 21);
        assert!(log.borrow().is_empty());
    }
    #[test]
    fn non_queue_rejection_returns_once_with_terminal_unsubmitted_retention() {
        let mut transaction = setup();
        let error = IpcError::Status(77);
        transaction.calls.rejections.push_back(error);
        let attempts = transaction.calls.attempts.clone();
        let log = transaction.calls.log.clone();
        assert_eq!(finish_transaction(transaction), Err(error));
        assert_eq!(attempts.get(), 1);
        assert!(log.borrow().is_empty());
    }
    #[test]
    fn admission_pause_is_clipped_to_original_window() {
        let mut transaction = setup();
        transaction.calls.submit_ms = 980;
        transaction.calls.rejections.extend([queue_full(); 2]);
        assert_eq!(transaction.run(), Err(queue_full()));
        assert_eq!(transaction.calls.pauses, vec![20]);
        assert!(transaction.calls.waits.is_empty());
        assert_eq!(transaction.phase, Phase::Retained);
    }
    #[test]
    fn accepted_timeout_aborts_without_cancel_reconstruction_or_new_budget() {
        let mut transaction = setup();
        transaction.calls.outcome = Ok(false);
        rejected(|| {
            transaction.run().unwrap();
        });
        assert_eq!(transaction.calls.failures, vec![Failure::Deadline]);
        assert_eq!(transaction.calls.attempts.get(), 1);
        assert_eq!(transaction.phase, Phase::Retained);
        rejected(|| {
            transaction.run().unwrap();
        });
        assert_eq!(transaction.calls.attempts.get(), 1);
    }
    #[test]
    fn wait_admission_and_observation_failures_remain_distinct_from_timeout() {
        for error in [
            SocketError::Ipc(IpcError::Status(catten_syscall::ipc_status::RESOURCE_LIMIT)),
            SocketError::Ipc(IpcError::Status(99)),
            SocketError::InvalidReply,
            SocketError::Service(-2),
        ] {
            let mut transaction = setup();
            transaction.calls.outcome = Err(error);
            rejected(|| {
                transaction.run().unwrap();
            });
            assert_eq!(transaction.calls.failures, vec![Failure::Observation(error)]);
            assert_eq!(transaction.calls.attempts.get(), 1);
            assert!(transaction.calls.log.borrow().is_empty());
        }
    }
    #[test]
    fn slow_accepted_submission_does_not_restart_the_wait_budget() {
        let mut transaction = setup();
        transaction.calls.submit_ms = CLOSE_MS;
        rejected(|| {
            transaction.run().unwrap();
        });
        assert_eq!(transaction.calls.attempts.get(), 1);
        assert!(transaction.calls.waits.is_empty());
        assert!(transaction.owner.pending.is_some());
    }
    #[test]
    fn rejected_slow_submission_never_becomes_accepted_uncertainty() {
        let mut transaction = setup();
        transaction.calls.submit_ms = CLOSE_MS;
        transaction.calls.rejections.push_back(queue_full());
        assert_eq!(transaction.run(), Err(queue_full()));
        assert!(transaction.owner.pending.is_none());
        assert!(transaction.calls.waits.is_empty());
        assert_eq!(transaction.phase, Phase::Retained);
    }
    #[test]
    fn rejected_operation_cannot_be_reentered_by_fallback() {
        let mut transaction = setup();
        transaction.calls.rejections.push_back(IpcError::Status(77));
        assert!(transaction.run().is_err());
        rejected(|| {
            transaction.run().unwrap();
        });
        assert_eq!(transaction.calls.attempts.get(), 1);
        assert_eq!(transaction.calls.failures, vec![Failure::Reentry]);
    }
}
