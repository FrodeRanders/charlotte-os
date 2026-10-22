//! Socket records own deferred reply authority and remote-resource metadata.
use alloc::collections::BTreeMap;

use catten_rt::owned::{
    IncomingMessage,
    MappedMemory,
    MemoryError,
    ReadOnly,
    Writable,
};
use smoltcp::{
    iface::SocketSet,
    socket::{
        tcp::Socket as TcpSocket,
        udp::Socket as UdpSocket,
    },
    wire::IpEndpoint,
};

use crate::{
    network_random::NetworkRandom,
    socket_lifetime::{
        SocketLifetime,
        SocketOwner,
    },
};

/// Completion of the listening socket's handshake, including peer half-close.
/// SYN-RECEIVED still belongs to the pending accept operation.
pub fn accept_result(state: smoltcp::socket::tcp::State) -> Option<i64> {
    use smoltcp::socket::tcp::State;
    match state {
        State::Listen | State::SynReceived => None,
        State::Established | State::CloseWait => Some(0),
        _ => Some(crate::socket::ERR_CONNECTION_REFUSED),
    }
}

pub fn receive_eof(state: smoltcp::socket::tcp::State, can_recv: bool) -> bool {
    use smoltcp::socket::tcp::State;
    !can_recv
        && matches!(
            state,
            State::CloseWait | State::Closing | State::LastAck | State::TimeWait | State::Closed
        )
}

pub fn reply_accept(mut request: IncomingMessage, result: i64) {
    if let Some(reply) = request.reply.take() {
        let _ = reply.reply(result);
    }
}

pub struct SocketEntry {
    pub handle: smoltcp::iface::SocketHandle,
    pub kind: SocketKind,
    pub owner: SocketOwner,
    pub buffer_bytes: usize,
    pub lifetime: SocketLifetime,
    /// Set once the client has successfully bound, listened, or connected
    /// the socket. smoltcp reports a newly allocated but not-yet-configured
    /// socket as closed, so that state must not be reaped between OP_SOCKET
    /// and the follow-up operation.
    pub activated: bool,
    /// UDP has no connected state in smoltcp. The service remembers the peer
    /// selected by `OP_CONNECT` and filters received datagrams to it.
    pub udp_remote: Option<IpEndpoint>,
    pub recv_pending: Option<PendingReceive>,
    pub accept_pending: Option<IncomingMessage>,
    pub send_pending: Option<PendingSend>,
    /// Set by `OP_CLOSE`: the socket was gracefully closed and may be swept
    /// from the set once it reaches a final state.
    pub closing: bool,
    /// Reactor time at which graceful close was requested.  This is used to
    /// bound how long a peer that never completes the FIN handshake can hold
    /// the owner's socket quota.
    pub close_started_ms: Option<u64>,
}

/// Both send forms own a complete moved request and its read-only mapping.
/// Deferred TCP may retain it while full; immediate TCP/UDP attempts once.
/// Mapping access ends before reply/cancellation; enqueue progress fences replay.
pub struct PendingSend {
    request: IncomingMessage,
    mapping: Option<MappedMemory<ReadOnly>>,
    enqueue: crate::socket_send::Enqueue,
}
impl PendingSend {
    // Rejection returns the complete transaction without another allocation.
    #[allow(clippy::result_large_err)]
    pub fn prepare(request: IncomingMessage) -> Result<Self, (Self, i64)> {
        let mut owner = Self {
            request,
            mapping: None,
            enqueue: crate::socket_send::Enqueue::default(),
        };
        if owner.request.connection.is_some()
            || !owner
                .request
                .memory
                .as_ref()
                .is_some_and(|memory| memory.len() == crate::socket_send::FRAME_BYTES)
        {
            return Err((owner, crate::socket::ERR_BAD_OPCODE));
        }
        let memory = owner.request.memory.take().unwrap();
        match memory.map_read_only() {
            Ok(mapping) => owner.mapping = Some(mapping),
            Err((memory, _)) => {
                owner.request.memory = Some(memory);
                return Err((owner, crate::socket::ERR_BAD_OPCODE));
            }
        }
        if crate::socket_send::decode(owner.mapping.as_ref().unwrap().as_slice()).is_none() {
            return Err((owner, crate::socket::ERR_BAD_OPCODE));
        }
        Ok(owner)
    }

