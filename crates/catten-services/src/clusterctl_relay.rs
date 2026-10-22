//! Bounded DNS forwarding for HTTP-facing cluster administration operations.
use catten_rt::owned::{
    CallResult,
    ConnectionRef,
    IncomingMessage,
    IpcError,
    MappedMemory,
    OwnedMemory,
    PendingCall,
    ReadOnly,
};

use crate::{
    clusterctl,
    deadline::Deadline,
    dns,
    name_catalog,
};

pub const MAX_RELAYS: usize = 8;
/// Original incoming backing plus one copied DNS request per admitted operation.
/// Returned records and transient decoder allocations remain separate budgets.
pub const MAX_REQUEST_BYTES: usize = 2 * crate::deployment_http::MAX_REQUEST_BYTES;
pub const RELAY_MS: u64 = crate::clusterctl_client::RPC_BUDGET_MS;

pub fn handles(opcode: u32) -> bool {
    matches!(
        opcode,
        clusterctl::OP_NOTIFY
            | clusterctl::OP_NOTIFY_RELEASE
            | clusterctl::OP_NOTIFY_OPERATIONS
            | clusterctl::OP_NOTIFY_SHUTDOWN
            | clusterctl::OP_NOTIFY_INGRESS_POLICY
            | clusterctl::OP_STATUS
            | clusterctl::OP_ROLLOUT
            | clusterctl::OP_INGRESS_POLICY_STATUS
    )
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Initial,
    Preflight,
    Submit,
    Reply,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Progress {
    Pending,
    Next,
    Complete,
}
trait Calls {
    type Owner;
    fn now(&self) -> (u64, u64);
    fn reservation(owner: &Self::Owner) -> Result<usize, i64>;
    fn begin(&mut self, owner: &mut Self::Owner, deadline: &Deadline) -> Result<(), i64>;
    fn bind(&mut self, owner: &mut Self::Owner, cq: u32) -> Result<(), IpcError>;
    fn poll(&mut self, owner: &mut Self::Owner) -> Result<Progress, i64>;
    fn finish(&mut self, owner: Self::Owner, code: Option<i64>);
    fn abort(&mut self, owner: &Self::Owner, error: IpcError) -> !;
}
struct Entry<O> {
    owner: O,
    deadline: Deadline,
    reserved: usize,
}
struct Machine<C: Calls> {
    entries: [Option<Entry<C::Owner>>; MAX_RELAYS],
    reserved: usize,
    cursor: usize,
    cq: u32,
    calls: C,
}
impl<C: Calls> Machine<C> {
    fn new(calls: C, cq: u32) -> Self {
        Self {
            entries: core::array::from_fn(|_| None),
            reserved: 0,
            cursor: 0,
            cq,
            calls,
        }
    }

    fn start(&mut self, owner: C::Owner, deadline: Deadline) {
        let reserved = match C::reservation(&owner) {
            Ok(bytes) => bytes,
            Err(code) => {
                self.calls.finish(owner, Some(code));
                return;
            }
        };
        let Some(index) = self
            .entries
            .iter()
            .position(Option::is_none)
            .filter(|_| reserved <= MAX_REQUEST_BYTES.saturating_sub(self.reserved))
        else {
            self.calls.finish(owner, Some(clusterctl::ERR_CAPACITY));
            return;
        };
        self.reserved += reserved;
        self.entries[index] = Some(Entry {
            owner,
            deadline,
            reserved,
        });
        self.arm(index);
    }

    fn arm(&mut self, index: usize) {
        let entry = self.entries[index].as_mut().unwrap();
        if entry.deadline.expired_at(self.calls.now().0) {
            self.finish(index, Some(clusterctl::ERR_RPC_TIMEOUT));
            return;
        }
        if let Err(code) = self.calls.begin(&mut entry.owner, &entry.deadline) {
            self.finish(index, Some(code));
            return;
        }
        if let Err(error) = self.calls.bind(&mut entry.owner, self.cq) {
            // begin accepted exactly one request. Keep its whole transaction in
            // the pool through abort; binding rejection cannot authorize replay.
            self.calls.abort(&entry.owner, error);
        }
    }

    fn finish(&mut self, index: usize, code: Option<i64>) {
        let entry = self.entries[index].take().unwrap();
        // Cancellation/mapping and upstream reply finish before logical capacity
        // returns. A moved DNS request may still belong to its receiver afterward.
        self.calls.finish(entry.owner, code);
        self.reserved -= entry.reserved;
    }

    fn advance(&mut self) -> bool {
        let now = self.calls.now().0;
        for offset in 0..MAX_RELAYS {
            let index = (self.cursor + offset) % MAX_RELAYS;
            let Some(entry) = self.entries[index].as_mut() else {
                continue;
            };
            let result = if entry.deadline.expired_at(now) {
                Err(clusterctl::ERR_RPC_TIMEOUT)
            } else {
                self.calls.poll(&mut entry.owner)
            };
            match result {
                Ok(Progress::Pending) => continue,
                Ok(Progress::Next) => {
                    self.cursor = (index + 1) % MAX_RELAYS;
                    self.arm(index);
                }
                Ok(Progress::Complete) => {
                    self.cursor = (index + 1) % MAX_RELAYS;
                    self.finish(index, None);
                }
                Err(code) => {
                    self.cursor = (index + 1) % MAX_RELAYS;
                    self.finish(index, Some(code));
                }
            }
            return true;
        }
        false
    }

    fn remaining_ms(&self) -> Option<u64> {
        let now = self.calls.now().0;
        self.entries.iter().flatten().map(|e| e.deadline.remaining_ms_at(now)).min()
    }
}
struct Relay {
    pending: Option<PendingCall<'static>>,
    mapping: Option<MappedMemory<ReadOnly>>,
    result: Option<CallResult>,
    incoming: IncomingMessage,
    phase: Phase,
    payload_len: usize,
}
impl Relay {
    fn new(incoming: IncomingMessage) -> Self {
        Self {
            pending: None,
            mapping: None,
            result: None,
            incoming,
            phase: Phase::Initial,
            payload_len: 0,
        }
    }

    fn bytes(&self) -> &[u8] {
        let offset = if prefixed(self.incoming.opcode) {
            8
        } else {
            0
        };
        &self.mapping.as_ref().unwrap().as_slice()[offset..offset + self.payload_len]
    }

    fn prepare(&mut self, key: &[u8; 32]) -> Result<(), i64> {
        if self.incoming.opcode == clusterctl::OP_INGRESS_POLICY_STATUS {
            return Ok(());
        }
        let memory = self.incoming.memory.take().ok_or(clusterctl::ERR_TOO_LARGE)?;
        self.mapping = Some(memory.map_read_only().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?);
        let bytes = self.mapping.as_ref().unwrap().as_slice();
        self.payload_len = payload_len(self.incoming.opcode, self.incoming.arg0, bytes)?;
        validate(self.incoming.opcode, self.bytes(), key)
    }

    fn finish(mut self, code: Option<i64>) {
        drop(self.pending.take());
        drop(self.mapping.take());
        drop(self.incoming.memory.take());
        drop(self.incoming.connection.take());
        let result = self.result.take();
        if let Some(code) = code {
            drop(result);
            if let Some(reply) = self.incoming.reply.take() {
                let _ = reply.reply(code);
            }
        } else if let Some(mut result) = result
            && let Some(reply) = self.incoming.reply.take()
        {
            if let Some(memory) = result.memory.take() {
                let _ = reply.reply_move(memory, result.result);
            } else {
                let _ = reply.reply(result.result);
            }
        }
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        // Original request access must end before reply-token destruction can
        // cancel an incoming loan. Pending DNS work is always cancelled first.
        drop(self.pending.take());
        drop(self.mapping.take());
    }
}
fn prefixed(opcode: u32) -> bool {
    !matches!(
        opcode,
        clusterctl::OP_STATUS | clusterctl::OP_ROLLOUT | clusterctl::OP_INGRESS_POLICY_STATUS
    )
}
fn limit(opcode: u32) -> usize {
    match opcode {
        clusterctl::OP_NOTIFY => charlotte_launch::deployment::MAX_DESCRIPTOR_LEN,
        clusterctl::OP_NOTIFY_RELEASE => charlotte_launch::release::MAX_RELEASE_LEN,
        clusterctl::OP_NOTIFY_OPERATIONS => charlotte_launch::operations_bundle::MAX_BUNDLE_LEN,
        clusterctl::OP_NOTIFY_SHUTDOWN => charlotte_launch::shutdown::ENCODED_LEN,
        clusterctl::OP_NOTIFY_INGRESS_POLICY => charlotte_launch::ingress_policy::MAX_ENCODED_LEN,
        clusterctl::OP_STATUS | clusterctl::OP_ROLLOUT => {
            charlotte_launch::deployment::MAX_ARTIFACT_NAME_LEN
        }
        _ => 0,
    }
}
fn request_reservation(opcode: u32, memory_len: usize, connection: bool) -> Result<usize, i64> {
    if !handles(opcode) || connection {
        return Err(clusterctl::ERR_TOO_LARGE);
    }
    if opcode == clusterctl::OP_INGRESS_POLICY_STATUS {
        return if memory_len == 0 {
            Ok(0)
        } else {
            Err(clusterctl::ERR_TOO_LARGE)
        };
    }
    let maximum = (limit(opcode)
        + if prefixed(opcode) {
            8
        } else {
            0
        })
    .div_ceil(4096)
        * 4096;
    if memory_len == 0 || memory_len > maximum {
        return Err(clusterctl::ERR_TOO_LARGE);
    }
    memory_len.checked_mul(2).ok_or(clusterctl::ERR_TOO_LARGE)
}
fn payload_len(opcode: u32, arg0: u64, bytes: &[u8]) -> Result<usize, i64> {
    let len = if prefixed(opcode) {
        u64::from_le_bytes(bytes.get(..8).ok_or(clusterctl::ERR_TOO_LARGE)?.try_into().unwrap())
    } else {
        arg0
    };
    let len = usize::try_from(len).map_err(|_| clusterctl::ERR_TOO_LARGE)?;
    let offset = if prefixed(opcode) {
        8
    } else {
        0
    };
    if len == 0 || len > limit(opcode) || len > bytes.len().saturating_sub(offset) {
        return Err(clusterctl::ERR_TOO_LARGE);
    }
    Ok(len)
}
fn validate(opcode: u32, bytes: &[u8], key: &[u8; 32]) -> Result<(), i64> {
    let trusted = match opcode {
        clusterctl::OP_NOTIFY => {
            if charlotte_launch::deployment::verify(bytes, key)
                != charlotte_launch::deployment::VerifyOutcome::Valid
            {
                return Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR);
            }
            let descriptor = charlotte_launch::deployment::decode(bytes)
                .ok_or(clusterctl::ERR_UNTRUSTED_DESCRIPTOR)?;
            if descriptor.node_key == 0
                && descriptor.placement != charlotte_launch::placement::PlacementPolicy::singleton()
            {
                return Err(clusterctl::ERR_UNSATISFIABLE_PLACEMENT);
            }
            true
        }
        clusterctl::OP_NOTIFY_RELEASE => {
            charlotte_launch::release::verify(bytes, key)
                == charlotte_launch::release::VerifyOutcome::Valid
        }
        // These envelopes are authenticated with current keys, UTC and recipient
        // context at the DNS leader, as before. This hop validates their grammar.
        clusterctl::OP_NOTIFY_OPERATIONS => {
            charlotte_launch::operations_bundle::decode(bytes).is_some()
        }
        clusterctl::OP_NOTIFY_SHUTDOWN => charlotte_launch::shutdown::decode(bytes).is_some(),
        clusterctl::OP_NOTIFY_INGRESS_POLICY => {
            charlotte_launch::ingress_policy::decode(bytes).is_some()
        }
        clusterctl::OP_STATUS | clusterctl::OP_ROLLOUT => {
            charlotte_launch::deployment::valid_artifact_name(bytes)
        }
        _ => false,
    };
    if trusted {
        Ok(())
    } else {
        Err(clusterctl::ERR_UNTRUSTED_DESCRIPTOR)
    }
}
fn copied_request(bytes: &[u8]) -> Result<OwnedMemory, i64> {
    let memory = OwnedMemory::allocate(bytes.len().div_ceil(4096).max(1))
        .map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    let mut mapping = memory.map_writable().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)?;
    mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
    mapping.unmap().map_err(|_| clusterctl::ERR_UPLOAD_FAILED)
}
fn dns_opcode(opcode: u32) -> u32 {
    match opcode {
        clusterctl::OP_NOTIFY => dns::OP_DEPLOY_SUBMIT,
        clusterctl::OP_NOTIFY_RELEASE => dns::OP_DEPLOY_RELEASE,
        clusterctl::OP_NOTIFY_OPERATIONS => dns::OP_DEPLOY_OPERATIONS,
        clusterctl::OP_NOTIFY_SHUTDOWN => dns::OP_SHUTDOWN_SUBMIT,
        clusterctl::OP_NOTIFY_INGRESS_POLICY => dns::OP_INGRESS_POLICY_SUBMIT,
        clusterctl::OP_STATUS => dns::OP_DEPLOY_QUERY_NAMED,
        clusterctl::OP_ROLLOUT => dns::OP_DEPLOY_ROLLOUT_NAMED,
        clusterctl::OP_INGRESS_POLICY_STATUS => dns::OP_INGRESS_POLICY_QUERY,
        _ => unreachable!(),
    }
}
// The deployment grammar has a u16 pin count. Bound before mapping or allocating
// decoder vectors, even when the returned backing is larger than its claimed frame.
fn reply_bound(opcode: u32, len: usize, backing: usize) -> bool {
    let maximum = match opcode {
        clusterctl::OP_NOTIFY_RELEASE => charlotte_launch::release_admission::MAX_LEN,
        clusterctl::OP_STATUS => {
            22 + charlotte_launch::deployment::MAX_DESCRIPTOR_LEN + usize::from(u16::MAX) * 10
        }
        clusterctl::OP_ROLLOUT => clusterctl::ROLLOUT_STATUS_LEN,
        clusterctl::OP_INGRESS_POLICY_STATUS => {
            8 + charlotte_launch::ingress_policy::MAX_ENCODED_LEN
        }
        _ => return false,
    };
    len != 0 && len <= maximum && len <= backing && backing <= maximum.div_ceil(4096) * 4096
}
/// Only an explicit absent record permits first submission. Decode/transport
/// failure never substitutes for absence or permits advancing to mutation.
fn preflight(descriptor: &[u8], result: i64, memory: Option<&[u8]>) -> Result<Option<i64>, i64> {
    let requested = charlotte_launch::deployment::decode(descriptor).ok_or(dns::ERR_UNCERTAIN)?;
    if result == dns::ERR_NOT_FOUND && memory.is_none() {
        return Ok(None);
    }
    if result < 0 {
        return Err(if memory.is_some() {
            dns::ERR_UNCERTAIN
        } else {
            result
        });
    }
    let bytes = memory.ok_or(dns::ERR_UNCERTAIN)?;
    let current = name_catalog::decode_deployment_result(bytes).ok_or(dns::ERR_UNCERTAIN)?;
    let previous =
        charlotte_launch::deployment::decode(&current.descriptor).ok_or(dns::ERR_UNCERTAIN)?;
    if previous.artifact_name != requested.artifact_name {
        return Err(dns::ERR_UNCERTAIN);
    }
    if requested.sequence < previous.sequence {
        return Err(clusterctl::ERR_STALE_DESCRIPTOR);
    }
    if requested.sequence == previous.sequence {
        return if descriptor == current.descriptor {
            Ok(Some(i64::try_from(current.generation).map_err(|_| dns::ERR_UNCERTAIN)?))
        } else {
            Err(clusterctl::ERR_CONFLICTING_DESCRIPTOR)
        };
    }
    Ok(None)
}
struct Runtime<'a> {
    dns: ConnectionRef<'a>,
    key: &'a [u8; 32],
}
impl Calls for Runtime<'_> {
    type Owner = Relay;

    fn now(&self) -> (u64, u64) {
        catten_syscall::monotonic_clock()
    }

    fn reservation(owner: &Relay) -> Result<usize, i64> {
        request_reservation(
            owner.incoming.opcode,
            owner.incoming.memory.as_ref().map_or(0, |m| m.len()),
            owner.incoming.connection.is_some(),
        )
    }

    fn begin(&mut self, owner: &mut Relay, deadline: &Deadline) -> Result<(), i64> {
        assert!(owner.pending.is_none());
        if owner.phase == Phase::Initial {
            owner.prepare(self.key)?;
        }
        let preflight =
            owner.phase == Phase::Initial && owner.incoming.opcode == clusterctl::OP_NOTIFY;
        let (opcode, len, memory) = if owner.incoming.opcode == clusterctl::OP_INGRESS_POLICY_STATUS
        {
            (dns::OP_INGRESS_POLICY_QUERY, 0, None)
        } else {
            let bytes = if preflight {
                charlotte_launch::deployment::decode(owner.bytes()).unwrap().artifact_name
            } else {
                owner.bytes()
            };
            (
                if preflight {
                    dns::OP_DEPLOY_QUERY_NAMED
                } else {
                    dns_opcode(owner.incoming.opcode)
                },
                bytes.len() as u64,
                Some(copied_request(bytes)?),
            )
        };
        if deadline.expired() {
            return Err(clusterctl::ERR_RPC_TIMEOUT);
        }
        owner.pending = Some(match memory {
            Some(memory) => {
                self.dns.call_move(opcode, len, memory).map_err(|_| clusterctl::ERR_NOT_LEADER)?
            }
            None => self.dns.call(opcode, 0).map_err(|_| clusterctl::ERR_NOT_LEADER)?,
        });
        owner.phase = if preflight {
            Phase::Preflight
        } else {
            Phase::Reply
        };
        Ok(())
    }

    fn bind(&mut self, owner: &mut Relay, cq: u32) -> Result<(), IpcError> {
        owner.pending.as_mut().unwrap().bind_cq(cq)
    }

    fn poll(&mut self, owner: &mut Relay) -> Result<Progress, i64> {
        let Some(mut reply) =
            owner.pending.as_mut().unwrap().poll().map_err(|_| dns::ERR_UNCERTAIN)?
        else {
            return Ok(Progress::Pending);
        };
        drop(owner.pending.take());
        if reply.connection.is_some() {
            return Err(dns::ERR_UNCERTAIN);
        }
        if owner.phase == Phase::Preflight {
            let decision = if let Some(memory) = reply.memory.take() {
                let len = usize::try_from(reply.result).map_err(|_| dns::ERR_UNCERTAIN)?;
                if !reply_bound(clusterctl::OP_STATUS, len, memory.len()) {
                    return Err(dns::ERR_UNCERTAIN);
                }
                let mapping = memory.map_read_only().map_err(|_| dns::ERR_UNCERTAIN)?;
                preflight(
                    owner.bytes(),
                    reply.result,
                    Some(mapping.as_slice().get(..len).ok_or(dns::ERR_UNCERTAIN)?),
                )?
            } else {
                preflight(owner.bytes(), reply.result, None)?
            };
            if let Some(generation) = decision {
                owner.result = Some(CallResult {
                    result: generation,
                    memory: None,
                    connection: None,
                });
                return Ok(Progress::Complete);
            }
            owner.phase = Phase::Submit;
            return Ok(Progress::Next);
        }
        if reply.result < 0
            || matches!(
                owner.incoming.opcode,
                clusterctl::OP_NOTIFY
                    | clusterctl::OP_NOTIFY_OPERATIONS
                    | clusterctl::OP_NOTIFY_SHUTDOWN
                    | clusterctl::OP_NOTIFY_INGRESS_POLICY
            )
        {
            if reply.memory.is_some() {
                return Err(dns::ERR_UNCERTAIN);
            }
        } else {
            let len = usize::try_from(reply.result).map_err(|_| dns::ERR_UNCERTAIN)?;
            let memory = reply.memory.take().ok_or(dns::ERR_UNCERTAIN)?;
            if !reply_bound(owner.incoming.opcode, len, memory.len()) {
                return Err(dns::ERR_UNCERTAIN);
            }
            let mapping = memory.map_read_only().map_err(|_| dns::ERR_UNCERTAIN)?;
            let bytes = mapping.as_slice().get(..len).ok_or(dns::ERR_UNCERTAIN)?;
            let valid = match owner.incoming.opcode {
                clusterctl::OP_NOTIFY_RELEASE => {
                    crate::release_admission::valid_receipt(bytes, owner.bytes()).is_some()
                }
                clusterctl::OP_STATUS => {
                    name_catalog::decode_deployment_result(bytes).is_some_and(|entry| {
                        charlotte_launch::deployment::decode(&entry.descriptor)
                            .is_some_and(|descriptor| descriptor.artifact_name == owner.bytes())
                    })
                }
                clusterctl::OP_ROLLOUT => clusterctl::RolloutStatus::decode(bytes).is_some(),
                clusterctl::OP_INGRESS_POLICY_STATUS => {
                    name_catalog::decode_ingress_policy_result(bytes).is_some()
                }
                _ => false,
            };
            if !valid {
                return Err(dns::ERR_UNCERTAIN);
            }
            reply.memory = Some(mapping.unmap().map_err(|_| dns::ERR_UNCERTAIN)?);
        }
        owner.result = Some(reply);
        Ok(Progress::Complete)
    }

    fn finish(&mut self, owner: Relay, code: Option<i64>) {
        owner.finish(code);
    }

    fn abort(&mut self, _owner: &Relay, error: IpcError) -> ! {
        catten_rt::logln!(
            "[clusterctl] accepted DNS relay binding failed; transaction retained: {error:?}"
        );
        catten_rt::domain_abort()
    }
}
/// Server-side request owners retain their incoming reply and original mapping
/// until the one accepted DNS phase completes or is explicitly cancelled.
pub struct Relays<'a>(Machine<Runtime<'a>>);
impl<'a> Relays<'a> {
    pub fn new(dns: ConnectionRef<'a>, key: &'a [u8; 32], cq: u32) -> Self {
        Self(Machine::new(
            Runtime {
                dns,
                key,
            },
            cq,
        ))
    }

