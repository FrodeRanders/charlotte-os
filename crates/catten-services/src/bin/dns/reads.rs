//! Read transactions retain their fresh Raft fence, destination and original
//! deadline while the reactor continues to process quorum traffic.
use alloc::{
    string::String,
    vec::Vec,
};

use catten_graft::{
    node::{
        PendingRead,
        RaftNode,
    },
    types::NodeState,
};
use catten_rt::owned::{
    ConnectionRef,
    IncomingMessage,
    OwnedMemory,
    ReplyToken,
};
use catten_services::{
    deadline::Deadline,
    dns,
    name_catalog::{
        CatalogEntry,
        decode_query_result,
        encode_lookup_query,
    },
    relmsg_transport::RelmsgRaftTransport,
};

use super::{
    MAX_IN_FLIGHT_CALLS,
    REMOTE_CALL_TIMEOUT_MS,
    local_calls::begin_local_call,
    message_memory::{
        packed_name,
        read_call_request,
    },
    state::{
        InFlightCall,
        LocalCallDestination,
        PendingLocalCall,
        PendingQuery,
        PendingQueryKind,
    },
};

enum Destination {
    Lookup(Option<ReplyToken>),
    Call {
        reply: Option<ReplyToken>,
        opcode: u32,
        arg: i64,
    },
    Remote {
        peer: String,
        session: u64,
        query_id: u64,
    },
}

struct Operation {
    read: PendingRead,
    name: Vec<u8>,
    deadline: Deadline,
    destination: Destination,
}

#[derive(Default)]
pub(super) struct Reads {
    pending: Vec<Operation>,
}

pub(super) struct Routes<'a> {
    pub names: ConnectionRef<'a>,
    pub node_name: &'a [u8],
    pub transport: &'a RelmsgRaftTransport,
    pub local: &'a mut Vec<PendingLocalCall>,
    pub remote: &'a mut Vec<InFlightCall>,
    pub next_call_id: &'a mut u64,
    pub session: u64,
}

fn error(code: i64) -> i64 {
    if code == catten_graft::types::ERR_RESOURCE_LIMIT {
        dns::ERR_RESOURCE_LIMIT
    } else {
        dns::ERR_NOT_LEADER
    }
}

fn reply_remote(
    transport: &RelmsgRaftTransport,
    peer: &str,
    session: u64,
    query_id: u64,
    status: i64,
    entry: Option<&CatalogEntry>,
) {
    match catten_services::rquery::encode_reply(
        session,
        query_id,
        status,
        entry.map_or(0, |entry| entry.generation),
        entry.map_or(&[][..], |entry| entry.node.as_slice()),
    ) {
        Ok(reply) => transport.send_message(peer, catten_services::rquery::TAG_REPLY, reply),
        Err(error) => catten_rt::logln!("[dns] query reply preparation rejected: {:?}", error),
    }
}

impl Destination {
    fn reject(self, code: i64, transport: &RelmsgRaftTransport) {
        match self {
            Self::Lookup(reply)
            | Self::Call {
                reply,
                ..
            } => {
                if let Some(reply) = reply {
                    let _ = reply.reply(code);
                }
            }
            Self::Remote {
                peer,
                session,
                query_id,
            } => reply_remote(transport, &peer, session, query_id, code, None),
        }
    }
}

