//! CQ-bound socket creation, listen setup and idle accept in the client pool.
use catten_rt::owned::{
    ConnectionRef,
    IpcError,
    PendingCall,
};

use crate::{
    deadline::Deadline,
    deployment_input::MAX_CLIENTS,
    socket::{
        self,
        OpeningSocket,
        OwnedSocket,
        SocketError,
    },
};

pub const SETUP_MS: u64 = 5_000;
pub const ADMISSION_BACKOFF_MS: u64 = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Open,
    Listen,
    Accept,
}
#[derive(Debug)]
enum Failure {
    Deadline,
    Binding(IpcError),
    Observation(SocketError),
}

trait Calls {
    type Owner;
    type Socket;
    fn now(&self) -> (u64, u64);
    fn owner(&self) -> Self::Owner;
    fn submit(&mut self, owner: &mut Self::Owner, stage: Stage) -> Result<(), SocketError>;
    fn bind(&mut self, owner: &mut Self::Owner, stage: Stage, cq: u32) -> Result<(), IpcError>;
    fn poll(&mut self, owner: &mut Self::Owner, stage: Stage) -> Result<bool, SocketError>;
    fn take_socket(&mut self, owner: &mut Self::Owner) -> Option<Self::Socket>;
    fn abort(&mut self, owner: &Self::Owner, failure: Failure) -> !;
}
struct Entry<O> {
    owner: O,
    stage: Stage,
    accepted: bool,
    deadline: Deadline,
    retry: Option<Deadline>,
}
struct Machine<C: Calls> {
    entries: [Option<Entry<C::Owner>>; MAX_CLIENTS],
    calls: C,
    cq: u32,
    cursor: usize,
    retry: Option<Deadline>,
}
impl<C: Calls> Machine<C> {
    fn new(calls: C, cq: u32) -> Self {
        Self {
            entries: core::array::from_fn(|_| None),
            calls,
            cq,
            cursor: 0,
            retry: None,
        }
    }