    pub fn start(&mut self, incoming: IncomingMessage) {
        let deadline = Deadline::after(RELAY_MS);
        self.0.start(Relay::new(incoming), deadline);
    }

    pub fn advance(&mut self) -> bool {
        self.0.advance()
    }

    pub fn remaining_ms(&self) -> Option<u64> {
        self.0.remaining_ms()
    }

    pub fn occupied_slots(&self) -> usize {
        self.0.entries.iter().flatten().count()
    }

    pub fn reserved_bytes(&self) -> usize {
        self.0.reserved
    }
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
    type Log = Rc<RefCell<Vec<(&'static str, usize)>>>;
    struct Resource(&'static str, usize, Log);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.2.borrow_mut().push((self.0, self.1));
        }
    }
    struct Owner {
        id: usize,
        pending: Option<Resource>,
        mapping: Option<Resource>,
        reply: Option<Resource>,
        phase: usize,
        bytes: usize,
    }
    struct Fake {
        now: Cell<u64>,
        log: Log,
        replies: [VecDeque<Result<Progress, i64>>; MAX_RELAYS],
        begin_error: Option<i64>,
        bind_error: bool,
        preparation_ms: u64,
        submitted: Vec<(usize, usize)>,
        results: Vec<(usize, Option<i64>)>,
    }
    impl Calls for Fake {
        type Owner = Owner;

