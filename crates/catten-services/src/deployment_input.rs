//! Bounded deployment request inputs sharing one completion queue.
//!
//! Controller work and response transmission are separate from this input pool.
use crate::{
    deadline::Deadline,
    deployment_http::{
        Error,
        MAX_REQUEST_BYTES,
        Request,
        RequestBuffer,
    },
    management_io::{
        Advance,
        Connection as OwnedSocket,
    },
};

pub const MAX_CLIENTS: usize = 4;
pub const MAX_BUFFERED_BYTES: usize = MAX_REQUEST_BYTES;
pub const HEADER_MS: u64 = 5_000;
pub const REQUEST_MS: u64 = 30_000;

trait Calls {
    type Socket;
    type Pending;
    type Reply;
    fn receive(&mut self, socket: &mut Self::Socket, cq: u32) -> Result<Self::Pending, Error>;
    fn poll(
        &mut self,
        socket: &mut Self::Socket,
        pending: &mut Self::Pending,
        deadline: &Deadline,
    ) -> Result<Advance<Self::Reply>, Error>;
    fn remaining(_socket: &Self::Socket) -> u64 {
        u64::MAX
    }
    fn append(
        socket: &Self::Socket,
        reply: Self::Reply,
        buffer: &mut RequestBuffer,
        available: usize,
    ) -> Result<Option<Request>, Error>;
}

struct Input<S, P> {
    // Cancel the accepted receive before releasing its remote socket owner.
    pending: Option<P>,
    socket: S,
    buffer: RequestBuffer,
    header: Deadline,
    total: Deadline,
}

impl<S, P> Input<S, P> {
    fn remaining(&self, now: u64) -> u64 {
        let total = self.total.remaining_ms_at(now);
        if self.buffer.has_header() {
            total
        } else {
            total.min(self.header.remaining_ms_at(now))
        }
    }
}

type Finished<S> = (S, Result<Request, Error>);

struct Machine<C: Calls> {
    inputs: [Option<Input<C::Socket, C::Pending>>; MAX_CLIENTS],
    calls: C,
    cursor: usize,
    cq: u32,
    byte_limit: usize,
}

impl<C: Calls> Machine<C> {
    fn new(calls: C, cq: u32) -> Self {
        Self {
            inputs: core::array::from_fn(|_| None),
            calls,
            cursor: 0,
            cq,
            byte_limit: MAX_BUFFERED_BYTES,
        }
    }

    fn has_capacity(&self) -> bool {
        self.inputs.iter().any(Option::is_none)
    }

    fn admit(
        &mut self,
        socket: C::Socket,
        now: u64,
        frequency: u64,
    ) -> Result<(), (C::Socket, Error)> {
        let Some(slot) = self.inputs.iter_mut().find(|slot| slot.is_none()) else {
            return Err((socket, Error::AllocationFailed));
        };
        let mut input = Input {
            pending: None,
            socket,
            buffer: RequestBuffer::default(),
            header: Deadline::at(now, frequency, HEADER_MS),
            total: Deadline::at(now, frequency, REQUEST_MS),
        };
        match self.calls.receive(&mut input.socket, self.cq) {
            Ok(pending) => input.pending = Some(pending),
            Err(error) => return Err((input.socket, error)),
        }
        *slot = Some(input);
        Ok(())
    }

    /// Advance at most one ready input per turn. A readable peer moves behind
    /// every other slot even when it immediately supplies another full page.
    fn advance(&mut self, now: u64, reserved: usize) -> (bool, Option<Finished<C::Socket>>) {
        let used: usize = self.inputs.iter().flatten().map(|i| i.buffer.buffered_len()).sum();
        let used = used.saturating_add(reserved);
        for offset in 0..MAX_CLIENTS {
            let index = (self.cursor + offset) % MAX_CLIENTS;
            let Some(input) = self.inputs[index].as_mut() else {
                continue;
            };
            let result = if input.remaining(now) == 0 {
                Err(Error::Malformed)
            } else {
                let deadline = if input.buffer.has_header() {
                    input.total
                } else {
                    input.header
                };
                match self.calls.poll(
                    &mut input.socket,
                    input.pending.as_mut().expect("admitted receive"),
                    &deadline,
                ) {
                    Ok(Advance::Waiting) => continue,
                    Ok(Advance::Progress) => {
                        self.cursor = (index + 1) % MAX_CLIENTS;
                        return (true, None);
                    }
                    Err(error) => Err(error),
                    Ok(Advance::Ready(reply)) => {
                        drop(input.pending.take());
                        C::append(
                            &input.socket,
                            reply,
                            &mut input.buffer,
                            self.byte_limit.saturating_sub(used),
                        )
                    }
                }
            };
            self.cursor = (index + 1) % MAX_CLIENTS;
            let result = match result {
                Ok(Some(request)) => Ok(request),
                Ok(None) => match self.calls.receive(&mut input.socket, self.cq) {
                    Ok(pending) => {
                        input.pending = Some(pending);
                        return (true, None);
                    }
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            };
            let Input {
                pending,
                socket,
                ..
            } = self.inputs[index].take().unwrap();
            drop(pending);
            return (true, Some((socket, result)));
        }
        (false, None)
    }

    fn remaining(&self, now: u64) -> Option<u64> {
        self.inputs.iter().flatten().map(|i| i.remaining(now).min(C::remaining(&i.socket))).min()
    }
}

struct Runtime<'a>(core::marker::PhantomData<&'a ()>);

