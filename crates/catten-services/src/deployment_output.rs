//! Owning, CQ-bound deployment response sends with confirmed prefix receipts.
use catten_rt::owned::{
    CallResult,
    IpcError,
    PendingCall,
};

use crate::{
    deadline::Deadline,
    deployment_control::Response,
    deployment_input::MAX_CLIENTS,
    socket::{
        OwnedSocket,
        SocketError,
    },
    socket_send::MAX_PAYLOAD,
};

pub const OUTPUT_MS: u64 = 5_000;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum Error {
    Submission(SocketError),
    Observation(IpcError),
    Service(i64),
    InvalidReply,
    Deadline,
    Capacity,
}

trait Calls {
    type Socket;
    type Pending;
    type Reply;
    fn now(&self) -> u64;
    fn submit(
        &mut self,
        socket: &Self::Socket,
        bytes: &[u8],
        deadline: &Deadline,
    ) -> Result<Self::Pending, Error>;
    fn bind(&mut self, pending: &mut Self::Pending, cq: u32) -> Result<(), ()>;
    fn abort(&mut self, pending: &Self::Pending, socket: &Self::Socket) -> !;
    fn poll(&mut self, pending: &mut Self::Pending) -> Result<Option<Self::Reply>, Error>;
    fn receipt(reply: Self::Reply, submitted: usize) -> Result<usize, Error>;
}

struct Output<P, S> {
    // Pending owns a moved copy, so no borrow of response survives submission.
    // Cancel/observe that exact call before releasing the socket owner.
    pending: Option<P>,
    socket: S,
    response: Response,
    offset: usize,
    submitted: usize,
    deadline: Deadline,
}

type Finished<S> = (S, Result<(), Error>);
struct Machine<C: Calls> {
    outputs: [Option<Output<C::Pending, C::Socket>>; MAX_CLIENTS],
    calls: C,
    cursor: usize,
    cq: u32,
}
impl<C: Calls> Machine<C> {
    fn new(calls: C, cq: u32) -> Self {
        Self {
            outputs: core::array::from_fn(|_| None),
            calls,
            cursor: 0,
            cq,
        }
    }

    fn finish(&mut self, index: usize, result: Result<(), Error>) -> Finished<C::Socket> {
        let Output {
            pending,
            socket,
            ..
        } = self.outputs[index].take().unwrap();
        drop(pending);
        (socket, result)
    }

    fn arm(&mut self, index: usize) -> Result<(), Error> {
        let owner = self.outputs[index].as_mut().unwrap();
        if owner.deadline.expired_at(self.calls.now()) {
            return Err(Error::Deadline);
        }
        let end = owner.response.len().min(owner.offset + MAX_PAYLOAD);
        let bytes = &owner.response.as_bytes()[owner.offset..end];
        owner.submitted = bytes.len();
        owner.pending = Some(self.calls.submit(&owner.socket, bytes, &owner.deadline)?);
        if self.calls.bind(owner.pending.as_mut().unwrap(), self.cq).is_err() {
            self.calls.abort(owner.pending.as_ref().unwrap(), &owner.socket);
        }
        Ok(())
    }

    fn start(
        &mut self,
        socket: C::Socket,
        response: Response,
        now: u64,
        frequency: u64,
    ) -> Option<Finished<C::Socket>> {
        let Some(index) = self.outputs.iter().position(Option::is_none) else {
            return Some((socket, Err(Error::Capacity)));
        };
        let retained = match &response {
            alloc::borrow::Cow::Borrowed(bytes) => bytes.len(),
            alloc::borrow::Cow::Owned(bytes) => bytes.capacity(),
        };
        if retained > MAX_RESPONSE_BYTES {
            return Some((socket, Err(Error::Capacity)));
        }
        if response.is_empty() {
            return Some((socket, Ok(())));
        }
        self.outputs[index] = Some(Output {
            pending: None,
            socket,
            response,
            offset: 0,
            submitted: 0,
            deadline: Deadline::at(now, frequency, OUTPUT_MS),
        });
        match self.arm(index) {
            Ok(()) => None,
            Err(error) => Some(self.finish(index, Err(error))),
        }
    }

