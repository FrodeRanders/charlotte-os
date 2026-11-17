//! Owned deployment controller operations sharing the ingress reactor CQ.
use alloc::{
    borrow::Cow,
    format,
    vec::Vec,
};

use catten_rt::owned::{
    CallResult,
    ConnectionRef,
    OwnedMemory,
    PendingCall,
};

use crate::{
    clusterctl,
    clusterctl_client::{
        Error as RpcError,
        RPC_BUDGET_MS,
    },
    deadline::Deadline,
    deployment_http::Request,
    deployment_input::MAX_CLIENTS,
    management_io::Connection as OwnedSocket,
    name_catalog,
};

pub type Response = Cow<'static, str>;

pub enum ReplyKind {
    Scalar,
    Release(Vec<u8>),
    Rollout,
    Policy,
}

struct Prepared {
    memory: Option<OwnedMemory>,
    kind: ReplyKind,
    opcode: u32,
    arg: u64,
    reserved: usize,
}

fn prepare(request: Request, available: usize) -> Result<Prepared, i64> {
    let (opcode, arg, body, prefix, kind) = match request {
        Request::Notify(bytes) => {
            let value = charlotte_launch::deployment::decode(&bytes)
                .ok_or(clusterctl::ERR_UNTRUSTED_DESCRIPTOR)?;
            if value.artifact_name.is_empty() {
                return Err(clusterctl::ERR_TOO_LARGE);
            }
            (clusterctl::OP_NOTIFY, 0, bytes, true, ReplyKind::Scalar)
        }
        Request::Release(bytes) => {
            if charlotte_launch::release::decode(&bytes).is_none() {
                return Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR);
            }
            // The same owner validates the receipt after the moved request's
            // controller call ends. No second retained envelope is allocated.
            return prepare_release(bytes, available);
        }
        Request::Operations(bytes) => {
            if charlotte_launch::operations_bundle::decode(&bytes).is_none() {
                return Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR);
            }
            (clusterctl::OP_NOTIFY_OPERATIONS, 0, bytes, true, ReplyKind::Scalar)
        }
        Request::Shutdown(bytes) => {
            if charlotte_launch::shutdown::decode(&bytes).is_none() {
                return Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR);
            }
            (clusterctl::OP_NOTIFY_SHUTDOWN, 0, bytes, true, ReplyKind::Scalar)
        }
        Request::IngressPolicy(bytes) => {
            if charlotte_launch::ingress_policy::decode(&bytes).is_none() {
                return Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR);
            }
            (clusterctl::OP_NOTIFY_INGRESS_POLICY, 0, bytes, true, ReplyKind::Scalar)
        }
        Request::Status(name) => {
            let length = name.len() as u64;
            (clusterctl::OP_ROLLOUT, length, name, false, ReplyKind::Rollout)
        }
        Request::IngressPolicyStatus => {
            return Ok(Prepared {
                memory: None,
                kind: ReplyKind::Policy,
                opcode: clusterctl::OP_INGRESS_POLICY_STATUS,
                arg: 0,
                reserved: 0,
            });
        }
    };
    let (memory, reserved) = prepare_memory(&body, prefix, 0, available)?;
    Ok(Prepared {
        memory: Some(memory),
        kind,
        opcode,
        arg,
        reserved,
    })
}

fn prepare_release(bytes: Vec<u8>, available: usize) -> Result<Prepared, i64> {
    let (memory, reserved) = prepare_memory(&bytes, true, bytes.capacity(), available)?;
    Ok(Prepared {
        memory: Some(memory),
        kind: ReplyKind::Release(bytes),
        opcode: clusterctl::OP_NOTIFY_RELEASE,
        arg: 0,
        reserved,
    })
}