    pub fn write(&mut self, send: impl FnOnce(&[u8]) -> Result<usize, i64>) -> Result<usize, i64> {
        let bytes = crate::socket_send::decode(self.mapping.as_ref().unwrap().as_slice())
            .expect("validated immutable send frame");
        self.enqueue.attempt(|| send(bytes))
    }

    pub fn reply(mut self, result: i64) {
        drop(self.mapping.take());
        drop(self.request.memory.take());
        if let Some(reply) = self.request.reply.take() {
            let _ = reply.reply(result);
        }
    }
}
impl Drop for PendingSend {
    fn drop(&mut self) {
        drop(self.mapping.take());
    }
}

/// Handshaking/full sockets keep the accepted request. Terminal states reject.
pub fn send_ready(state: smoltcp::socket::tcp::State, can_send: bool) -> Option<bool> {
    use smoltcp::socket::tcp::State;
    match state {
        State::SynSent | State::SynReceived => Some(false),
        State::Established | State::CloseWait => Some(can_send),
        _ => None,
    }
}

/// Own the complete receive transaction, including caller-sponsored backing,
/// transient mapping and reply/loan authority. No service page is allocated.
pub struct PendingReceive {
    request: IncomingMessage,
    mapping: Option<MappedMemory<Writable>>,
}

impl PendingReceive {
    pub fn new(request: IncomingMessage) -> Self {
        Self {
            request,
            mapping: None,
        }
    }

    pub fn valid_buffer(&self) -> bool {
        self.request
            .memory
            .as_ref()
            .is_some_and(|memory| memory.len() == crate::socket::REPLY_BUFFER_BYTES)
    }

    pub fn fill(
        &mut self,
        read: impl FnOnce(&mut [u8]) -> Option<usize>,
    ) -> Result<Option<usize>, MemoryError> {
        let memory = self.request.memory.take().expect("validated receive buffer");
        match memory.map_writable() {
            Ok(mapping) => self.mapping = Some(mapping),
            Err((memory, error)) => {
                self.request.memory = Some(memory);
                return Err(error);
            }
        }
        let result = read(self.mapping.as_mut().unwrap().as_mut_slice());
        match self.mapping.take().unwrap().unmap() {
            Ok(memory) => self.request.memory = Some(memory),
            Err((mapping, error)) => {
                self.mapping = Some(mapping);
                return Err(error);
            }
        }
        Ok(result)
    }

    pub fn reply(mut self, result: i64) {
        // End local access before any reply can release the caller's loan.
        drop(self.mapping.take());
        let memory = self.request.memory.take();
        if let Some(reply) = self.request.reply.take() {
            if result > 0 {
                let _ = reply.reply_move(memory.expect("filled receive buffer"), result);
            } else {
                drop(memory);
                let _ = reply.reply(result);
            }
        }
    }
}