impl<'a> Calls for Runtime<'a> {
    type Pending = ();
    type Reply = (charlotte_management_tls::Identity, zeroize::Zeroizing<alloc::vec::Vec<u8>>);
    type Socket = OwnedSocket<'a>;

    fn receive(&mut self, socket: &mut Self::Socket, cq: u32) -> Result<(), Error> {
        socket.begin_receive(cq).map_err(|_| Error::Malformed)
    }

    fn poll(
        &mut self,
        socket: &mut Self::Socket,
        _: &mut (),
        deadline: &Deadline,
    ) -> Result<Advance<Self::Reply>, Error> {
        socket.poll_receive(deadline).map_err(|_| Error::Malformed)
    }

    fn remaining(socket: &Self::Socket) -> u64 {
        socket.remaining_ms()
    }

    fn append(
        socket: &Self::Socket,
        (identity, bytes): Self::Reply,
        buffer: &mut RequestBuffer,
        available: usize,
    ) -> Result<Option<Request>, Error> {
        if bytes.len() > available {
            return Err(Error::AllocationFailed);
        }
        let had_header = buffer.has_header();
        let request = buffer.push(&bytes)?;
        let mutation = buffer.requires_administrator()
            || request.as_ref().is_some_and(Request::requires_administrator);
        let permission = if mutation {
            charlotte_management_tls::Permission::Mutate
        } else {
            charlotte_management_tls::Permission::Observe
        };
        let allowed = identity.allows(permission);
        if request.is_some() || (!had_header && buffer.has_header()) || !allowed {
            socket.audit(identity, permission, allowed);
        }
        if !allowed {
            return Err(Error::Forbidden);
        }
        Ok(request)
    }
}

/// Owns up to four sockets, their accepted receives and original input budgets.
pub struct Inputs<'a>(Machine<Runtime<'a>>);

impl<'a> Inputs<'a> {
    pub fn new(cq: u32) -> Self {
        Self(Machine::new(Runtime(core::marker::PhantomData), cq))
    }

    pub fn has_capacity(&self) -> bool {
        self.0.has_capacity()
    }

    pub fn available_slots(&self) -> usize {
        self.0.inputs.iter().filter(|input| input.is_none()).count()
    }

    /// Start budgets when this reactor observes accept completion.
    // Rejection must return the complete owner without a new fallible allocation.
    #[allow(clippy::result_large_err)]
    pub fn admit(&mut self, socket: OwnedSocket<'a>) -> Result<(), (OwnedSocket<'a>, Error)> {
        let (now, frequency) = catten_syscall::monotonic_clock();
        self.0.admit(socket, now, frequency)
    }

    pub fn advance(&mut self) -> (bool, Option<Finished<OwnedSocket<'a>>>) {
        self.advance_reserved(0)
    }

    pub fn advance_reserved(
        &mut self,
        reserved: usize,
    ) -> (bool, Option<Finished<OwnedSocket<'a>>>) {
        self.0.advance(catten_syscall::monotonic_clock().0, reserved)
    }

    pub fn available_bytes(&self) -> usize {
        let used: usize = self.0.inputs.iter().flatten().map(|i| i.buffer.buffered_len()).sum();
        MAX_BUFFERED_BYTES.saturating_sub(used)
    }

    pub fn remaining_ms(&self) -> Option<u64> {
        self.0.remaining(catten_syscall::monotonic_clock().0)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        collections::VecDeque,
        rc::Rc,
        vec,
        vec::Vec,
    };
    use core::cell::RefCell;

    use super::*;

