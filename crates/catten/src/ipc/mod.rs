//! Experimental endpoint IPC.
//!
//! This is the first Xous-inspired cross-protection-domain IPC substrate. It is
//! deliberately scalar-only: endpoints, connections, pending calls, and reply
//! tokens are separate from completion capabilities and from the LP-indexed
//! mailbox smoke ABI.

use alloc::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    vec::Vec,
};
use core::ops::BitOr;

use spin::{
    LazyLock,
    RwLock,
};

use crate::memory::AddressSpaceId;

pub type CapabilityId = u64;
type EndpointId = u64;
type ReplyTokenId = u64;
type PendingCallId = u64;

pub const REPLY_CANCELLED: i64 = -3;
pub const REPLY_ENDPOINT_CLOSED: i64 = -7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    UnknownCapability,
    WrongType,
    PermissionDenied,
    QueueFull,
    EndpointClosed,
    NoMessage,
    ReplyAlreadyUsed,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionRights(u32);

impl ConnectionRights {
    pub const ALL: Self =
        Self(Self::SEND.0 | Self::CALL.0 | Self::RECEIVE.0 | Self::MINT_CONNECTION.0);
    pub const CALL: Self = Self(1 << 1);
    pub const MINT_CONNECTION: Self = Self(1 << 3);
    pub const RECEIVE: Self = Self(1 << 2);
    pub const SEND: Self = Self(1 << 0);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    fn intersection(self, allowed: Self) -> Self {
        Self(self.0 & allowed.0)
    }
}

impl BitOr for ConnectionRights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarMessage {
    pub sender: AddressSpaceId,
    pub interface: u64,
    pub version: u32,
    pub opcode: u32,
    pub arg0: u64,
    pub reply: Option<CapabilityId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capability {
    Endpoint {
        endpoint: EndpointId,
        rights: ConnectionRights,
    },
    Connection {
        endpoint: EndpointId,
        rights: ConnectionRights,
    },
    ReplyToken {
        token: ReplyTokenId,
    },
    PendingCall {
        call: PendingCallId,
    },
}

#[derive(Debug)]
struct AsIpcCaps {
    next: CapabilityId,
    caps: BTreeMap<CapabilityId, Capability>,
}

impl AsIpcCaps {
    fn new() -> Self {
        Self {
            next: 1,
            caps: BTreeMap::new(),
        }
    }

    fn insert(&mut self, cap: Capability) -> CapabilityId {
        let id = self.next;
        self.next = self.next.checked_add(1).expect("IPC capability id overflow");
        self.caps.insert(id, cap);
        id
    }
}

#[derive(Debug, Clone)]
struct QueuedMessage {
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<CapabilityId>,
}

#[derive(Debug)]
struct Endpoint {
    owner: AddressSpaceId,
    interface: u64,
    version: u32,
    capacity: usize,
    queue: VecDeque<QueuedMessage>,
    closed: bool,
}

#[derive(Debug)]
struct ReplyToken {
    server: AddressSpaceId,
    call: PendingCallId,
    consumed: bool,
}

#[derive(Debug)]
struct PendingCall {
    caller: AddressSpaceId,
    result: Option<i64>,
}

#[derive(Debug)]
struct IpcRegistry {
    next_endpoint: EndpointId,
    next_reply: ReplyTokenId,
    next_call: PendingCallId,
    endpoints: BTreeMap<EndpointId, Endpoint>,
    reply_tokens: BTreeMap<ReplyTokenId, ReplyToken>,
    pending_calls: BTreeMap<PendingCallId, PendingCall>,
    caps: BTreeMap<AddressSpaceId, AsIpcCaps>,
}

impl IpcRegistry {
    fn new() -> Self {
        Self {
            next_endpoint: 1,
            next_reply: 1,
            next_call: 1,
            endpoints: BTreeMap::new(),
            reply_tokens: BTreeMap::new(),
            pending_calls: BTreeMap::new(),
            caps: BTreeMap::new(),
        }
    }

    fn alloc_endpoint(&mut self) -> EndpointId {
        let id = self.next_endpoint;
        self.next_endpoint = self.next_endpoint.checked_add(1).expect("endpoint id overflow");
        id
    }

    fn alloc_reply(&mut self) -> ReplyTokenId {
        let id = self.next_reply;
        self.next_reply = self.next_reply.checked_add(1).expect("reply token id overflow");
        id
    }

    fn alloc_call(&mut self) -> PendingCallId {
        let id = self.next_call;
        self.next_call = self.next_call.checked_add(1).expect("pending call id overflow");
        id
    }

    fn as_caps(&mut self, asid: AddressSpaceId) -> &mut AsIpcCaps {
        self.caps.entry(asid).or_insert_with(AsIpcCaps::new)
    }