        fn now(&self) -> (u64, u64) {
            (self.now.get(), 1000)
        }

        fn reservation(owner: &Owner) -> Result<usize, i64> {
            Ok(owner.bytes)
        }

        fn begin(&mut self, owner: &mut Owner, deadline: &Deadline) -> Result<(), i64> {
            assert!(owner.pending.is_none());
            self.now.set(self.now.get() + self.preparation_ms);
            if deadline.expired_at(self.now.get()) {
                return Err(clusterctl::ERR_RPC_TIMEOUT);
            }
            if let Some(code) = self.begin_error.take() {
                return Err(code);
            }
            assert!(!self.submitted.contains(&(owner.id, owner.phase)), "accepted phase replayed");
            self.submitted.push((owner.id, owner.phase));
            owner.pending = Some(Resource("pending", owner.id, self.log.clone()));
            Ok(())
        }

        fn bind(&mut self, owner: &mut Owner, cq: u32) -> Result<(), IpcError> {
            assert_eq!(cq, 7);
            assert!(owner.pending.is_some());
            if self.bind_error {
                Err(IpcError::Status(99))
            } else {
                Ok(())
            }
        }

        fn poll(&mut self, owner: &mut Owner) -> Result<Progress, i64> {
            assert!(owner.pending.is_some());
            let result = self.replies[owner.id].pop_front().unwrap_or(Ok(Progress::Pending));
            if result != Ok(Progress::Pending) {
                drop(owner.pending.take());
            }
            if result == Ok(Progress::Next) {
                owner.phase += 1;
            }
            result
        }