    fn occupied_slots(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    fn accepting(&self) -> bool {
        self.entries.iter().flatten().any(|e| e.stage == Stage::Accept && e.accepted)
    }

    fn replenish(&mut self, available: usize) {
        let (now, frequency) = self.calls.now();
        if self.retry.is_some_and(|d| !d.expired_at(now)) {
            return;
        }
        self.retry = None;
        let mut count = self.occupied_slots();
        for slot in &mut self.entries {
            if count >= available {
                break;
            }
            if slot.is_none() {
                *slot = Some(Entry {
                    owner: self.calls.owner(),
                    stage: Stage::Open,
                    accepted: false,
                    deadline: Deadline::at(now, frequency, SETUP_MS),
                    retry: None,
                });
                count += 1;
            }
        }
    }

    // One bounded transition per turn keeps existing clients and lifecycle checks
    // ahead of another setup attempt. Only rejected admission is retried.
    fn advance(&mut self, draining: bool) -> (bool, Option<(C::Socket, bool)>) {
        let (now, frequency) = self.calls.now();
        let cursor = self.cursor;
        for offset in 0..MAX_CLIENTS {
            let index = (cursor + offset) % MAX_CLIENTS;
            let Some(entry) = self.entries[index].as_mut() else {
                continue;
            };
            if draining && (!entry.accepted || entry.stage == Stage::Accept) {
                let socket = self.calls.take_socket(&mut entry.owner);
                self.entries[index] = None;
                self.cursor = (index + 1) % MAX_CLIENTS;
                return (true, socket.map(|s| (s, false)));
            }
            // Idle, admitted accept has no timer. Setup retains its original
            // deadline through submission, CQ binding and both observed replies.
            if !(entry.stage == Stage::Accept && entry.accepted) && entry.deadline.expired_at(now) {
                self.calls.abort(&entry.owner, Failure::Deadline);
            }
            if entry.retry.is_some_and(|d| !d.expired_at(now)) {
                continue;
            }
            self.cursor = (index + 1) % MAX_CLIENTS;
            if !entry.accepted {
                match self.calls.submit(&mut entry.owner, entry.stage) {
                    Ok(()) => {
                        entry.accepted = true;
                        entry.retry = None;
                        if let Err(error) = self.calls.bind(&mut entry.owner, entry.stage, self.cq)
                        {
                            self.calls.abort(&entry.owner, Failure::Binding(error));
                        }
                        if entry.deadline.expired_at(self.calls.now().0) {
                            self.calls.abort(&entry.owner, Failure::Deadline);
                        }
                    }
                    Err(SocketError::Ipc(IpcError::Status(
                        catten_syscall::ipc_status::QUEUE_FULL,
                    ))) => {
                        entry.retry = Some(Deadline::at(now, frequency, ADMISSION_BACKOFF_MS));
                    }
                    Err(_) => {
                        // No call was accepted. A known socket moves into the
                        // existing close pool; creation rejection owns no socket.
                        return self.reject(index, now, frequency);
                    }
                }
                return (true, None);
            }
            match self.calls.poll(&mut entry.owner, entry.stage) {
                Ok(false) => continue,
                Ok(true) => match entry.stage {
                    Stage::Open if draining => {
                        let socket =
                            self.calls.take_socket(&mut entry.owner).expect("observed socket");
                        self.entries[index] = None;
                        return (true, Some((socket, false)));
                    }
                    Stage::Open => {
                        entry.stage = Stage::Listen;
                        entry.accepted = false;
                    }
                    Stage::Listen => {
                        entry.stage = Stage::Accept;
                        entry.accepted = false;
                    }
                    Stage::Accept => {
                        let socket =
                            self.calls.take_socket(&mut entry.owner).expect("accepted socket");
                        self.entries[index] = None;
                        return (true, Some((socket, true)));
                    }
                },
                Err(SocketError::Service(_)) => return self.reject(index, now, frequency),
                Err(error) => self.calls.abort(&entry.owner, Failure::Observation(error)),
            }
            return (true, None);
        }
        (false, None)
    }

    fn reject(
        &mut self,
        index: usize,
        now: u64,
        frequency: u64,
    ) -> (bool, Option<(C::Socket, bool)>) {
        let socket = self.calls.take_socket(&mut self.entries[index].as_mut().unwrap().owner);
        self.entries[index] = None;
        self.retry = Some(Deadline::at(now, frequency, ADMISSION_BACKOFF_MS));
        (true, socket.map(|s| (s, false)))
    }

    fn remaining_ms(&self, replenishing: bool) -> Option<u64> {
        let (now, _) = self.calls.now();
        self.entries
            .iter()
            .flatten()
            .filter(|e| !(e.stage == Stage::Accept && e.accepted))
            .map(|e| {
                let left = e.deadline.remaining_ms_at(now);
                e.retry.map_or(left, |r| left.min(r.remaining_ms_at(now)))
            })
            .chain(
                if replenishing {
                    self.retry.map(|r| r.remaining_ms_at(now))
                } else {
                    None
                },
            )
            .min()
    }
}
struct Owner<'a> {
    opening: Option<OpeningSocket<'a>>,
    pending: Option<PendingCall<'static>>,
    socket: Option<OwnedSocket<'a>>,
}
impl Drop for Owner<'_> {
    fn drop(&mut self) {
        if self.opening.is_some() || self.pending.is_some() || self.socket.is_some() {
            catten_rt::domain_abort();
        }
    }
}
struct Runtime<'a> {
    tcp: ConnectionRef<'a>,
}
impl<'a> Calls for Runtime<'a> {
    type Owner = Owner<'a>;
    type Socket = OwnedSocket<'a>;

    fn now(&self) -> (u64, u64) {
        catten_syscall::monotonic_clock()
    }