    fn cap(&self, asid: AddressSpaceId, cap: CapabilityId) -> Result<Capability, IpcError> {
        self.caps
            .get(&asid)
            .and_then(|caps| caps.caps.get(&cap))
            .copied()
            .ok_or(IpcError::UnknownCapability)
    }

    fn remove_cap(
        &mut self,
        asid: AddressSpaceId,
        cap: CapabilityId,
    ) -> Result<Capability, IpcError> {
        self.caps
            .get_mut(&asid)
            .and_then(|caps| caps.caps.remove(&cap))
            .ok_or(IpcError::UnknownCapability)
    }

    fn remove_matching_caps(&mut self, asid: AddressSpaceId, target: Capability) {
        if let Some(caps) = self.caps.get_mut(&asid) {
            caps.caps.retain(|_, cap| *cap != target);
        }
    }
}

static IPC: LazyLock<RwLock<IpcRegistry>> = LazyLock::new(|| RwLock::new(IpcRegistry::new()));

pub fn endpoint_create(
    owner: AddressSpaceId,
    interface: u64,
    version: u32,
    capacity: usize,
) -> Result<CapabilityId, IpcError> {
    if capacity == 0 {
        return Err(IpcError::QueueFull);
    }

    let mut ipc = IPC.write();
    let endpoint = ipc.alloc_endpoint();
    ipc.endpoints.insert(
        endpoint,
        Endpoint {
            owner,
            interface,
            version,
            capacity,
            queue: VecDeque::new(),
            closed: false,
        },
    );
    Ok(ipc.as_caps(owner).insert(Capability::Endpoint {
        endpoint,
        rights: ConnectionRights::ALL,
    }))
}

pub fn connection_mint(
    owner: AddressSpaceId,
    endpoint_cap: CapabilityId,
    rights: ConnectionRights,
) -> Result<CapabilityId, IpcError> {
    connection_delegate(owner, endpoint_cap, owner, rights)
}

pub fn connection_delegate(
    owner: AddressSpaceId,
    endpoint_cap: CapabilityId,
    target: AddressSpaceId,
    rights: ConnectionRights,
) -> Result<CapabilityId, IpcError> {
    let mut ipc = IPC.write();
    let (endpoint, endpoint_rights) = match ipc.cap(owner, endpoint_cap)? {
        Capability::Endpoint {
            endpoint,
            rights,
        } => (endpoint, rights),
        _ => return Err(IpcError::WrongType),
    };
    if !endpoint_rights.contains(ConnectionRights::MINT_CONNECTION) {
        return Err(IpcError::PermissionDenied);
    }
    let granted = rights.intersection(endpoint_rights);
    Ok(ipc.as_caps(target).insert(Capability::Connection {
        endpoint,
        rights: granted,
    }))
}

pub fn scalar_send(
    sender: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let (endpoint_id, rights) = match ipc.cap(sender, connection_cap)? {
        Capability::Connection {
            endpoint,
            rights,
        } => (endpoint, rights),
        _ => return Err(IpcError::WrongType),
    };
    if !rights.contains(ConnectionRights::SEND) {
        return Err(IpcError::PermissionDenied);
    }

    enqueue_scalar(&mut ipc, endpoint_id, sender, opcode, arg0, None)
}

pub fn scalar_call(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
) -> Result<CapabilityId, IpcError> {
    let mut ipc = IPC.write();
    let (endpoint_id, rights) = match ipc.cap(caller, connection_cap)? {
        Capability::Connection {
            endpoint,
            rights,
        } => (endpoint, rights),
        _ => return Err(IpcError::WrongType),
    };
    if !rights.contains(ConnectionRights::CALL) {
        return Err(IpcError::PermissionDenied);
    }

    let server = {
        let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
        if endpoint.closed {
            return Err(IpcError::EndpointClosed);
        }
        if endpoint.queue.len() >= endpoint.capacity {
            return Err(IpcError::QueueFull);
        }
        endpoint.owner
    };

    let call = ipc.alloc_call();
    ipc.pending_calls.insert(
        call,
        PendingCall {
            caller,
            result: None,
        },
    );
    let call_cap = ipc.as_caps(caller).insert(Capability::PendingCall {
        call,
    });

    let token = ipc.alloc_reply();
    ipc.reply_tokens.insert(
        token,
        ReplyToken {
            server,
            call,
            consumed: false,
        },
    );
    let token_cap = ipc.as_caps(server).insert(Capability::ReplyToken {
        token,
    });

    enqueue_scalar(&mut ipc, endpoint_id, caller, opcode, arg0, Some(token_cap))?;
    Ok(call_cap)
}

fn enqueue_scalar(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<CapabilityId>,
) -> Result<(), IpcError> {
    let endpoint = ipc.endpoints.get_mut(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    if endpoint.queue.len() >= endpoint.capacity {
        return Err(IpcError::QueueFull);
    }
    endpoint.queue.push_back(QueuedMessage {
        sender,
        opcode,
        arg0,
        reply,
    });
    Ok(())
}

pub fn receive(
    receiver: AddressSpaceId,
    endpoint_cap: CapabilityId,
) -> Result<ScalarMessage, IpcError> {
    let mut ipc = IPC.write();
    let (endpoint_id, rights) = match ipc.cap(receiver, endpoint_cap)? {
        Capability::Endpoint {
            endpoint,
            rights,
        } => (endpoint, rights),
        _ => return Err(IpcError::WrongType),
    };
    if !rights.contains(ConnectionRights::RECEIVE) {
        return Err(IpcError::PermissionDenied);
    }

    let endpoint = ipc.endpoints.get_mut(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    let message = endpoint.queue.pop_front().ok_or(IpcError::NoMessage)?;
    Ok(ScalarMessage {
        sender: message.sender,
        interface: endpoint.interface,
        version: endpoint.version,
        opcode: message.opcode,
        arg0: message.arg0,
        reply: message.reply,
    })
}

pub fn reply(server: AddressSpaceId, reply_cap: CapabilityId, result: i64) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let token_id = match ipc.cap(server, reply_cap)? {
        Capability::ReplyToken {
            token,
        } => token,
        _ => return Err(IpcError::WrongType),
    };
    let token = ipc.reply_tokens.get(&token_id).ok_or(IpcError::UnknownCapability)?;
    if token.server != server {
        return Err(IpcError::PermissionDenied);
    }
    if token.consumed {
        return Err(IpcError::ReplyAlreadyUsed);
    }
    let call_id = token.call;
    ipc.reply_tokens.remove(&token_id);
    let call = ipc.pending_calls.get_mut(&call_id).ok_or(IpcError::UnknownCapability)?;
    call.result = Some(result);
    let _ = ipc.remove_cap(server, reply_cap);
    Ok(())
}

pub fn poll_reply(caller: AddressSpaceId, call_cap: CapabilityId) -> Result<Option<i64>, IpcError> {
    let ipc = IPC.read();
    let call_id = match ipc.cap(caller, call_cap)? {
        Capability::PendingCall {
            call,
        } => call,
        _ => return Err(IpcError::WrongType),
    };
    let call = ipc.pending_calls.get(&call_id).ok_or(IpcError::UnknownCapability)?;
    if call.caller != caller {
        return Err(IpcError::PermissionDenied);
    }
    Ok(call.result)
}

pub fn close_cap(asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    match ipc.remove_cap(asid, cap)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => {
            let queued_replies = if let Some(endpoint) = ipc.endpoints.get_mut(&endpoint) {
                if endpoint.owner != asid {
                    Vec::new()
                } else {
                    endpoint.closed = true;
                    endpoint.queue.drain(..).filter_map(|message| message.reply).collect()
                }
            } else {
                Vec::new()
            };
            for reply_cap in queued_replies {
                consume_reply_cap(&mut ipc, asid, reply_cap, REPLY_ENDPOINT_CLOSED);
            }
        }
        Capability::PendingCall {
            call,
        } => {
            ipc.pending_calls.remove(&call);
            remove_reply_tokens_for_call(&mut ipc, call);
        }
        Capability::ReplyToken {
            token,
        } => {
            if let Some(token) = ipc.reply_tokens.remove(&token) {
                if let Some(call) = ipc.pending_calls.get_mut(&token.call) {
                    call.result = Some(REPLY_CANCELLED);
                }
            }
        }
        Capability::Connection {
            ..
        } => {}
    }
    Ok(())
}

pub fn close_address_space(asid: AddressSpaceId) {
    let caps = {
        let ipc = IPC.read();
        ipc.caps
            .get(&asid)
            .map(|caps| caps.caps.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for cap in caps {
        let _ = close_cap(asid, cap);
    }
    IPC.write().caps.remove(&asid);
}

fn consume_reply_cap(
    ipc: &mut IpcRegistry,
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    result: i64,
) {
    let token = match ipc.remove_cap(server, reply_cap) {
        Ok(Capability::ReplyToken {
            token,
        }) => token,
        _ => return,
    };
    if let Some(token) = ipc.reply_tokens.remove(&token) {
        if let Some(call) = ipc.pending_calls.get_mut(&token.call) {
            call.result = Some(result);
        }
    }
}

fn remove_reply_tokens_for_call(ipc: &mut IpcRegistry, call: PendingCallId) {
    let tokens = ipc
        .reply_tokens
        .iter()
        .filter_map(|(token_id, token)| {
            if token.call == call {
                Some((*token_id, token.server))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for (token, server) in tokens {
        ipc.reply_tokens.remove(&token);
        ipc.remove_matching_caps(
            server,
            Capability::ReplyToken {
                token,
            },
        );
    }
}