    type Log = Rc<RefCell<Vec<(&'static str, usize)>>>;
    type Replies = Rc<RefCell<VecDeque<Result<Vec<u8>, Error>>>>;
    #[derive(Debug)]
    struct Socket {
        id: usize,
        log: Log,
    }
    impl Drop for Socket {
        fn drop(&mut self) {
            self.log.borrow_mut().push(("socket", self.id));
        }
    }
    struct Pending {
        id: usize,
        log: Log,
        replies: Replies,
    }
    impl Drop for Pending {
        fn drop(&mut self) {
            self.log.borrow_mut().push(("pending", self.id));
        }
    }
    struct Fake {
        replies: [Replies; MAX_CLIENTS + 1],
        reject: bool,
        log: Log,
    }
    impl Calls for Fake {
        type Pending = Pending;
        type Reply = Result<Vec<u8>, Error>;
        type Socket = Socket;

        fn receive(&mut self, socket: &mut Socket, cq: u32) -> Result<Pending, Error> {
            assert_eq!(cq, 7);
            if core::mem::take(&mut self.reject) {
                return Err(Error::AllocationFailed);
            }
            Ok(Pending {
                id: socket.id,
                log: self.log.clone(),
                replies: self.replies[socket.id].clone(),
            })
        }

        fn poll(
            &mut self,
            _socket: &mut Socket,
            pending: &mut Pending,
            _deadline: &Deadline,
        ) -> Result<Advance<Self::Reply>, Error> {
            Ok(match pending.replies.borrow_mut().pop_front() {
                Some(reply) => Advance::Ready(reply),
                None => Advance::Waiting,
            })
        }

        fn append(
            _socket: &Socket,
            reply: Self::Reply,
            buffer: &mut RequestBuffer,
            available: usize,
        ) -> Result<Option<Request>, Error> {
            let bytes = reply?;
            if bytes.is_empty() {
                return Err(Error::Malformed);
            }
            if bytes.len() > available {
                return Err(Error::AllocationFailed);
            }
            buffer.push(&bytes)
        }
    }
    fn setup() -> (Machine<Fake>, Log) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let machine = Machine::new(
            Fake {
                replies: core::array::from_fn(|_| Rc::new(RefCell::new(VecDeque::new()))),
                reject: false,
                log: log.clone(),
            },
            7,
        );
        (machine, log)
    }
    fn admit(m: &mut Machine<Fake>, id: usize) {
        let socket = Socket {
            id,
            log: m.calls.log.clone(),
        };
        m.admit(socket, 0, 1000).unwrap();
    }
    fn data(m: &mut Machine<Fake>, id: usize, bytes: &[u8]) {
        m.calls.replies[id].borrow_mut().push_back(Ok(bytes.to_vec()));
    }
    const STATUS: &[u8] = b"GET /v1/deployments/healthy HTTP/1.1\r\nHost: test\r\n\r\n";

    #[test]
    fn healthy_input_finishes_with_three_stalled_peers() {
        let (mut m, _) = setup();
        for id in 0..4 {
            admit(&mut m, id);
        }
        data(&mut m, 3, STATUS);
        let (progress, result) = m.advance(150, 0);
        assert!(progress);
        let (socket, request) = result.unwrap();
        assert_eq!(socket.id, 3);
        assert_eq!(request, Ok(Request::Status(b"healthy".to_vec())));
        assert_eq!(m.inputs.iter().flatten().count(), 3);
        assert_eq!(m.remaining(150), Some(4850));
    }

    #[test]
    fn readable_peer_cannot_monopolize_turns() {
        let (mut m, _) = setup();
        admit(&mut m, 0);
        admit(&mut m, 1);
        data(&mut m, 0, b"GET ");
        data(&mut m, 0, b"/v1/");
        data(&mut m, 1, STATUS);
        assert!(matches!(m.advance(1, 0), (true, None)));
        let (_, result) = m.advance(2, 0);
        assert_eq!(result.unwrap().0.id, 1);
        assert_eq!(m.inputs[0].as_ref().unwrap().buffer.buffered_len(), 4);
    }