fn request_size(
    length: usize,
    prefix: bool,
    retained: usize,
    available: usize,
) -> Result<(usize, usize), i64> {
    let length = length
        .checked_add(
            if prefix {
                8
            } else {
                0
            },
        )
        .ok_or(clusterctl::ERR_TOO_LARGE)?;
    let pages = length.div_ceil(4096).max(1);
    let reserved = pages
        .checked_mul(4096)
        .and_then(|bytes| bytes.checked_add(retained))
        .ok_or(clusterctl::ERR_TOO_LARGE)?;
    if reserved > available {
        return Err(clusterctl::ERR_UPLOAD_FAILED);
    }
    Ok((pages, reserved))
}

fn prepare_memory(
    bytes: &[u8],
    prefix: bool,
    retained: usize,
    available: usize,
) -> Result<(OwnedMemory, usize), i64> {
    let (pages, reserved) = request_size(bytes.len(), prefix, retained, available)?;
    let memory = OwnedMemory::allocate(pages).map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    let mut mapping = memory.map_writable().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    let start = if prefix {
        mapping.as_mut_slice()[..8].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
        8
    } else {
        0
    };
    mapping.as_mut_slice()[start..start + bytes.len()].copy_from_slice(bytes);
    let memory = mapping.unmap().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    Ok((memory, reserved))
}

fn decode_release(
    mut reply: CallResult,
    envelope: &[u8],
) -> Result<charlotte_launch::release_admission::Receipt, i64> {
    if reply.result < 0 {
        return Err(reply.result);
    }
    if reply.connection.is_some() {
        return Err(crate::dns::ERR_UNCERTAIN);
    }
    let len = usize::try_from(reply.result).map_err(|_| crate::dns::ERR_UNCERTAIN)?;
    let memory = reply.memory.take().ok_or(crate::dns::ERR_UNCERTAIN)?;
    if len > memory.len() || len > charlotte_launch::release_admission::MAX_LEN {
        return Err(crate::dns::ERR_UNCERTAIN);
    }
    let mapping = memory.map_read_only().map_err(|_| crate::dns::ERR_UNCERTAIN)?;
    crate::release_admission::valid_receipt(&mapping.as_slice()[..len], envelope)
        .ok_or(crate::dns::ERR_UNCERTAIN)
}
fn decode_rollout(reply: CallResult) -> Result<clusterctl::RolloutStatus, i64> {
    if reply.connection.is_some() {
        return Err(crate::dns::ERR_UNCERTAIN);
    }
    if reply.result < 0 {
        return Err(reply.result);
    }
    if reply.result != clusterctl::ROLLOUT_STATUS_LEN as i64 {
        return Err(clusterctl::ERR_NOT_FOUND);
    }
    let memory = reply.memory.ok_or(clusterctl::ERR_NOT_FOUND)?;
    let mapping = memory.map_read_only().map_err(|_| clusterctl::ERR_NOT_FOUND)?;
    clusterctl::RolloutStatus::decode(
        mapping
            .as_slice()
            .get(..clusterctl::ROLLOUT_STATUS_LEN)
            .ok_or(clusterctl::ERR_NOT_FOUND)?,
    )
    .ok_or(clusterctl::ERR_NOT_FOUND)
}
fn decode_policy(reply: CallResult) -> Result<alloc::string::String, i64> {
    if reply.connection.is_some() {
        return Err(crate::dns::ERR_UNCERTAIN);
    }
    if reply.result < 0 {
        return Err(reply.result);
    }
    let len = usize::try_from(reply.result).map_err(|_| clusterctl::ERR_NOT_FOUND)?;
    let memory = reply.memory.ok_or(clusterctl::ERR_NOT_FOUND)?;
    let mapping = memory.map_read_only().map_err(|_| clusterctl::ERR_NOT_FOUND)?;
    let entry = name_catalog::decode_ingress_policy_result(
        mapping.as_slice().get(..len).ok_or(clusterctl::ERR_NOT_FOUND)?,
    )
    .ok_or(clusterctl::ERR_NOT_FOUND)?;
    let policy = charlotte_launch::ingress_policy::decode(&entry.envelope)
        .ok_or(clusterctl::ERR_NOT_FOUND)?;
    Ok(format!(
        "{{\"generation\":{},\"sequence\":{},\"assignments\":{},\"not_before_unix_seconds\":{},\"\
         expires_unix_seconds\":{}}}\n",
        entry.generation,
        policy.sequence,
        policy.assignments().count(),
        policy.not_before_unix_seconds,
        policy.expires_unix_seconds
    ))
}
fn release_response(
    result: Result<charlotte_launch::release_admission::Receipt, i64>,
) -> alloc::string::String {
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(code) => return response(code),
    };
    let mut bytes = [0; charlotte_launch::release_admission::MAX_LEN];
    let Some(len) = receipt.encode(&mut bytes) else {
        return response(crate::dns::ERR_UNCERTAIN);
    };
    let mut hex = alloc::string::String::with_capacity(len * 2);
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in &bytes[..len] {
        hex.push(DIGITS[(byte >> 4) as usize] as char);
        hex.push(DIGITS[(byte & 15) as usize] as char);
    }
    let body = format!("{{\"admission\":\"{hex}\"}}\n");
    format!(
        "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: \
         {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn response(result: i64) -> alloc::string::String {
    let (status, body) = match result {
        generation if generation > 0 => {
            ("202 Accepted", format!("{{\"generation\":{generation}}}\n"))
        }
        clusterctl::ERR_UNTRUSTED_DESCRIPTOR => {
            ("403 Forbidden", format!("{{\"error\":{}}}\n", clusterctl::ERR_UNTRUSTED_DESCRIPTOR))
        }
        clusterctl::ERR_STALE_DESCRIPTOR | clusterctl::ERR_CONFLICTING_DESCRIPTOR => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", result))
        }
        clusterctl::ERR_EXPIRED_OPERATION => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", clusterctl::ERR_EXPIRED_OPERATION))
        }
        clusterctl::ERR_OUTSIDE_VALIDITY => {
            ("409 Conflict", format!("{{\"error\":{}}}\n", clusterctl::ERR_OUTSIDE_VALIDITY))
        }
        clusterctl::ERR_UNSATISFIABLE_PLACEMENT | clusterctl::ERR_INCOMPATIBLE_TARGET => {
            ("409 Conflict", format!("{{\"error\":{result}}}\n"))
        }
        code => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body}",
        body.len()
    )
}