    fn advance(&mut self) -> (bool, Option<Finished<C::Socket>>) {
        let now = self.calls.now();
        for offset in 0..MAX_CLIENTS {
            let index = (self.cursor + offset) % MAX_CLIENTS;
            let Some(owner) = self.outputs[index].as_mut() else {
                continue;
            };
            let result = if owner.deadline.expired_at(now) {
                Err(Error::Deadline)
            } else {
                match self.calls.poll(owner.pending.as_mut().expect("accepted send")) {
                    Ok(None) => continue,
                    Ok(Some(reply)) => {
                        drop(owner.pending.take());
                        C::receipt(reply, owner.submitted)
                    }
                    Err(error) => Err(error),
                }
            };
            self.cursor = (index + 1) % MAX_CLIENTS;
            let count = match result {
                Ok(count) => count,
                Err(error) => return (true, Some(self.finish(index, Err(error)))),
            };
            let owner = self.outputs[index].as_mut().unwrap();
            owner.offset += count;
            if owner.offset == owner.response.len() {
                return (true, Some(self.finish(index, Ok(()))));
            }
            return match self.arm(index) {
                Ok(()) => (true, None),
                Err(error) => (true, Some(self.finish(index, Err(error)))),
            };
        }
        (false, None)
    }

    fn remaining_ms(&self) -> Option<u64> {
        let now = self.calls.now();
        self.outputs.iter().flatten().map(|owner| owner.deadline.remaining_ms_at(now)).min()
    }

    fn occupied_slots(&self) -> usize {
        self.outputs.iter().flatten().count()
    }
}

struct Runtime<'a>(core::marker::PhantomData<&'a ()>);
impl<'a> Calls for Runtime<'a> {
    type Pending = PendingCall<'static>;
    type Reply = CallResult;
    type Socket = OwnedSocket<'a>;

    fn now(&self) -> u64 {
        catten_syscall::monotonic_clock().0
    }

    fn submit(
        &mut self,
        socket: &Self::Socket,
        bytes: &[u8],
        deadline: &Deadline,
    ) -> Result<Self::Pending, Error> {
        socket.send_call(bytes, deadline).map_err(|error| match error {
            SocketError::RetryExhausted => Error::Deadline,
            other => Error::Submission(other),
        })
    }

    fn bind(&mut self, pending: &mut Self::Pending, cq: u32) -> Result<(), ()> {
        pending.bind_cq(cq).map_err(|_| ())
    }

    fn abort(&mut self, _pending: &Self::Pending, _socket: &Self::Socket) -> ! {
        catten_rt::domain_abort()
    }

    fn poll(&mut self, pending: &mut Self::Pending) -> Result<Option<Self::Reply>, Error> {
        pending.poll().map_err(Error::Observation)
    }

    fn receipt(reply: Self::Reply, submitted: usize) -> Result<usize, Error> {
        if reply.memory.is_some() || reply.connection.is_some() {
            return Err(Error::InvalidReply);
        }
        if reply.result < 0 {
            return Err(Error::Service(reply.result));
        }
        let count = usize::try_from(reply.result).map_err(|_| Error::InvalidReply)?;
        if count == 0 || count > submitted {
            return Err(Error::InvalidReply);
        }
        Ok(count)
    }
}

/// Output is one of the service's four active phases. Each response retains its
/// original deadline, pending moved page, socket and complete remaining bytes.
/// Only a positive receipt permits submitting a remainder. Error/timeout ends
/// this output without replay; explicit logical socket close remains separate.
pub struct Outputs<'a>(Machine<Runtime<'a>>);
impl<'a> Outputs<'a> {
    pub fn new(cq: u32) -> Self {
        Self(Machine::new(Runtime(core::marker::PhantomData), cq))
    }