    #[test]
    fn original_header_and_body_deadlines_survive_progress_and_unrelated_wakes() {
        let (mut m, _) = setup();
        admit(&mut m, 0);
        admit(&mut m, 1);
        data(&mut m, 0, b"GET ");
        data(&mut m, 1, b"POST /v1/operations HTTP/1.1\r\nContent-Length: 3584\r\n\r\nx");
        assert!(matches!(m.advance(4900, 0), (true, None)));
        assert!(matches!(m.advance(4901, 0), (true, None)));
        assert!(matches!(m.advance(4999, 0), (false, None)));
        let (socket, error) = m.advance(5000, 0).1.unwrap();
        assert_eq!(socket.id, 0);
        assert_eq!(error, Err(Error::Malformed));
        assert_eq!(m.remaining(5000), Some(25000));
        assert!(m.advance(29999, 0).1.is_none());
        assert_eq!(m.advance(30000, 0).1.unwrap().1, Err(Error::Malformed));
    }

    #[test]
    fn aggregate_byte_rejection_preserves_other_input_and_its_deadline() {
        let (mut m, _) = setup();
        m.byte_limit = 8;
        admit(&mut m, 0);
        admit(&mut m, 1);
        data(&mut m, 0, b"GET ");
        data(&mut m, 1, b"GET /over");
        assert!(matches!(m.advance(10, 0), (true, None)));
        let (socket, error) = m.advance(20, 0).1.unwrap();
        assert_eq!(socket.id, 1);
        assert_eq!(error, Err(Error::AllocationFailed));
        assert_eq!(m.inputs[0].as_ref().unwrap().buffer.buffered_len(), 4);
        assert_eq!(m.remaining(20), Some(4980));
    }

    #[test]
    fn fifth_socket_is_returned_without_submitting_or_dropping_it() {
        let (mut m, log) = setup();
        for id in 0..4 {
            admit(&mut m, id);
        }
        let (socket, error) = m
            .admit(
                Socket {
                    id: 4,
                    log: log.clone(),
                },
                0,
                1000,
            )
            .unwrap_err();
        assert_eq!(error, Error::AllocationFailed);
        assert!(log.borrow().is_empty());
        assert_eq!(socket.id, 4);
        assert!(!m.has_capacity());
        drop(socket);
        assert_eq!(&*log.borrow(), &[("socket", 4)]);
    }

    #[test]
    fn rejected_submission_returns_socket_with_no_pending_call() {
        let (mut m, log) = setup();
        m.calls.reject = true;
        let (socket, error) = m
            .admit(
                Socket {
                    id: 0,
                    log: log.clone(),
                },
                0,
                1000,
            )
            .unwrap_err();
        assert_eq!(error, Error::AllocationFailed);
        assert!(log.borrow().is_empty());
        assert!(m.has_capacity());
        drop(socket);
        assert_eq!(&*log.borrow(), &[("socket", 0)]);
    }

    #[test]
    fn rearm_failure_releases_completed_call_and_returns_socket_once() {
        let (mut m, log) = setup();
        admit(&mut m, 0);
        data(&mut m, 0, b"GET ");
        m.calls.reject = true;
        let (socket, error) = m.advance(1, 0).1.unwrap();
        assert_eq!(error, Err(Error::AllocationFailed));
        assert!(m.has_capacity());
        assert_eq!(&*log.borrow(), &[("pending", 0)]);
        drop(socket);
        assert_eq!(&*log.borrow(), &[("pending", 0), ("socket", 0)]);
    }

    #[test]
    fn eof_and_mapping_rejection_release_the_slot() {
        for reply in [Ok(vec![]), Err(Error::AllocationFailed)] {
            let (mut m, _) = setup();
            admit(&mut m, 0);
            m.calls.replies[0].borrow_mut().push_back(reply.clone());
            let (_, error) = m.advance(1, 0).1.unwrap();
            assert_eq!(error, Err(reply.err().unwrap_or(Error::Malformed)));
            assert!(m.has_capacity());
        }
    }

    #[test]
    fn controller_reservations_limit_input_growth_without_changing_deadlines() {
        let (mut m, _) = setup();
        m.byte_limit = 8;
        admit(&mut m, 0);
        data(&mut m, 0, b"GET ");
        let (_, result) = m.advance(10, 5);
        assert_eq!(result.unwrap().1, Err(Error::AllocationFailed));
        assert!(m.has_capacity());
    }

    #[test]
    fn cancellation_precedes_remote_socket_drop_for_every_slot() {
        let (mut m, log) = setup();
        for id in 0..4 {
            admit(&mut m, id);
        }
        drop(m);
        assert_eq!(
            &*log.borrow(),
            &[
                ("pending", 0),
                ("socket", 0),
                ("pending", 1),
                ("socket", 1),
                ("pending", 2),
                ("socket", 2),
                ("pending", 3),
                ("socket", 3),
            ]
        );
    }
}
