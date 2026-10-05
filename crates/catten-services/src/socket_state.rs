//! Socket records own deferred reply authority and remote-resource metadata.
use alloc::collections::BTreeMap;

use catten_rt::owned::ReplyToken;
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
    pub recv_pending: Option<ReplyToken>,
    /// Set by `OP_CLOSE`: the socket was gracefully closed and may be swept
    /// from the set once it reaches a final state.
    pub closing: bool,
    /// Reactor time at which graceful close was requested.  This is used to
    /// bound how long a peer that never completes the FIN handshake can hold
    /// the owner's socket quota.
    pub close_started_ms: Option<u64>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum SocketKind {
    Tcp,
    Udp,
}

pub struct TcpipState {
    pub sockets: BTreeMap<u64, SocketEntry>,
    pub next_sock_id: u64,
    pub random: NetworkRandom,
}

impl TcpipState {
    pub fn alloc_sock_id(&mut self) -> u64 {
        loop {
            let id = self.next_sock_id;
            self.next_sock_id = if id >= i64::MAX as u64 {
                1
            } else {
                id + 1
            };
            if !self.sockets.contains_key(&id) {
                return id;
            }
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
            closing: false,
            close_started_ms: None,
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
            let socket = entry(&mut set, owner, SocketKind::Tcp, false);
            assert_eq!(state.publish(&mut set, socket, |_| Err(())), Err(()));
            assert_eq!(set.iter().count(), 0);
            assert_eq!(state.owner_socket_count(owner), 0);
            assert_eq!(state.owner_buffer_bytes(owner), 0);
        }
        let socket = entry(&mut set, owner, SocketKind::Udp, true);
        assert!(state.publish(&mut set, socket, |_| Ok::<_, ()>(())).is_ok());
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
            let socket = entry(&mut set, old, kind, activated);
            state.publish(&mut set, socket, |_| Ok::<_, ()>(())).unwrap();
        }
        let socket = entry(&mut set, successor, SocketKind::Udp, true);
        let id = state.publish(&mut set, socket, |_| Ok::<_, ()>(())).unwrap();
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
            let socket = entry(&mut set, owner, SocketKind::Tcp, activated);
            state.publish(&mut set, socket, |_| Ok::<_, ()>(())).unwrap();
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
        entry: SocketEntry,
        deliver: impl FnOnce(u64) -> Result<(), E>,
    ) -> Result<u64, E> {
        let id = self.alloc_sock_id();
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