    pub fn start(
        &mut self,
        socket: OwnedSocket<'a>,
        response: Response,
    ) -> Option<Finished<OwnedSocket<'a>>> {
        let (now, frequency) = catten_syscall::monotonic_clock();
        self.0.start(socket, response, now, frequency)
    }

    pub fn advance(&mut self) -> (bool, Option<Finished<OwnedSocket<'a>>>) {
        self.0.advance()
    }

    pub fn remaining_ms(&self) -> Option<u64> {
        self.0.remaining_ms()
    }

    pub fn occupied_slots(&self) -> usize {
        self.0.occupied_slots()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::{
        borrow::Cow,
        collections::VecDeque,
        rc::Rc,
        string::String,
        vec,
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
    struct Fake {
        now: Rc<Cell<u64>>,
        log: Log,
        reject: bool,
        reject_bind: bool,
        submissions: Vec<(usize, Vec<u8>, u64)>,
        replies: [VecDeque<Result<i64, Error>>; MAX_CLIENTS],
    }
    impl Calls for Fake {
        type Pending = Pending;
        type Reply = i64;
        type Socket = Socket;

        fn now(&self) -> u64 {
            self.now.get()
        }

        fn submit(
            &mut self,
            socket: &Socket,
            bytes: &[u8],
            deadline: &Deadline,
        ) -> Result<Pending, Error> {
            if self.reject {
                return Err(Error::Capacity);
            }
            self.submissions.push((socket.0, bytes.to_vec(), deadline.remaining_ms_at(self.now())));
            Ok(Pending(socket.0, self.log.clone()))
        }

        fn bind(&mut self, _: &mut Pending, cq: u32) -> Result<(), ()> {
            assert_eq!(cq, 7);
            if self.reject_bind {
                Err(())
            } else {
                Ok(())
            }
        }

        fn abort(&mut self, pending: &Pending, socket: &Socket) -> ! {
            assert_eq!(pending.0, socket.0);
            assert!(self.log.borrow().is_empty());
            panic!("send bind rejected with both owners live")
        }

        fn poll(&mut self, pending: &mut Pending) -> Result<Option<i64>, Error> {
            self.replies[pending.0].pop_front().transpose()
        }

        fn receipt(result: i64, submitted: usize) -> Result<usize, Error> {
            if result < 0 {
                return Err(Error::Service(result));
            }
            if result == 0 || result as usize > submitted {
                return Err(Error::InvalidReply);
            }
            Ok(result as usize)
        }
    }
    fn setup() -> Machine<Fake> {
        Machine::new(
            Fake {
                now: Rc::new(Cell::new(0)),
                log: Rc::new(RefCell::new(Vec::new())),
                reject: false,
                reject_bind: false,
                submissions: Vec::new(),
                replies: core::array::from_fn(|_| VecDeque::new()),
            },
            7,
        )
    }
    fn start(m: &mut Machine<Fake>, id: usize, response: Response) -> Option<Finished<Socket>> {
        m.start(Socket(id, m.calls.log.clone()), response, m.calls.now(), 1000)
    }
    #[test]
    fn blocked_output_does_not_hide_healthy_receipt_or_restart_deadline() {
        let mut m = setup();
        assert!(start(&mut m, 0, Cow::Borrowed("blocked")).is_none());
        assert!(start(&mut m, 1, Cow::Borrowed("healthy")).is_none());
        for _ in 0..10 {
            assert!(!m.advance().0);
        }
        m.calls.now.set(1234);
        m.calls.replies[1].push_back(Ok(7));
        let (socket, result) = m.advance().1.unwrap();
        assert_eq!(socket.0, 1);
        assert!(result.is_ok());
        drop(socket);
        assert_eq!(m.remaining_ms(), Some(OUTPUT_MS - 1234));
        m.calls.now.set(OUTPUT_MS);
        let (socket, result) = m.advance().1.unwrap();
        assert_eq!(socket.0, 0);
        assert!(matches!(result, Err(Error::Deadline)));
        drop(socket);
        assert_eq!(m.calls.submissions.len(), 2);
        assert_eq!(m.occupied_slots(), 0);
    }
    #[test]
    fn only_confirmed_prefix_advances_and_remainder_keeps_original_budget() {
        let mut m = setup();
        let body = "a".repeat(MAX_PAYLOAD + 3);
        assert!(start(&mut m, 0, Cow::Owned(body)).is_none());
        assert_eq!(m.calls.submissions[0].1.len(), MAX_PAYLOAD);
        m.calls.now.set(4000);
        m.calls.replies[0].push_back(Ok((MAX_PAYLOAD - 2) as i64));
        assert!(matches!(m.advance(), (true, None)));
        assert_eq!(m.calls.submissions[1].1, vec![b'a'; 5]);
        assert_eq!(m.calls.submissions[1].2, 1000);
        m.calls.replies[0].push_back(Ok(5));
        assert!(m.advance().1.unwrap().1.is_ok());
        assert_eq!(m.calls.submissions.len(), 2);
    }
    #[test]
    fn uncertainty_service_error_and_invalid_receipts_never_replay() {
        for reply in [Err(Error::Observation(IpcError::Status(99))), Ok(-99), Ok(0), Ok(4)] {
            let mut m = setup();
            start(&mut m, 0, Cow::Borrowed("abc"));
            m.calls.replies[0].push_back(reply);
            let (socket, result) = m.advance().1.unwrap();
            assert!(result.is_err());
            drop(socket);
            assert_eq!(m.calls.submissions.len(), 1);
            assert!(!m.advance().0);
            assert_eq!(&*m.calls.log.borrow(), &[("pending", 0), ("socket", 0)]);
        }
    }
    #[test]
    fn failed_submission_and_failed_remainder_return_socket_without_replay() {
        let mut m = setup();
        m.calls.reject = true;
        assert!(start(&mut m, 0, Cow::Borrowed("abc")).unwrap().1.is_err());
        assert_eq!(m.occupied_slots(), 0);
        assert!(m.calls.submissions.is_empty());
        m.calls.reject = false;
        start(&mut m, 1, Cow::Borrowed("abc"));
        m.calls.replies[1].push_back(Ok(1));
        m.calls.reject = true;
        assert!(m.advance().1.unwrap().1.is_err());
        assert_eq!(m.calls.submissions.len(), 1);
        assert_eq!(m.occupied_slots(), 0);
    }
    #[test]
    fn capacity_accounts_retained_string_and_fixed_slots() {
        let mut m = setup();
        let mut huge = String::with_capacity(MAX_RESPONSE_BYTES + 1);
        huge.push('x');
        assert!(matches!(start(&mut m, 0, Cow::Owned(huge)).unwrap().1, Err(Error::Capacity)));
        assert!(start(&mut m, 0, Cow::Borrowed("")).unwrap().1.is_ok());
        for id in 0..MAX_CLIENTS {
            assert!(start(&mut m, id, Cow::Borrowed("a")).is_none());
        }
        assert!(matches!(start(&mut m, 0, Cow::Borrowed("a")).unwrap().1, Err(Error::Capacity)));
        assert_eq!(m.calls.submissions.len(), MAX_CLIENTS);
    }
    #[test]
    fn dropping_output_cancels_pending_before_socket() {
        let mut m = setup();
        let log = m.calls.log.clone();
        start(&mut m, 0, Cow::Borrowed("abc"));
        drop(m);
        assert_eq!(&*log.borrow(), &[("pending", 0), ("socket", 0)]);
    }
    #[test]
    fn accepted_binding_rejection_reaches_abort_with_owners_live() {
        let mut m = setup();
        m.calls.reject_bind = true;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            start(&mut m, 0, Cow::Borrowed("abc"));
        }));
        assert!(result.is_err());
        assert!(m.calls.log.borrow().is_empty());
        assert_eq!(m.occupied_slots(), 1);
    }
    #[test]
    fn ready_outputs_rotate_between_partial_receipts() {
        let mut m = setup();
        for id in 0..2 {
            start(&mut m, id, Cow::Borrowed("abc"));
            m.calls.replies[id].extend([Ok(1), Ok(2)]);
        }
        for _ in 0..4 {
            assert!(m.advance().0);
        }
        let ids: Vec<_> = m.calls.submissions.iter().map(|row| row.0).collect();
        assert_eq!(ids, vec![0, 1, 0, 1]);
        assert_eq!(m.occupied_slots(), 0);
    }
}