        fn finish(&mut self, mut owner: Owner, code: Option<i64>) {
            drop(owner.pending.take());
            drop(owner.mapping.take());
            drop(owner.reply.take());
            self.results.push((owner.id, code));
        }

        fn abort(&mut self, owner: &Owner, _error: IpcError) -> ! {
            assert!(owner.pending.is_some() && owner.mapping.is_some() && owner.reply.is_some());
            assert!(self.log.borrow().is_empty());
            panic!("relay retains all fields at abort")
        }
    }
    fn machine() -> Machine<Fake> {
        Machine::new(
            Fake {
                now: Cell::new(0),
                log: Rc::new(RefCell::new(Vec::new())),
                replies: core::array::from_fn(|_| VecDeque::new()),
                begin_error: None,
                bind_error: false,
                preparation_ms: 0,
                submitted: Vec::new(),
                results: Vec::new(),
            },
            7,
        )
    }
    fn start(m: &mut Machine<Fake>, id: usize, bytes: usize) {
        let log = m.calls.log.clone();
        m.start(
            Owner {
                id,
                pending: None,
                mapping: Some(Resource("mapping", id, log.clone())),
                reply: Some(Resource("reply", id, log)),
                phase: 0,
                bytes,
            },
            Deadline::at(m.calls.now.get(), 1000, RELAY_MS),
        );
    }
    #[test]
    fn held_dns_reply_does_not_block_another_request_and_reply_follows_local_cleanup() {
        let mut m = machine();
        start(&mut m, 0, 8192);
        start(&mut m, 1, 8192);
        m.calls.replies[1].push_back(Ok(Progress::Complete));
        assert!(m.advance());
        assert_eq!(m.calls.results, [(1, None)]);
        assert_eq!(m.reserved, 8192);
        assert_eq!(&*m.calls.log.borrow(), &[("pending", 1), ("mapping", 1), ("reply", 1)]);
        assert!(m.entries[0].as_ref().unwrap().owner.pending.is_some());
    }
    #[test]
    fn preflight_and_mutation_share_original_budget_without_replay() {
        let mut m = machine();
        start(&mut m, 0, 8192);
        m.calls.now.set(4000);
        m.calls.replies[0].push_back(Ok(Progress::Next));
        assert!(m.advance());
        assert_eq!(m.remaining_ms(), Some(1000));
        assert_eq!(m.calls.submitted, [(0, 0), (0, 1)]);
        for _ in 0..8 {
            assert!(!m.advance());
        }
        m.calls.now.set(5000);
        assert!(m.advance());
        assert_eq!(m.calls.results, [(0, Some(clusterctl::ERR_RPC_TIMEOUT))]);
        assert_eq!(m.calls.submitted, [(0, 0), (0, 1)]);
        assert_eq!(m.reserved, 0);
    }
    #[test]
    fn expired_preflight_never_submits_a_mutation() {
        let mut m = machine();
        start(&mut m, 0, 8192);
        m.calls.now.set(5000);
        m.calls.replies[0].push_back(Ok(Progress::Next));
        assert!(m.advance());
        assert_eq!(m.calls.submitted, [(0, 0)]);
        assert_eq!(m.calls.results, [(0, Some(clusterctl::ERR_RPC_TIMEOUT))]);
    }
    #[test]
    fn failed_preflight_or_observation_finishes_without_submitting_next_phase() {
        for code in
            [dns::ERR_UNCERTAIN, clusterctl::ERR_NOT_LEADER, clusterctl::ERR_STALE_DESCRIPTOR]
        {
            let mut m = machine();
            start(&mut m, 0, 8192);
            m.calls.replies[0].push_back(Err(code));
            assert!(m.advance());
            assert_eq!(m.calls.submitted, [(0, 0)]);
            assert_eq!(m.calls.results, [(0, Some(code))]);
            assert_eq!(m.reserved, 0);
        }
    }
    #[test]
    fn preparation_mapping_and_submission_rejection_end_unstarted_owners_once() {
        for code in [clusterctl::ERR_UPLOAD_FAILED, clusterctl::ERR_NOT_LEADER] {
            let mut m = machine();
            m.calls.begin_error = Some(code);
            start(&mut m, 0, 8192);
            assert_eq!(m.calls.results, [(0, Some(code))]);
            assert!(m.calls.submitted.is_empty());
            assert_eq!(m.reserved, 0);
            assert_eq!(&*m.calls.log.borrow(), &[("mapping", 0), ("reply", 0)]);
        }
    }
    #[test]
    fn preparation_time_counts_before_submission_and_binding_rejection_retains_pool() {
        let mut m = machine();
        m.calls.preparation_ms = 5000;
        start(&mut m, 0, 8192);
        assert!(m.calls.submitted.is_empty());
        assert_eq!(m.calls.results, [(0, Some(clusterctl::ERR_RPC_TIMEOUT))]);
        let mut m = machine();
        m.calls.bind_error = true;
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| start(&mut m, 0, 8192)))
                .is_err()
        );
        assert_eq!(m.reserved, 8192);
        assert!(m.entries[0].as_ref().unwrap().owner.pending.is_some());
    }
    #[test]
    fn count_and_backing_ceiling_reject_before_submission() {
        let mut m = machine();
        for id in 0..MAX_RELAYS {
            start(&mut m, id, 8192);
        }
        start(&mut m, 0, 8192);
        assert_eq!(m.calls.submitted.len(), MAX_RELAYS);
        assert_eq!(m.calls.results, [(0, Some(clusterctl::ERR_CAPACITY))]);
        assert_eq!(m.reserved, 8 * 8192);
        let mut m = machine();
        start(&mut m, 0, MAX_REQUEST_BYTES);
        start(&mut m, 1, 8192);
        assert_eq!(m.calls.submitted, [(0, 0)]);
        assert_eq!(m.calls.results, [(1, Some(clusterctl::ERR_CAPACITY))]);
        assert_eq!(m.reserved, MAX_REQUEST_BYTES);
    }
    #[test]
    fn protocol_limits_account_original_and_copy_and_reject_oversized_or_foreign_attachments() {
        assert_eq!(
            request_reservation(clusterctl::OP_NOTIFY_OPERATIONS, 1048576, false),
            Ok(2097152)
        );
        for opcode in [
            clusterctl::OP_NOTIFY,
            clusterctl::OP_NOTIFY_RELEASE,
            clusterctl::OP_STATUS,
            clusterctl::OP_ROLLOUT,
        ] {
            assert_eq!(request_reservation(opcode, 4096, false), Ok(8192));
            assert!(request_reservation(opcode, 8192, false).is_err());
            assert!(request_reservation(opcode, 4096, true).is_err());
        }
        assert_eq!(request_reservation(clusterctl::OP_INGRESS_POLICY_STATUS, 0, false), Ok(0));
        assert!(request_reservation(clusterctl::OP_INGRESS_POLICY_STATUS, 4096, false).is_err());
        assert!(payload_len(clusterctl::OP_NOTIFY, 0, &u64::MAX.to_le_bytes()).is_err());
        assert!(payload_len(clusterctl::OP_STATUS, 49, &[0; 4096]).is_err());
        assert!(reply_bound(clusterctl::OP_ROLLOUT, clusterctl::ROLLOUT_STATUS_LEN, 4096));
        assert!(!reply_bound(clusterctl::OP_ROLLOUT, clusterctl::ROLLOUT_STATUS_LEN, 8192));
        assert!(!reply_bound(clusterctl::OP_STATUS, usize::MAX, 4096));
    }
    fn descriptor(name: &[u8], sequence: u64) -> Vec<u8> {
        use charlotte_launch::{
            deployment as d,
            execution_target::{
                Architecture,
                ExecutionTarget,
            },
            placement::PlacementPolicy,
        };
        let variants = [d::ArtifactVariant {
            target: ExecutionTarget::baseline(Architecture::Aarch64),
            priority: 0,
            artifact_digest: [1; 32],
            object_key: b"probe.elf",
        }];
        let fields = d::DescriptorFields {
            sequence,
            node_key: 1,
            variants: &variants,
            artifact_name: name,
            stack_pages_per_thread: 4,
            max_threads: 1,
            shutdown_grace_ms: 5000,
            placement: PlacementPolicy::singleton(),
            grants: &[],
        };
        let mut bytes = vec![0; d::encoded_len(&fields).unwrap()];
        d::encode_unsigned(&fields, &[1; 32], &mut bytes).unwrap();
        bytes
    }
    fn record(descriptor: Vec<u8>, generation: u64) -> Vec<u8> {
        name_catalog::encode_deployment_result(&name_catalog::DeploymentEntry {
            generation,
            descriptor,
            replica_nodes: vec![1],
            replica_variants: vec![0],
        })
        .unwrap()
    }
    #[test]
    fn only_explicit_absence_or_checked_newer_revision_permits_submission() {
        let wanted = descriptor(b"probe", 2);
        assert_eq!(preflight(&wanted, dns::ERR_NOT_FOUND, None), Ok(None));
        assert_eq!(preflight(&wanted, dns::ERR_UNCERTAIN, None), Err(dns::ERR_UNCERTAIN));
        assert_eq!(
            preflight(&wanted, clusterctl::ERR_RPC_TIMEOUT, None),
            Err(clusterctl::ERR_RPC_TIMEOUT)
        );
        assert_eq!(preflight(&wanted, 22, None), Err(dns::ERR_UNCERTAIN));
        assert_eq!(preflight(&wanted, 22, Some(&[0; 22])), Err(dns::ERR_UNCERTAIN));
        let older = record(descriptor(b"probe", 1), 1);
        assert_eq!(preflight(&wanted, older.len() as i64, Some(&older)), Ok(None));
        let foreign = record(descriptor(b"other", 1), 1);
        assert_eq!(
            preflight(&wanted, foreign.len() as i64, Some(&foreign)),
            Err(dns::ERR_UNCERTAIN)
        );
        let same = record(wanted.clone(), 7);
        assert_eq!(preflight(&wanted, same.len() as i64, Some(&same)), Ok(Some(7)));
        let newer = record(descriptor(b"probe", 3), 8);
        assert_eq!(
            preflight(&wanted, newer.len() as i64, Some(&newer)),
            Err(clusterctl::ERR_STALE_DESCRIPTOR)
        );
        let mut changed = wanted.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        let conflict = record(changed, 8);
        assert_eq!(
            preflight(&wanted, conflict.len() as i64, Some(&conflict)),
            Err(clusterctl::ERR_CONFLICTING_DESCRIPTOR)
        );
    }
}