    fn owner(&self) -> Self::Owner {
        Owner {
            opening: None,
            pending: None,
            socket: None,
        }
    }

    fn submit(&mut self, owner: &mut Owner<'a>, stage: Stage) -> Result<(), SocketError> {
        match stage {
            Stage::Open => {
                owner.opening = Some(OwnedSocket::open_call(self.tcp, socket::DOMAIN_TCP)?)
            }
            Stage::Listen => {
                owner.pending = Some(
                    owner.socket.as_ref().unwrap().listen_call(crate::clusterctl::NOTIFY_PORT)?,
                )
            }
            Stage::Accept => owner.pending = Some(owner.socket.as_ref().unwrap().accept_call()?),
        }
        Ok(())
    }

    fn bind(&mut self, owner: &mut Owner<'a>, stage: Stage, cq: u32) -> Result<(), IpcError> {
        match stage {
            Stage::Open => owner.opening.as_mut().unwrap().bind_cq(cq),
            _ => owner.pending.as_mut().unwrap().bind_cq(cq),
        }
    }

    fn poll(&mut self, owner: &mut Owner<'a>, stage: Stage) -> Result<bool, SocketError> {
        if stage == Stage::Open {
            match owner.opening.as_mut().unwrap().poll() {
                Ok(Some(socket)) => {
                    owner.socket = Some(socket);
                    drop(owner.opening.take());
                    return Ok(true);
                }
                Err(error @ SocketError::Service(_)) => {
                    drop(owner.opening.take());
                    return Err(error);
                }
                Ok(None) => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        let Some(reply) = owner.pending.as_mut().unwrap().poll()? else {
            return Ok(false);
        };
        if reply.memory.is_some() || reply.connection.is_some() || reply.result > 0 {
            return Err(SocketError::InvalidReply);
        }
        drop(owner.pending.take());
        if reply.result == 0 {
            Ok(true)
        } else {
            Err(SocketError::Service(reply.result))
        }
    }

    fn take_socket(&mut self, owner: &mut Owner<'a>) -> Option<Self::Socket> {
        // Only confirmed replies, rejected submissions or an explicitly cancelled
        // idle accept enter here. No opening request is discarded unobserved.
        assert!(owner.opening.is_none());
        drop(owner.pending.take());
        owner.socket.take()
    }

    fn abort(&mut self, _owner: &Owner<'a>, failure: Failure) -> ! {
        match failure {
            Failure::Binding(error) => {
                catten_rt::logln!("[deployd] listener binding retained: {error:?}")
            }
            Failure::Observation(error) => {
                catten_rt::logln!("[deployd] listener observation retained: {error:?}")
            }
            Failure::Deadline => {
                catten_rt::logln!("[deployd] listener setup deadline; owner retained")
            }
        }
        catten_rt::domain_abort()
    }
}
/// Setup and idle accepts consume the same positions as input/control/output/close.
pub struct Listeners<'a>(Machine<Runtime<'a>>);
impl<'a> Listeners<'a> {
    pub fn new(tcp: ConnectionRef<'a>, cq: u32) -> Self {
        Self(Machine::new(
            Runtime {
                tcp,
            },
            cq,
        ))
    }

    pub fn replenish(&mut self, available: usize) {
        self.0.replenish(available);
    }