impl Reads {
    pub fn remaining_ms(&self) -> impl Iterator<Item = u64> + '_ {
        self.pending.iter().map(|op| op.deadline.remaining_ms())
    }

    fn admit(
        &mut self,
        node: &mut RaftNode,
        name: Vec<u8>,
        deadline: Deadline,
        destination: Destination,
    ) -> Result<(), (i64, Destination)> {
        if deadline.expired() {
            return Err((dns::ERR_NOT_LEADER, destination));
        }
        if self.pending.len() >= MAX_IN_FLIGHT_CALLS {
            return Err((dns::ERR_BUSY, destination));
        }
        if self.pending.try_reserve(1).is_err() {
            return Err((dns::ERR_RESOURCE_LIMIT, destination));
        }
        // Prepare query bytes and queue storage before publishing the no-op.
        let query = encode_lookup_query(&name);
        if deadline.expired() {
            return Err((dns::ERR_NOT_LEADER, destination));
        }
        let fence = match node.begin_read(query, node.millis()) {
            Ok(read) => read,
            Err(code) => return Err((error(code), destination)),
        };
        self.pending.push(Operation {
            read: fence,
            name,
            deadline,
            destination,
        });
        Ok(())
    }

    pub fn remote_query(
        &mut self,
        node: &mut RaftNode,
        transport: &RelmsgRaftTransport,
        peer: String,
        session: u64,
        query_id: u64,
        name: Vec<u8>,
    ) {
        let deadline = Deadline::after(REMOTE_CALL_TIMEOUT_MS);
        if let Err((code, destination)) = self.admit(
            node,
            name,
            deadline,
            Destination::Remote {
                peer,
                session,
                query_id,
            },
        ) {
            destination.reject(code, transport);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn client(
        &mut self,
        node: &mut RaftNode,
        message: catten_syscall::IpcMessage,
        transport: &RelmsgRaftTransport,
        node_name: &[u8],
        queries: &mut Vec<PendingQuery>,
        next_query_id: &mut u64,
        session: u64,
    ) {
        let deadline = Deadline::after(REMOTE_CALL_TIMEOUT_MS);
        let (name, destination) = if message.opcode == dns::OP_CALL {
            let Some(client) = read_call_request(message) else {
                return;
            };
            (
                client.name,
                Destination::Call {
                    reply: client.reply,
                    opcode: client.opcode,
                    arg: client.arg,
                },
            )
        } else {
            // Raw receive ABI boundary: adopt every unique attachment once,
            // before the main loop's legacy scalar fallback guards.
            let Ok(Some(mut request)) = (unsafe { IncomingMessage::from_raw(message) }) else {
                return;
            };
            (packed_name(request.arg0), Destination::Lookup(request.reply.take()))
        };
        if name.is_empty() {
            destination.reject(dns::ERR_TOO_LARGE, transport);
            return;
        }
        if node.state == NodeState::Leader {
            if let Err((code, destination)) = self.admit(node, name, deadline, destination) {
                destination.reject(code, transport);
            }
            return;
        }
        let Some(leader) = node.known_leader_id.clone() else {
            destination.reject(dns::ERR_NOT_LEADER, transport);
            return;
        };
        if deadline.expired()
            || queries.len() >= MAX_IN_FLIGHT_CALLS
            || !transport.has_peer(&leader)
        {
            destination.reject(dns::ERR_NOT_LEADER, transport);
            return;
        }
        if queries.try_reserve(1).is_err() {
            destination.reject(dns::ERR_RESOURCE_LIMIT, transport);
            return;
        }
        let query_id = *next_query_id;
        let Some(next_id) = query_id.checked_add(1) else {
            destination.reject(dns::ERR_RESOURCE_LIMIT, transport);
            return;
        };
        *next_query_id = next_id;
        let request =
            match catten_services::rquery::encode_request(session, query_id, node_name, &name) {
                Ok(request) if !deadline.expired() => request,
                Ok(_) => {
                    destination.reject(dns::ERR_NOT_LEADER, transport);
                    return;
                }
                Err(error) => {
                    destination.reject(error.status(), transport);
                    return;
                }
            };
        let kind = match destination {
            Destination::Lookup(reply) => PendingQueryKind::Lookup {
                reply,
            },
            Destination::Call {
                reply,
                opcode,
                arg,
            } => PendingQueryKind::Call {
                reply,
                name,
                opcode,
                arg,
            },
            Destination::Remote {
                ..
            } => unreachable!(),
        };
        queries.push(PendingQuery {
            query_id,
            expected_leader: leader.clone(),
            deadline,
            kind,
        });
        transport.send_message(&leader, catten_services::rquery::TAG_REQUEST, request);
    }

    pub fn drive(&mut self, node: &RaftNode, routes: &mut Routes<'_>) {
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index].deadline.expired() {
                self.pending
                    .swap_remove(index)
                    .destination
                    .reject(dns::ERR_NOT_LEADER, routes.transport);
                continue;
            }
            match node.read_ready(&self.pending[index].read) {
                Ok(false) => {
                    index += 1;
                    continue;
                }
                Err(code) => {
                    self.pending
                        .swap_remove(index)
                        .destination
                        .reject(error(code), routes.transport);
                    continue;
                }
                Ok(true) => {}
            }
            let op = self.pending.swap_remove(index);
            match node.handle_client_query(op.read) {
                Ok(bytes) => resolve(
                    op.name,
                    op.deadline,
                    op.destination,
                    decode_query_result(&bytes),
                    routes,
                ),
                Err((code, _)) => op.destination.reject(error(code), routes.transport),
            }
        }
    }
}

pub(super) fn resolved_lookup(
    reply: Option<ReplyToken>,
    entry: Option<CatalogEntry>,
    deadline: Deadline,
    routes: &mut Routes<'_>,
) {
    resolve(Vec::new(), deadline, Destination::Lookup(reply), entry, routes);
}