impl Drop for PendingReceive {
    fn drop(&mut self) {
        // Mapping destruction precedes reply/borrow cancellation even when
        // fill/unmap failed or a pending socket is reclaimed.
        drop(self.mapping.take());
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum SocketKind {
    Tcp,
    Udp,
}

#[derive(Debug, Eq, PartialEq)]
pub enum CloseTarget {
    Live,
    Unavailable,
}

/// Unpublished selector reservation. Dropping it burns the ID; it owns no
/// backend socket yet. Only publication may consume its private serial.
#[must_use]
pub struct ReservedSocketId(u64);

pub struct TcpipState {
    pub sockets: BTreeMap<u64, SocketEntry>,
    pub next_sock_id: u64,
    pub random: NetworkRandom,
}

impl TcpipState {
    /// IDs are never reused within this endpoint lifetime, including failed
    /// publication. Exhaustion rejects before preparing socket backing.
    pub fn alloc_sock_id(&mut self) -> Option<ReservedSocketId> {
        let id = self.next_sock_id;
        if id == 0 || id > i64::MAX as u64 {
            return None;
        }
        self.next_sock_id = id + 1;
        Some(ReservedSocketId(id))
    }

    /// Unavailability is only a logical selector receipt. It attests to no
    /// former owner, backend cleanup, physical release or refunded charge.
    pub fn close_target(&self, id: u64, owner: SocketOwner) -> Result<CloseTarget, i64> {
        if id == 0 || id >= self.next_sock_id {
            return Err(crate::socket::ERR_BAD_SOCKET);
        }
        match self.sockets.get(&id) {
            Some(entry) if entry.owner == owner => Ok(CloseTarget::Live),
            Some(_) => Err(crate::socket::ERR_BAD_SOCKET),
            None => Ok(CloseTarget::Unavailable),
        }
    }

    /// Allocate a local ephemeral port for an outgoing connection. The
    /// default ephemeral range is 49152..=65535.
    pub fn alloc_ephemeral_port(&mut self) -> u16 {
        self.random.ephemeral_port()
    }

    pub fn owner_socket_count(&self, owner: SocketOwner) -> usize {
        self.sockets.values().filter(|entry| entry.owner == owner).count()
    }

    pub fn owner_buffer_bytes(&self, owner: SocketOwner) -> usize {
        self.sockets
            .values()
            .filter(|entry| entry.owner == owner)
            .map(|entry| entry.buffer_bytes)
            .sum()
    }

    pub fn owned_entry_mut(
        &mut self,
        socket_id: u64,
        owner: SocketOwner,
    ) -> Option<&mut SocketEntry> {
        let entry = self.sockets.get_mut(&socket_id)?;
        (entry.owner == owner).then_some(entry)
    }

    pub fn owned_entry(&self, socket_id: u64, owner: SocketOwner) -> Option<&SocketEntry> {
        let entry = self.sockets.get(&socket_id)?;
        (entry.owner == owner).then_some(entry)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn accept_waits_for_handshake_and_receive_drains_before_eof() {
        use smoltcp::socket::tcp::State;
        for state in [State::Listen, State::SynReceived] {
            assert_eq!(super::accept_result(state), None);
            assert!(!super::receive_eof(state, false));
        }
        for state in [State::Established, State::CloseWait] {
            assert_eq!(super::accept_result(state), Some(0));
        }
        for state in [State::Closed, State::Closing, State::LastAck, State::TimeWait] {
            assert_eq!(super::accept_result(state), Some(crate::socket::ERR_CONNECTION_REFUSED));
        }
        for state in
            [State::Closed, State::CloseWait, State::Closing, State::LastAck, State::TimeWait]
        {
            assert!(super::receive_eof(state, false));
            assert!(!super::receive_eof(state, true));
        }
        for state in [State::SynSent, State::Established, State::FinWait1, State::FinWait2] {
            assert!(!super::receive_eof(state, false));
        }
    }

    use alloc::{
        vec,
        vec::Vec,
    };

    use smoltcp::{
        iface::SocketStorage,
        socket::{
            tcp::SocketBuffer,
            udp::{
                PacketBuffer,
                PacketMetadata,
            },
        },
    };

    use super::*;

    fn state() -> TcpipState {
        TcpipState {
            sockets: BTreeMap::new(),
            next_sock_id: 1,
            random: NetworkRandom::initialize(|bytes| {
                bytes.fill(3);
                Ok::<_, ()>(())
            })
            .unwrap(),
        }
    }

    fn entry(
        set: &mut SocketSet<'_>,
        owner: SocketOwner,
        kind: SocketKind,
        activated: bool,
    ) -> SocketEntry {
        let handle = match kind {
            SocketKind::Tcp => {
                let mut socket = TcpSocket::new(
                    SocketBuffer::new(vec![0; 128]),
                    SocketBuffer::new(vec![0; 128]),
                );
                if activated {
                    socket.listen(8080).unwrap();
                }
                set.add(socket)
            }
            SocketKind::Udp => {
                let mut socket = UdpSocket::new(
                    PacketBuffer::new(vec![PacketMetadata::EMPTY; 1], vec![0; 128]),
                    PacketBuffer::new(vec![PacketMetadata::EMPTY; 1], vec![0; 128]),
                );
                if activated {
                    socket.bind(8081).unwrap();
                }
                set.add(socket)
            }
        };
        SocketEntry {
            handle,
            kind,
            owner,
            buffer_bytes: 256,
            lifetime: SocketLifetime::new(false, 0),
            activated,
            udp_remote: None,
            recv_pending: None,
            accept_pending: None,
            send_pending: None,
            closing: false,
            close_started_ms: None,
        }
    }

    #[test]
    fn deferred_send_waits_for_handshake_and_capacity_but_rejects_terminal_states() {
        use smoltcp::socket::tcp::State;
        for state in [State::SynSent, State::SynReceived] {
            assert_eq!(send_ready(state, true), Some(false));
        }
        for state in [State::Established, State::CloseWait] {
            assert_eq!(send_ready(state, false), Some(false));
            assert_eq!(send_ready(state, true), Some(true));
        }
        for state in [
            State::Closed,
            State::Listen,
            State::FinWait1,
            State::FinWait2,
            State::Closing,
            State::LastAck,
            State::TimeWait,
        ] {
            assert_eq!(send_ready(state, true), None);
        }
    }

    #[test]
    fn socket_ids_never_wrap_or_reuse_after_failed_publication() {
        let mut state = state();
        assert_eq!(state.alloc_sock_id().map(|id| id.0), Some(1));
        assert_eq!(state.alloc_sock_id().map(|id| id.0), Some(2));
        state.next_sock_id = i64::MAX as u64;
        assert_eq!(state.alloc_sock_id().map(|id| id.0), Some(i64::MAX as u64));
        assert_eq!(state.alloc_sock_id().map(|id| id.0), None);
        assert_eq!(state.alloc_sock_id().map(|id| id.0), None);
        assert_eq!(state.next_sock_id, i64::MAX as u64 + 1);
    }

    #[test]
    fn full_width_selectors_distinguish_live_sockets_with_equal_low_halves() {
        let mut state = state();
        let owner = SocketOwner {
            address_space: 5,
            generation: 7,
            principal: 9,
        };
        let foreign = SocketOwner {
            generation: 8,
            ..owner
        };
        let mut storage: Vec<SocketStorage<'_>> =
            core::iter::repeat_with(Default::default).take(2).collect();
        let mut set = SocketSet::new(&mut storage[..]);
        let low = state.alloc_sock_id().unwrap();
        let low_id = low.0;
        let first = entry(&mut set, owner, SocketKind::Tcp, true);
        state.publish(&mut set, low, first, |_| Ok::<_, ()>(())).unwrap();
        state.next_sock_id = (1 << 32) | low_id;
        let high = state.alloc_sock_id().unwrap();
        let high_id = high.0;
        let second = entry(&mut set, owner, SocketKind::Udp, true);
        state.publish(&mut set, high, second, |_| Ok::<_, ()>(())).unwrap();
        assert_ne!(low_id, high_id);
        assert_eq!(low_id as u32, high_id as u32);
        assert!(state.owned_entry(low_id, owner).unwrap().kind == SocketKind::Tcp);
        assert!(state.owned_entry(high_id, owner).unwrap().kind == SocketKind::Udp);
        assert!(state.owned_entry(high_id, foreign).is_none());
        assert!(state.owned_entry((2 << 32) | low_id, owner).is_none());
    }

    #[test]
    fn logical_close_distinguishes_live_foreign_unavailable_and_unissued_targets() {
        let mut state = state();
        let owner = SocketOwner {
            address_space: 1,
            generation: 1,
            principal: 7,
        };
        let foreign = SocketOwner {
            generation: 2,
            ..owner
        };
        let mut storage: Vec<SocketStorage<'_>> =
            core::iter::repeat_with(Default::default).take(1).collect();
        let mut set = SocketSet::new(&mut storage[..]);
        let reserved = state.alloc_sock_id().unwrap();
        let socket = entry(&mut set, owner, SocketKind::Udp, true);
        let diagnostic_id = reserved.0;
        state.publish(&mut set, reserved, socket, |_| Ok::<_, ()>(())).unwrap();
        assert_eq!(state.close_target(diagnostic_id, owner), Ok(CloseTarget::Live));
        assert_eq!(state.close_target(diagnostic_id, foreign), Err(crate::socket::ERR_BAD_SOCKET));
        assert_eq!(state.close_target(0, owner), Err(crate::socket::ERR_BAD_SOCKET));
        assert_eq!(
            state.close_target(state.next_sock_id, owner),
            Err(crate::socket::ERR_BAD_SOCKET)
        );
        assert_eq!(state.reap_abandoned(&mut set, 0, |_| false), 1);
        assert_eq!(state.close_target(diagnostic_id, owner), Ok(CloseTarget::Unavailable));
        assert_ne!(state.alloc_sock_id().map(|id| id.0), Some(diagnostic_id));
    }

    #[test]
    fn close_receipt_decoder_keeps_unavailability_distinct_from_close_started() {
        use crate::socket::{
            CLOSE_STARTED,
            CLOSE_UNAVAILABLE,
            CloseReceipt,
        };
        assert_eq!(CloseReceipt::decode(CLOSE_STARTED), Some(CloseReceipt::Started));
        assert_eq!(CloseReceipt::decode(CLOSE_UNAVAILABLE), Some(CloseReceipt::Unavailable));
        for result in [crate::socket::ERR_BAD_SOCKET, -99, 2, i64::MAX] {
            assert_eq!(CloseReceipt::decode(result), None);
        }
    }

    #[test]
    fn cancelled_creation_returns_fixed_socket_storage_and_owner_charges() {
        let owner = SocketOwner {
            address_space: 1,
            generation: 1,
            principal: 7,
        };
        let mut storage: Vec<SocketStorage<'_>> =
            core::iter::repeat_with(Default::default).take(1).collect();
        let mut set = SocketSet::new(&mut storage[..]);
        let mut state = state();
        for _ in 0..1024 {
            let reserved = state.alloc_sock_id().unwrap();
            let socket = entry(&mut set, owner, SocketKind::Tcp, false);
            assert_eq!(state.publish(&mut set, reserved, socket, |_| Err(())), Err(()));
            assert_eq!(set.iter().count(), 0);
            assert_eq!(state.owner_socket_count(owner), 0);
            assert_eq!(state.owner_buffer_bytes(owner), 0);
        }
        let reserved = state.alloc_sock_id().unwrap();
        let socket = entry(&mut set, owner, SocketKind::Udp, true);
        assert!(state.publish(&mut set, reserved, socket, |_| Ok::<_, ()>(())).is_ok());
        assert_eq!(set.iter().count(), 1);
    }

    #[test]
    fn dead_generation_releases_fresh_tcp_listener_and_open_udp_together() {
        let old = SocketOwner {
            address_space: 1,
            generation: 1,
            principal: 7,
        };
        let successor = SocketOwner {
            generation: 2,
            ..old
        };
        let mut storage: Vec<SocketStorage<'_>> =
            core::iter::repeat_with(Default::default).take(4).collect();
        let mut set = SocketSet::new(&mut storage[..]);
        let mut state = state();
        for (kind, activated) in
            [(SocketKind::Tcp, false), (SocketKind::Tcp, true), (SocketKind::Udp, true)]
        {
            let reserved = state.alloc_sock_id().unwrap();
            let socket = entry(&mut set, old, kind, activated);
            state.publish(&mut set, reserved, socket, |_| Ok::<_, ()>(())).unwrap();
        }
        let reserved = state.alloc_sock_id().unwrap();
        let socket = entry(&mut set, successor, SocketKind::Udp, true);
        let id = state.publish(&mut set, reserved, socket, |_| Ok::<_, ()>(())).unwrap();
        assert!(state.owned_entry(id, old).is_none());
        assert_eq!(state.reap_abandoned(&mut set, 1, |owner| owner == successor), 3);
        assert_eq!(state.owner_buffer_bytes(old), 0);
        assert_eq!(state.owner_socket_count(successor), 1);
        assert_eq!(set.iter().count(), 1);
        assert_eq!(state.reap_abandoned(&mut set, 2, |_| false), 1);
        assert_eq!(set.iter().count(), 0);
    }

    #[test]
    fn unobserved_live_owner_creation_expires_but_active_socket_survives() {
        let owner = SocketOwner {
            address_space: 1,
            generation: 1,
            principal: 7,
        };
        let mut storage: Vec<SocketStorage<'_>> =
            core::iter::repeat_with(Default::default).take(2).collect();
        let mut set = SocketSet::new(&mut storage[..]);
        let mut state = state();
        for activated in [false, true] {
            let reserved = state.alloc_sock_id().unwrap();
            let socket = entry(&mut set, owner, SocketKind::Tcp, activated);
            state.publish(&mut set, reserved, socket, |_| Ok::<_, ()>(())).unwrap();
        }
        assert_eq!(state.reap_abandoned(&mut set, 4999, |_| true), 0);
        assert_eq!(state.reap_abandoned(&mut set, 5000, |_| true), 1);
        assert_eq!(state.owner_buffer_bytes(owner), 256);
        assert_eq!(set.iter().count(), 1);
    }
}

impl TcpipState {
    /// Install and deliver a new remote ID as one publication operation.
    /// Rejected/cancelled delivery immediately returns smoltcp storage/buffers.
    pub fn publish<E>(
        &mut self,
        sockets: &mut SocketSet<'_>,
        id: ReservedSocketId,
        entry: SocketEntry,
        deliver: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<u64, E> {
        let id = id.0;
        assert!(id > 0 && id < self.next_sock_id && !self.sockets.contains_key(&id));
        self.sockets.insert(id, entry);
        if let Err(error) = deliver(id) {
            let entry = self.sockets.remove(&id).expect("just installed socket");
            sockets.remove(entry.handle);
            return Err(error);
        }
        Ok(id)
    }

    /// Exact-owner liveness comes from the designated kernel adapter. No name
    /// lookup or successor's numeric ASID can keep an earlier owner alive.
    pub fn reap_abandoned(
        &mut self,
        sockets: &mut SocketSet<'_>,
        now_ms: u64,
        mut live: impl FnMut(SocketOwner) -> bool,
    ) -> usize {
        let before = self.sockets.len();
        self.sockets.retain(|_, entry| {
            if !entry.lifetime.reclaim(live(entry.owner), entry.activated, now_ms) {
                return true;
            }
            match entry.kind {
                SocketKind::Tcp => sockets.get_mut::<TcpSocket>(entry.handle).abort(),
                SocketKind::Udp => sockets.get_mut::<UdpSocket>(entry.handle).close(),
            }
            sockets.remove(entry.handle);
            false
        });
        before - self.sockets.len()
    }
}