    pub fn advance(&mut self, draining: bool) -> (bool, Option<(OwnedSocket<'a>, bool)>) {
        self.0.advance(draining)
    }

    pub fn occupied_slots(&self) -> usize {
        self.0.occupied_slots()
    }

    pub fn accepting(&self) -> bool {
        self.0.accepting()
    }

    pub fn remaining_ms(&self, replenishing: bool) -> Option<u64> {
        self.0.remaining_ms(replenishing)
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
    type Log = Rc<RefCell<Vec<(&'static str, usize, Stage)>>>;
    struct Socket(usize, Log);
    impl Drop for Socket {
        fn drop(&mut self) {
            self.1.borrow_mut().push(("socket", self.0, Stage::Open));
        }
    }
    struct Pending(usize, Stage, Log);
    impl Drop for Pending {
        fn drop(&mut self) {
            self.2.borrow_mut().push(("pending", self.0, self.1));
        }
    }
    struct Owner {
        id: usize,
        pending: Option<Pending>,
        socket: Option<Socket>,
    }
    struct Fake {
        now: Cell<u64>,
        next: Cell<usize>,
        log: Log,
        replies: [VecDeque<Result<bool, SocketError>>; MAX_CLIENTS],
        rejection: Option<SocketError>,
        binding: bool,
        submit_ms: u64,
        bound: Vec<(usize, Stage)>,
    }
    impl Calls for Fake {
        type Owner = Owner;
        type Socket = Socket;

        fn now(&self) -> (u64, u64) {
            (self.now.get(), 1000)
        }

        fn owner(&self) -> Owner {
            let id = self.next.get();
            self.next.set(id + 1);
            Owner {
                id,
                pending: None,
                socket: None,
            }
        }

        fn submit(&mut self, owner: &mut Owner, stage: Stage) -> Result<(), SocketError> {
            assert!(owner.pending.is_none(), "accepted work must never be resubmitted");
            self.log.borrow_mut().push(("submit", owner.id, stage));
            self.now.set(self.now.get() + self.submit_ms);
            if let Some(error) = self.rejection.take() {
                return Err(error);
            }
            owner.pending = Some(Pending(owner.id, stage, self.log.clone()));
            Ok(())
        }

        fn bind(&mut self, owner: &mut Owner, stage: Stage, cq: u32) -> Result<(), IpcError> {
            assert_eq!(cq, 7);
            assert!(owner.pending.is_some());
            self.bound.push((owner.id, stage));
            if self.binding {
                Err(IpcError::Status(99))
            } else {
                Ok(())
            }
        }

        fn poll(&mut self, owner: &mut Owner, stage: Stage) -> Result<bool, SocketError> {
            assert_eq!(owner.pending.as_ref().unwrap().1, stage);
            let reply = self.replies[owner.id].pop_front().unwrap_or(Ok(false));
            if matches!(reply, Ok(true) | Err(SocketError::Service(_))) {
                drop(owner.pending.take());
                if stage == Stage::Open && reply == Ok(true) {
                    owner.socket = Some(Socket(owner.id, self.log.clone()));
                }
            }
            reply
        }

        fn take_socket(&mut self, owner: &mut Owner) -> Option<Socket> {
            assert!(!owner.pending.as_ref().is_some_and(|p| p.1 == Stage::Open));
            drop(owner.pending.take());
            owner.socket.take()
        }

        fn abort(&mut self, owner: &Owner, failure: Failure) -> ! {
            // Verify retained fields at the boundary, before test unwinding.
            assert!(
                !self
                    .log
                    .borrow()
                    .iter()
                    .any(|&(action, id, _)| action == "socket" && id == owner.id)
            );
            if matches!(failure, Failure::Binding(_) | Failure::Observation(_)) {
                assert!(owner.pending.is_some());
            }
            panic!("listener retained: {failure:?}")
        }
    }
    fn machine(count: usize) -> Machine<Fake> {
        let mut m = Machine::new(
            Fake {
                now: Cell::new(0),
                next: Cell::new(0),
                log: Rc::new(RefCell::new(Vec::new())),
                replies: core::array::from_fn(|_| VecDeque::new()),
                rejection: None,
                binding: false,
                submit_ms: 0,
                bound: Vec::new(),
            },
            7,
        );
        m.replenish(count);
        m
    }
    fn step(m: &mut Machine<Fake>, id: usize, reply: bool) {
        m.cursor = id;
        if reply {
            m.calls.replies[id].push_back(Ok(true));
        }
        assert!(m.advance(false).0);
    }
    fn ready(m: &mut Machine<Fake>, id: usize) {
        for reply in [false, true, false, true, false] {
            step(m, id, reply);
        }
        assert!(m.entries[id].as_ref().unwrap().accepted);
        assert_eq!(m.entries[id].as_ref().unwrap().stage, Stage::Accept);
    }
    fn panics(f: impl FnOnce()) {
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err());
    }
    #[test]
    fn delayed_creation_does_not_block_another_accept_and_all_phases_bind_once() {
        let mut m = machine(2);
        step(&mut m, 0, false);
        ready(&mut m, 1);
        m.calls.replies[1].push_back(Ok(true));
        let (_, result) = m.advance(false);
        let (socket, accepted) = result.unwrap();
        assert_eq!(socket.0, 1);
        assert!(accepted);
        assert_eq!(m.occupied_slots(), 1);
        assert!(m.entries[0].as_ref().unwrap().owner.pending.is_some());
        assert_eq!(
            m.calls.bound,
            [(0, Stage::Open), (1, Stage::Open), (1, Stage::Listen), (1, Stage::Accept)]
        );
    }
    #[test]
    fn setup_has_one_deadline_while_idle_accept_is_event_only() {
        let mut m = machine(1);
        step(&mut m, 0, false);
        m.calls.now.set(4000);
        step(&mut m, 0, true);
        assert_eq!(m.remaining_ms(false), Some(1000));
        step(&mut m, 0, false);
        m.calls.now.set(4999);
        step(&mut m, 0, true);
        step(&mut m, 0, false);
        assert_eq!(m.remaining_ms(false), None);
        m.calls.now.set(100000);
        assert!(!m.advance(false).0);
        assert!(m.accepting());
    }
    #[test]
    fn queue_full_is_retried_only_after_backoff_without_resetting_setup() {
        let mut m = machine(1);
        m.calls.rejection =
            Some(SocketError::Ipc(IpcError::Status(catten_syscall::ipc_status::QUEUE_FULL)));
        step(&mut m, 0, false);
        assert_eq!(m.remaining_ms(false), Some(50));
        m.calls.now.set(49);
        assert!(!m.advance(false).0);
        m.calls.now.set(50);
        step(&mut m, 0, false);
        assert_eq!(m.remaining_ms(false), Some(4950));
        assert!(!m.advance(false).0);
        assert_eq!(m.calls.bound, [(0, Stage::Open)]);
        assert_eq!(m.calls.log.borrow().iter().filter(|(a, _, _)| *a == "submit").count(), 2);
    }
    #[test]
    fn delayed_open_and_listen_timeout_keep_accepted_owners_without_replay() {
        for listening in [false, true] {
            let mut m = machine(1);
            step(&mut m, 0, false);
            if listening {
                step(&mut m, 0, true);
                step(&mut m, 0, false);
            }
            let before = m.calls.bound.len();
            m.calls.now.set(SETUP_MS);
            panics(|| {
                m.advance(false);
            });
            assert!(m.entries[0].as_ref().unwrap().owner.pending.is_some());
            assert_eq!(m.calls.bound.len(), before);
        }
    }
    #[test]
    fn setup_admission_cost_cannot_escape_deadline_at_accept_transition() {
        let mut m = machine(1);
        for reply in [false, true, false, true] {
            step(&mut m, 0, reply);
        }
        m.calls.now.set(4999);
        m.calls.submit_ms = 1;
        panics(|| {
            m.advance(false);
        });
        assert_eq!(m.calls.bound.last(), Some(&(0, Stage::Accept)));
        assert!(m.entries[0].as_ref().unwrap().owner.pending.is_some());
    }
    #[test]
    fn confirmed_rejection_returns_known_socket_to_close_and_cools_replenishment() {
        for stage in [Stage::Open, Stage::Listen, Stage::Accept] {
            let mut m = machine(1);
            step(&mut m, 0, false);
            if stage != Stage::Open {
                step(&mut m, 0, true);
                step(&mut m, 0, false);
            }
            if stage == Stage::Accept {
                step(&mut m, 0, true);
                step(&mut m, 0, false);
            }
            m.calls.replies[0].push_back(Err(SocketError::Service(-8)));
            let (progress, result) = m.advance(false);
            assert!(progress);
            assert_eq!(
                result.as_ref().map(|(s, accepted)| (s.0, *accepted)),
                if stage == Stage::Open {
                    None
                } else {
                    Some((0, false))
                }
            );
            assert_eq!(m.occupied_slots(), 0);
            assert_eq!(m.remaining_ms(true), Some(50));
            m.replenish(1);
            assert_eq!(m.occupied_slots(), 0);
            assert!(!m.calls.log.borrow().iter().any(|&(a, _, _)| a == "socket"));
            drop(result);
        }
    }
    #[test]
    fn mapping_and_submission_rejection_return_socket_without_destructor_close() {
        for error in [
            SocketError::Memory(catten_rt::owned::MemoryError::MemoryStatus(99)),
            SocketError::Ipc(IpcError::Status(99)),
        ] {
            let mut m = machine(1);
            step(&mut m, 0, false);
            step(&mut m, 0, true);
            m.calls.rejection = Some(error);
            let (_, result) = m.advance(false);
            let (socket, accepted) = result.unwrap();
            assert!(!accepted);
            assert_eq!(socket.0, 0);
            assert!(m.entries[0].is_none());
            assert_eq!(m.calls.bound, [(0, Stage::Open)]);
            assert!(!m.calls.log.borrow().iter().any(|&(a, _, _)| a == "socket"));
        }
    }
    #[test]
    fn binding_and_uncertain_observation_fail_with_all_fields_live() {
        for stage in [Stage::Open, Stage::Listen, Stage::Accept] {
            for binding in [false, true] {
                let mut m = machine(1);
                if stage != Stage::Open {
                    step(&mut m, 0, false);
                    step(&mut m, 0, true);
                }
                if stage == Stage::Accept {
                    step(&mut m, 0, false);
                    step(&mut m, 0, true);
                }
                m.calls.binding = binding;
                if !binding {
                    step(&mut m, 0, false);
                    m.calls.replies[0].push_back(Err(SocketError::InvalidReply));
                }
                panics(|| {
                    m.advance(false);
                });
                assert!(m.entries[0].as_ref().unwrap().owner.pending.is_some());
            }
        }
    }
    #[test]
    fn shutdown_observes_creation_but_cancels_idle_accept_before_returning_socket() {
        let mut m = machine(2);
        step(&mut m, 0, false);
        ready(&mut m, 1);
        m.cursor = 0;
        let (_, result) = m.advance(true);
        assert_eq!(result.as_ref().map(|(s, a)| (s.0, *a)), Some((1, false)));
        assert!(m.entries[0].as_ref().unwrap().owner.pending.is_some());
        m.calls.replies[0].push_back(Ok(true));
        let (_, result) = m.advance(true);
        assert_eq!(result.as_ref().map(|(s, a)| (s.0, *a)), Some((0, false)));
        assert_eq!(m.occupied_slots(), 0);
        assert_eq!(m.calls.bound.iter().filter(|&&(id, _)| id == 0).count(), 1);
    }
    #[test]
    fn setup_respects_existing_client_positions_without_additional_capacity() {
        let mut m = machine(2);
        m.replenish(2);
        assert_eq!(m.occupied_slots(), 2);
        m.replenish(0);
        assert_eq!(m.occupied_slots(), 2);
        m.replenish(MAX_CLIENTS);
        assert_eq!(m.occupied_slots(), MAX_CLIENTS);
        m.replenish(MAX_CLIENTS + 1);
        assert_eq!(m.occupied_slots(), MAX_CLIENTS);
    }
}