fn resolve(
    name: Vec<u8>,
    deadline: Deadline,
    destination: Destination,
    entry: Option<CatalogEntry>,
    routes: &mut Routes<'_>,
) {
    if deadline.expired() {
        destination.reject(dns::ERR_NOT_LEADER, routes.transport);
        return;
    }
    match destination {
        Destination::Remote {
            peer,
            session,
            query_id,
        } => reply_remote(routes.transport, &peer, session, query_id, 0, entry.as_ref()),
        Destination::Call {
            reply,
            opcode,
            arg,
        } => routes.call(name, entry, reply, opcode, arg, deadline),
        Destination::Lookup(reply) => {
            let Some(reply) = reply else {
                return;
            };
            let Some(entry) = entry else {
                let _ = reply.reply(dns::ERR_NOT_FOUND);
                return;
            };
            if entry.node == routes.node_name {
                // Placement metadata supplies no capability-grant authority.
                // Ordinary NS lookup returns a non-delegable SEND/CALL owner;
                // invocation resolves and uses it directly instead of minting.
                let _ = reply.reply(dns::RESULT_LOCAL);
            } else {
                reply_bytes(reply, &entry.node);
            }
        }
    }
}

fn reply_bytes(reply: ReplyToken, bytes: &[u8]) {
    let prepared = (|| {
        let memory = OwnedMemory::allocate(bytes.len().div_ceil(4096))
            .map_err(|_| dns::ERR_RESOURCE_LIMIT)?;
        let mut mapping = memory.map_writable().map_err(|_| dns::ERR_RESOURCE_LIMIT)?;
        mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        mapping.unmap().map_err(|_| dns::ERR_RESOURCE_LIMIT)
    })();
    match prepared {
        Ok(memory) => {
            let _ = reply.reply_move(memory, dns::RESULT_REMOTE);
        }
        Err(code) => {
            let _ = reply.reply(code);
        }
    }
}

impl Routes<'_> {
    pub fn call(
        &mut self,
        name: Vec<u8>,
        entry: Option<CatalogEntry>,
        mut reply: Option<ReplyToken>,
        opcode: u32,
        arg: i64,
        deadline: Deadline,
    ) {
        let code = (|| {
            if deadline.expired() {
                return Some(dns::ERR_NOT_LEADER);
            }
            let Some(entry) = entry else {
                return Some(dns::ERR_NOT_FOUND);
            };
            if entry.node == self.node_name {
                if self.local.len() >= MAX_IN_FLIGHT_CALLS {
                    return Some(dns::ERR_BUSY);
                }
                if self.local.try_reserve(1).is_err() {
                    return Some(dns::ERR_RESOURCE_LIMIT);
                }
                match begin_local_call(
                    self.names,
                    &name,
                    opcode,
                    arg,
                    deadline,
                    LocalCallDestination::Client {
                        reply: reply.take(),
                    },
                ) {
                    Ok(call) => {
                        self.local.push(call);
                        None
                    }
                    Err((
                        code,
                        LocalCallDestination::Client {
                            reply: retained,
                        },
                    )) => {
                        reply = retained;
                        Some(code)
                    }
                    Err(_) => unreachable!(),
                }
            } else {
                let Ok(peer) = core::str::from_utf8(&entry.node) else {
                    return Some(dns::ERR_NOT_FOUND);
                };
                if !self.transport.has_peer(peer) {
                    return Some(dns::ERR_NOT_FOUND);
                }
                if self.remote.len() >= MAX_IN_FLIGHT_CALLS {
                    return Some(dns::ERR_BUSY);
                }
                if self.remote.try_reserve(1).is_err() {
                    return Some(dns::ERR_RESOURCE_LIMIT);
                }
                let call_id = *self.next_call_id;
                let Some(next_id) = call_id.checked_add(1) else {
                    return Some(dns::ERR_RESOURCE_LIMIT);
                };
                *self.next_call_id = next_id;
                match catten_services::rcall::encode_request(
                    self.session,
                    call_id,
                    self.node_name,
                    &name,
                    entry.generation,
                    opcode,
                    arg,
                ) {
                    Ok(_) if deadline.expired() => Some(dns::ERR_NOT_LEADER),
                    Ok(request) => {
                        self.remote.push(InFlightCall {
                            call_id,
                            expected_peer: peer.into(),
                            expected_generation: entry.generation,
                            reply: reply.take(),
                            deadline,
                        });
                        self.transport.send_message(
                            peer,
                            catten_services::rcall::TAG_REQUEST,
                            request,
                        );
                        None
                    }
                    Err(error) => Some(error.status()),
                }
            }
        })();
        if let Some(code) = code
            && let Some(reply) = reply
        {
            let _ = reply.reply(code);
        }
    }
}