fn status_response(result: Result<alloc::string::String, i64>) -> alloc::string::String {
    let (status, body) = match result {
        Ok(body) => ("200 OK", body),
        Err(clusterctl::ERR_NOT_FOUND) => {
            ("404 Not Found", "{\"error\":\"ingress policy not found\"}\n".into())
        }
        Err(code) => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn rollout_response(result: Result<clusterctl::RolloutStatus, i64>) -> alloc::string::String {
    let (http_status, body) = match result {
        Ok(status) => {
            let state = match status.state {
                clusterctl::ROLLOUT_READY => "ready",
                clusterctl::ROLLOUT_REPLACING => "replacing",
                _ => "committed",
            };
            (
                "200 OK",
                format!(
                    "{{\"state\":\"{state}\",\"deployment_generation\":{},\"service_generation\":\
                     {},\"node_key\":{},\"desired_replicas\":{},\"ready_replicas\":{}}}\n",
                    status.deployment_generation,
                    status.service_generation,
                    status.node_key,
                    status.desired_replicas,
                    status.ready_replicas
                ),
            )
        }
        Err(clusterctl::ERR_NOT_FOUND) => {
            ("404 Not Found", "{\"error\":\"deployment not found\"}\n".into())
        }
        Err(code) => ("503 Service Unavailable", format!("{{\"error\":{code}}}\n")),
    };
    format!(
        "HTTP/1.1 {http_status}\r\nContent-Type: application/json\r\nContent-Length: \
         {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn rpc_code(error: RpcError) -> i64 {
    match error {
        // Admission never accepted a request. A later observation failure
        // cannot establish that the controller did no work.
        RpcError::Observation(_) => crate::dns::ERR_UNCERTAIN,
        other => other.code(),
    }
}

fn render(kind: ReplyKind, result: Result<CallResult, RpcError>) -> Response {
    let reply = match result {
        Ok(reply) => reply,
        Err(error) => return Cow::Owned(response(rpc_code(error))),
    };
    Cow::Owned(match kind {
        ReplyKind::Scalar => {
            let code = if reply.memory.is_some() || reply.connection.is_some() {
                crate::dns::ERR_UNCERTAIN
            } else {
                reply.result
            };
            response(code)
        }
        ReplyKind::Release(envelope) => release_response(decode_release(reply, &envelope)),
        ReplyKind::Rollout => rollout_response(decode_rollout(reply)),
        ReplyKind::Policy => status_response(decode_policy(reply)),
    })
}

struct Begun<P> {
    pending: P,
    kind: ReplyKind,
    reserved: usize,
}

trait Calls {
    type Socket;
    type Pending;
    type Reply;
    fn now(&self) -> u64;
    fn begin(
        &mut self,
        request: Request,
        available: usize,
        deadline: Deadline,
    ) -> Result<Begun<Self::Pending>, Response>;
    fn bind(&mut self, pending: &mut Self::Pending, cq: u32) -> Result<(), ()>;
    fn abort(&mut self, pending: &Self::Pending, socket: &Self::Socket) -> !;
    fn poll(&mut self, pending: &mut Self::Pending) -> Result<Option<Self::Reply>, RpcError>;
    fn render(kind: ReplyKind, result: Result<Self::Reply, RpcError>) -> Response;
}

struct Operation<P, S> {
    // Cancel/observe the accepted call before releasing its socket or envelope.
    pending: P,
    socket: S,
    kind: ReplyKind,
    deadline: Deadline,
    reserved: usize,
}

type Finished<S> = (S, Response);

struct Machine<C: Calls> {
    operations: [Option<Operation<C::Pending, C::Socket>>; MAX_CLIENTS],
    calls: C,
    cursor: usize,
    cq: u32,
}

impl<C: Calls> Machine<C> {
    fn new(calls: C, cq: u32) -> Self {
        Self {
            operations: core::array::from_fn(|_| None),
            calls,
            cursor: 0,
            cq,
        }
    }

    fn start(
        &mut self,
        socket: C::Socket,
        request: Request,
        available: usize,
        now: u64,
        frequency: u64,
    ) -> Option<Finished<C::Socket>> {
        let deadline = Deadline::at(now, frequency, RPC_BUDGET_MS);
        let Some(index) = self.operations.iter().position(Option::is_none) else {
            return Some((socket, Cow::Owned(response(clusterctl::ERR_UPLOAD_FAILED))));
        };
        if deadline.expired_at(self.calls.now()) {
            return Some((socket, C::render(ReplyKind::Scalar, Err(RpcError::Timeout))));
        }
        let Begun {
            pending,
            kind,
            reserved,
        } = match self.calls.begin(request, available, deadline) {
            Ok(begun) => begun,
            Err(reply) => return Some((socket, reply)),
        };
        self.operations[index] = Some(Operation {
            pending,
            socket,
            kind,
            deadline,
            reserved,
        });
        let owner = self.operations[index].as_mut().unwrap();
        if self.calls.bind(&mut owner.pending, self.cq).is_err() {
            // Accepted work remains in its containing owner through abort.
            self.calls.abort(&owner.pending, &owner.socket);
        }
        None
    }

    fn advance(&mut self) -> Option<Finished<C::Socket>> {
        let now = self.calls.now();
        for offset in 0..MAX_CLIENTS {
            let index = (self.cursor + offset) % MAX_CLIENTS;
            let Some(owner) = self.operations[index].as_mut() else {
                continue;
            };
            let result = if owner.deadline.expired_at(now) {
                Err(RpcError::Timeout)
            } else {
                match self.calls.poll(&mut owner.pending) {
                    Ok(None) => continue,
                    Ok(Some(reply)) => Ok(reply),
                    Err(error) => Err(error),
                }
            };
            self.cursor = (index + 1) % MAX_CLIENTS;
            let Operation {
                pending,
                socket,
                kind,
                ..
            } = self.operations[index].take().unwrap();
            drop(pending);
            return Some((socket, C::render(kind, result)));
        }
        None
    }

    fn reserved_bytes(&self) -> usize {
        self.operations.iter().flatten().map(|owner| owner.reserved).sum()
    }

    fn occupied_slots(&self) -> usize {
        self.operations.iter().flatten().count()
    }

    fn remaining_ms(&self) -> Option<u64> {
        let now = self.calls.now();
        self.operations.iter().flatten().map(|owner| owner.deadline.remaining_ms_at(now)).min()
    }
}

struct Runtime<'a, S> {
    controller: ConnectionRef<'a>,
    _socket: core::marker::PhantomData<fn() -> S>,
}
impl<S> Calls for Runtime<'_, S> {
    type Pending = PendingCall<'static>;
    type Reply = CallResult;
    type Socket = S;

    fn now(&self) -> u64 {
        catten_syscall::monotonic_clock().0
    }

    fn begin(
        &mut self,
        request: Request,
        available: usize,
        deadline: Deadline,
    ) -> Result<Begun<Self::Pending>, Response> {
        let Prepared {
            memory,
            kind,
            opcode,
            arg,
            reserved,
        } = prepare(request, available).map_err(|code| Cow::Owned(response(code)))?;
        if deadline.expired() {
            return Err(Cow::Owned(response(clusterctl::ERR_RPC_TIMEOUT)));
        }
        let pending = match memory {
            Some(memory) => self
                .controller
                .call_move(opcode, arg, memory)
                .map_err(|(_memory, error)| RpcError::Admission(error)),
            None => self.controller.call(opcode, arg).map_err(RpcError::Admission),
        }
        .map_err(|error| Cow::Owned(response(error.code())))?;
        Ok(Begun {
            pending,
            kind,
            reserved,
        })
    }

    fn bind(&mut self, pending: &mut Self::Pending, cq: u32) -> Result<(), ()> {
        pending.bind_cq(cq).map_err(|_| ())
    }

    fn abort(&mut self, _pending: &Self::Pending, _socket: &Self::Socket) -> ! {
        catten_rt::domain_abort()
    }

    fn poll(&mut self, pending: &mut Self::Pending) -> Result<Option<Self::Reply>, RpcError> {
        pending.poll().map_err(RpcError::Observation)
    }

    fn render(kind: ReplyKind, result: Result<Self::Reply, RpcError>) -> Response {
        render(kind, result)
    }
}

/// Retains each socket with its one accepted controller call, original RPC
/// budget and receipt-validation envelope. Timeout never replays a mutation.
/// The reservation bounds active inputs/requests; it does not refund actual
/// kernel charges or prove the receiver released a transferred request.
pub struct Controls<'a, S = OwnedSocket<'a>>(Machine<Runtime<'a, S>>);
impl<'a, S> Controls<'a, S> {
    pub fn new(controller: ConnectionRef<'a>, cq: u32) -> Self {
        Self(Machine::new(
            Runtime {
                controller,
                _socket: core::marker::PhantomData,
            },
            cq,
        ))
    }

    pub fn start(&mut self, socket: S, request: Request, available: usize) -> Option<Finished<S>> {
        let (now, frequency) = catten_syscall::monotonic_clock();
        self.0.start(socket, request, available, now, frequency)
    }

    pub fn advance(&mut self) -> Option<Finished<S>> {
        self.0.advance()
    }

    pub fn remaining_ms(&self) -> Option<u64> {
        self.0.remaining_ms()
    }

    pub fn occupied_slots(&self) -> usize {
        self.0.occupied_slots()
    }

    pub fn reserved_bytes(&self) -> usize {
        self.0.reserved_bytes()
    }
}

#[cfg(test)]
mod tests {
    use alloc::{
        collections::VecDeque,
        rc::Rc,
        vec,
    };
    use core::cell::{
        Cell,
        RefCell,
    };

    use catten_rt::owned::IpcError;

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
        submitted: usize,
        reject: bool,
        bind_reject: bool,
        replies: [VecDeque<Result<i64, RpcError>>; MAX_CLIENTS],
    }
    impl Calls for Fake {
        type Pending = Pending;
        type Reply = i64;
        type Socket = Socket;

        fn now(&self) -> u64 {
            self.now.get()
        }

        fn begin(
            &mut self,
            _request: Request,
            available: usize,
            _deadline: Deadline,
        ) -> Result<Begun<Pending>, Response> {
            if self.reject || available < 4096 {
                return Err(Cow::Borrowed("rejected"));
            }
            let id = self.submitted;
            self.submitted += 1;
            Ok(Begun {
                pending: Pending(id, self.log.clone()),
                kind: ReplyKind::Scalar,
                reserved: 4096,
            })
        }

        fn bind(&mut self, _pending: &mut Pending, cq: u32) -> Result<(), ()> {
            assert_eq!(cq, 7);
            if self.bind_reject {
                Err(())
            } else {
                Ok(())
            }
        }

        fn abort(&mut self, pending: &Pending, socket: &Socket) -> ! {
            assert_eq!(pending.0, socket.0);
            assert!(self.log.borrow().is_empty());
            panic!("binding rejected with operation owner live")
        }

        fn poll(&mut self, pending: &mut Pending) -> Result<Option<i64>, RpcError> {
            self.replies[pending.0].pop_front().transpose()
        }

        fn render(_kind: ReplyKind, reply: Result<i64, RpcError>) -> Response {
            Cow::Owned(reply.map_or_else(rpc_code, |code| code).to_string())
        }
    }
    use alloc::string::ToString;
    fn setup() -> Machine<Fake> {
        Machine::new(
            Fake {
                now: Rc::new(Cell::new(0)),
                log: Rc::new(RefCell::new(Vec::new())),
                submitted: 0,
                reject: false,
                bind_reject: false,
                replies: core::array::from_fn(|_| VecDeque::new()),
            },
            7,
        )
    }
    fn start(m: &mut Machine<Fake>, id: usize) {
        assert!(
            m.start(Socket(id, m.calls.log.clone()), Request::IngressPolicyStatus, 4096, 0, 1000)
                .is_none()
        );
    }
    #[test]
    fn healthy_controller_reply_progresses_beside_an_unanswered_call() {
        let mut m = setup();
        start(&mut m, 0);
        start(&mut m, 1);
        m.calls.replies[1].push_back(Ok(42));
        m.calls.now.set(30);
        let (socket, reply) = m.advance().unwrap();
        assert_eq!(socket.0, 1);
        assert_eq!(reply, "42");
        assert_eq!(m.occupied_slots(), 1);
        assert_eq!(m.reserved_bytes(), 4096);
        assert_eq!(m.remaining_ms(), Some(4970));
        assert_eq!(m.calls.submitted, 2);
    }
    #[test]
    fn unrelated_wakes_preserve_original_timeout_and_never_replay() {
        let mut m = setup();
        start(&mut m, 0);
        for now in [1, 2000, 4999] {
            m.calls.now.set(now);
            assert!(m.advance().is_none());
        }
        m.calls.now.set(5000);
        let (socket, reply) = m.advance().unwrap();
        assert_eq!(reply, clusterctl::ERR_RPC_TIMEOUT.to_string());
        assert_eq!(m.calls.submitted, 1);
        assert_eq!(m.occupied_slots(), 0);
        assert_eq!(&*m.calls.log.borrow(), &[("pending", 0)]);
        drop(socket);
        assert_eq!(&*m.calls.log.borrow(), &[("pending", 0), ("socket", 0)]);
        assert!(m.advance().is_none());
    }
    #[test]
    fn submission_and_budget_rejection_return_socket_without_a_pending_owner() {
        for (reject, available) in [(true, 4096), (false, 4095)] {
            let mut m = setup();
            m.calls.reject = reject;
            let (socket, reply) = m
                .start(
                    Socket(0, m.calls.log.clone()),
                    Request::IngressPolicyStatus,
                    available,
                    0,
                    1000,
                )
                .unwrap();
            assert_eq!(reply, "rejected");
            assert_eq!(m.calls.submitted, 0);
            assert!(m.calls.log.borrow().is_empty());
            drop(socket);
            assert_eq!(&*m.calls.log.borrow(), &[("socket", 0)]);
        }
    }
    #[test]
    fn full_pool_rejects_before_submitting_a_fifth_call() {
        let mut m = setup();
        for id in 0..4 {
            start(&mut m, id);
        }
        let result =
            m.start(Socket(4, m.calls.log.clone()), Request::IngressPolicyStatus, 4096, 0, 1000);
        assert!(result.is_some());
        assert_eq!(m.calls.submitted, 4);
        assert_eq!(m.occupied_slots(), 4);
    }
    #[test]
    fn observation_rejection_is_terminal_and_cancels_once() {
        let mut m = setup();
        start(&mut m, 0);
        m.calls.replies[0].push_back(Err(RpcError::Observation(IpcError::Status(99))));
        let (_, reply) = m.advance().unwrap();
        assert_eq!(reply, crate::dns::ERR_UNCERTAIN.to_string());
        assert_eq!(m.calls.submitted, 1);
        assert!(m.advance().is_none());
    }
    #[test]
    fn pool_drop_cancels_each_pending_before_its_socket() {
        let mut m = setup();
        for id in 0..4 {
            start(&mut m, id);
        }
        let log = m.calls.log.clone();
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
                ("socket", 3)
            ]
        );
    }
    #[test]
    #[should_panic(expected = "binding rejected with operation owner live")]
    fn accepted_binding_rejection_aborts_with_pending_and_socket_live() {
        let mut m = setup();
        m.calls.bind_reject = true;
        start(&mut m, 0);
    }

    #[test]
    fn preparation_cost_includes_rounded_pages_and_receipt_backing() {
        assert_eq!(request_size(4090, true, 100, 8292), Ok((2, 8292)));
        assert_eq!(request_size(4090, true, 100, 8291), Err(clusterctl::ERR_UPLOAD_FAILED));
        assert_eq!(request_size(usize::MAX, true, 0, usize::MAX), Err(clusterctl::ERR_TOO_LARGE));
        assert_eq!(request_size(1, false, usize::MAX, usize::MAX), Err(clusterctl::ERR_TOO_LARGE));
        let size = charlotte_launch::operations_bundle::MAX_BUNDLE_LEN;
        assert_eq!(
            request_size(size, true, 0, crate::deployment_input::MAX_BUFFERED_BYTES),
            Ok((256, 1048576))
        );
    }
    #[test]
    fn retired_or_malformed_mutations_reject_before_kernel_memory_preparation() {
        for request in [
            Request::Notify(vec![0; 20]),
            Request::Release(vec![0; 20]),
            Request::Operations(vec![0; 20]),
            Request::Shutdown(vec![0; 20]),
            Request::IngressPolicy(vec![0; 20]),
        ] {
            assert!(matches!(
                prepare(request, usize::MAX),
                Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR)
            ));
        }
    }
}
