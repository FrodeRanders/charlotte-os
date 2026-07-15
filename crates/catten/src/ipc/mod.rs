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
    sync::Weak,
    vec::Vec,
};
use core::ops::BitOr;

use concurrent_queue::ConcurrentQueue;
use spin::{
    LazyLock,
    RwLock,
};

use crate::{
    klib::observer::{
        Observable,
        Observer,
    },
    memory::{
        AddressSpaceId,
        object::MemoryObjectCap,
    },
};

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
    MemoryTransferFailed,
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
    pub memory: Option<MemoryObjectCap>,
    pub connection: Option<CapabilityId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyValue {
    pub result: i64,
    pub cap: Option<CapabilityId>,
    pub memory: Option<MemoryObjectCap>,
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
    memory: Option<MemoryObjectCap>,
    connection: Option<CapabilityId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryBorrow {
    owner: AddressSpaceId,
    owner_cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
}

#[derive(Debug)]
struct Endpoint {
    owner: AddressSpaceId,
    interface: u64,
    version: u32,
    capacity: usize,
    queue: VecDeque<QueuedMessage>,
    observers: ConcurrentQueue<Weak<dyn Observer>>,
    closed: bool,
    /// When bound, endpoint readiness is delivered to this completion queue
    /// of the owner as a coalesced wake (architecture doc §16.3: readiness is
    /// a notification, not a completion). Posted on the empty→nonempty queue
    /// transition and on closure, so a shard can block on one CQ wait for
    /// both kernel completions and endpoint work (§7, Phase 7).
    notify_cq: Option<crate::completion::CqId>,
}

#[derive(Debug)]
struct ReplyToken {
    server: AddressSpaceId,
    call: PendingCallId,
    consumed: bool,
    borrow: Option<MemoryBorrow>,
}

#[derive(Debug)]
struct PendingCall {
    caller: AddressSpaceId,
    result: Option<ReplyValue>,
    /// Set once the caller has seen the result through `poll_reply`. From
    /// that point the returned connection/memory capabilities belong to the
    /// caller, and closing the pending-call cap no longer revokes them
    /// (state `ResultObserved` in the operation state machine).
    observed: bool,
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
            observers: ConcurrentQueue::unbounded(),
            closed: false,
            notify_cq: None,
        },
    );
    Ok(ipc.as_caps(owner).insert(Capability::Endpoint {
        endpoint,
        rights: ConnectionRights::ALL,
    }))
}

/// Binds an endpoint's readiness to one of the owner's completion queues.
///
/// After binding, the kernel posts a coalesced wake to that queue whenever
/// the endpoint's message queue transitions from empty to nonempty, and when
/// the endpoint closes. This lets a shard block on a single CQ wait for both
/// kernel/device completions and endpoint work (architecture doc §7,
/// Phase 7); readiness is a notification to inspect the endpoint, not a
/// completion record (§16.3).
pub fn endpoint_bind_cq(
    owner: AddressSpaceId,
    endpoint_cap: CapabilityId,
    cq: crate::completion::CqId,
) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let endpoint_id = match ipc.cap(owner, endpoint_cap)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => endpoint,
        _ => return Err(IpcError::WrongType),
    };
    let endpoint = ipc.endpoints.get_mut(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.owner != owner {
        return Err(IpcError::PermissionDenied);
    }
    endpoint.notify_cq = Some(cq);
    Ok(())
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
    let (endpoint, granted) = mintable_endpoint(&ipc, owner, endpoint_cap, rights)?;
    Ok(ipc.as_caps(target).insert(Capability::Connection {
        endpoint,
        rights: granted,
    }))
}

/// Resolves a capability usable as a connection-minting source.
///
/// Both endpoint caps and connection caps qualify, provided they carry
/// `MINT_CONNECTION`. Connection caps allow re-delegation with rights
/// attenuation: the minted rights are the intersection of the requested
/// rights and the source cap's rights.
fn mintable_endpoint(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
    requested: ConnectionRights,
) -> Result<(EndpointId, ConnectionRights), IpcError> {
    let (endpoint, source_rights) = match ipc.cap(asid, cap)? {
        Capability::Endpoint {
            endpoint,
            rights,
        }
        | Capability::Connection {
            endpoint,
            rights,
        } => (endpoint, rights),
        _ => return Err(IpcError::WrongType),
    };
    if !source_rights.contains(ConnectionRights::MINT_CONNECTION) {
        return Err(IpcError::PermissionDenied);
    }
    Ok((endpoint, requested.intersection(source_rights)))
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

    let delivery = enqueue_scalar(&mut ipc, endpoint_id, sender, opcode, arg0, None)?;
    drop(ipc);
    deliver(delivery);
    Ok(())
}

pub fn scalar_send_with_memory_move(
    sender: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
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

    let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
    let server_memory_cap = crate::memory::object::move_to(sender, memory_cap, server)
        .map_err(|_| IpcError::MemoryTransferFailed)?;
    let delivery = enqueue_scalar_with_memory(
        &mut ipc,
        endpoint_id,
        sender,
        opcode,
        arg0,
        None,
        Some(server_memory_cap),
    )?;
    drop(ipc);
    deliver(delivery);
    Ok(())
}

pub fn scalar_send_with_memory_copy(
    sender: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
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

    let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
    let server_memory_cap = crate::memory::object::copy_to(sender, memory_cap, server)
        .map_err(|_| IpcError::MemoryTransferFailed)?;
    let delivery = enqueue_scalar_with_memory(
        &mut ipc,
        endpoint_id,
        sender,
        opcode,
        arg0,
        None,
        Some(server_memory_cap),
    )?;
    drop(ipc);
    deliver(delivery);
    Ok(())
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
            observed: false,
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
            borrow: None,
        },
    );
    let token_cap = ipc.as_caps(server).insert(Capability::ReplyToken {
        token,
    });

    let delivery =
        enqueue_scalar(&mut ipc, endpoint_id, caller, opcode, arg0, Some(token_cap))?;
    drop(ipc);
    deliver(delivery);
    Ok(call_cap)
}

/// Scalar call carrying a delegated connection capability.
///
/// The caller attaches a connection to an endpoint it controls (either an
/// endpoint cap or a re-delegable connection cap bearing `MINT_CONNECTION`).
/// The kernel mints the attenuated connection into the receiving domain's
/// capability table and delivers its id together with the message. This is
/// the primitive that lets a service hand its endpoint authority to a name
/// or policy service without either side naming address spaces.
pub fn scalar_call_with_connection(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    delegate_cap: CapabilityId,
    delegate_rights: ConnectionRights,
) -> Result<CapabilityId, IpcError> {
    scalar_call_with_connection_impl(
        caller,
        connection_cap,
        opcode,
        arg0,
        delegate_cap,
        delegate_rights,
        None,
    )
}

/// Scalar call carrying a delegated connection capability *and* a copied
/// memory object.
///
/// Combined attachments allow a single registration call to deliver both a
/// service's endpoint authority and a memory-carried payload (for example a
/// long service name): the receiver observes the copied memory cap and the
/// minted connection cap together with one message.
pub fn scalar_call_with_connection_copy(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    delegate_cap: CapabilityId,
    delegate_rights: ConnectionRights,
    memory_cap: MemoryObjectCap,
) -> Result<CapabilityId, IpcError> {
    scalar_call_with_connection_impl(
        caller,
        connection_cap,
        opcode,
        arg0,
        delegate_cap,
        delegate_rights,
        Some(memory_cap),
    )
}

#[allow(clippy::too_many_arguments)]
fn scalar_call_with_connection_impl(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    delegate_cap: CapabilityId,
    delegate_rights: ConnectionRights,
    copied_memory: Option<MemoryObjectCap>,
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
    let (delegated_endpoint, granted) =
        mintable_endpoint(&ipc, caller, delegate_cap, delegate_rights)?;

    let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
    let server_memory_cap = if let Some(memory_cap) = copied_memory {
        Some(
            crate::memory::object::copy_to(caller, memory_cap, server)
                .map_err(|_| IpcError::MemoryTransferFailed)?,
        )
    } else {
        None
    };
    let attached_cap = ipc.as_caps(server).insert(Capability::Connection {
        endpoint: delegated_endpoint,
        rights: granted,
    });

    let call = ipc.alloc_call();
    ipc.pending_calls.insert(
        call,
        PendingCall {
            caller,
            result: None,
            observed: false,
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
            borrow: None,
        },
    );
    let token_cap = ipc.as_caps(server).insert(Capability::ReplyToken {
        token,
    });

    let delivery = enqueue_message(
        &mut ipc,
        endpoint_id,
        caller,
        opcode,
        arg0,
        Some(token_cap),
        server_memory_cap,
        Some(attached_cap),
    )?;
    drop(ipc);
    deliver(delivery);
    Ok(call_cap)
}

pub fn scalar_call_with_memory_move(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
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

    let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
    let server_memory_cap = crate::memory::object::move_to(caller, memory_cap, server)
        .map_err(|_| IpcError::MemoryTransferFailed)?;

    let call = ipc.alloc_call();
    ipc.pending_calls.insert(
        call,
        PendingCall {
            caller,
            result: None,
            observed: false,
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
            borrow: None,
        },
    );
    let token_cap = ipc.as_caps(server).insert(Capability::ReplyToken {
        token,
    });

    let delivery = enqueue_scalar_with_memory(
        &mut ipc,
        endpoint_id,
        caller,
        opcode,
        arg0,
        Some(token_cap),
        Some(server_memory_cap),
    )?;
    drop(ipc);
    deliver(delivery);
    Ok(call_cap)
}

pub fn scalar_call_with_memory_copy(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
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

    let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
    let server_memory_cap = crate::memory::object::copy_to(caller, memory_cap, server)
        .map_err(|_| IpcError::MemoryTransferFailed)?;

    let call = ipc.alloc_call();
    ipc.pending_calls.insert(
        call,
        PendingCall {
            caller,
            result: None,
            observed: false,
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
            borrow: None,
        },
    );
    let token_cap = ipc.as_caps(server).insert(Capability::ReplyToken {
        token,
    });

    let delivery = enqueue_scalar_with_memory(
        &mut ipc,
        endpoint_id,
        caller,
        opcode,
        arg0,
        Some(token_cap),
        Some(server_memory_cap),
    )?;
    drop(ipc);
    deliver(delivery);
    Ok(call_cap)
}

pub fn scalar_call_with_memory_borrow_read(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
) -> Result<CapabilityId, IpcError> {
    scalar_call_with_memory_borrow(caller, connection_cap, opcode, arg0, memory_cap, false)
}

pub fn scalar_call_with_memory_borrow_write(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
) -> Result<CapabilityId, IpcError> {
    scalar_call_with_memory_borrow(caller, connection_cap, opcode, arg0, memory_cap, true)
}

fn scalar_call_with_memory_borrow(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
    writable: bool,
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

    let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
    let server_memory_cap = if writable {
        crate::memory::object::lend_write(caller, memory_cap, server)
    } else {
        crate::memory::object::lend_read(caller, memory_cap, server)
    }
    .map_err(|_| IpcError::MemoryTransferFailed)?;

    let call = ipc.alloc_call();
    ipc.pending_calls.insert(
        call,
        PendingCall {
            caller,
            result: None,
            observed: false,
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
            borrow: Some(MemoryBorrow {
                owner: caller,
                owner_cap: memory_cap,
                borrower: server,
                borrower_cap: server_memory_cap,
            }),
        },
    );
    let token_cap = ipc.as_caps(server).insert(Capability::ReplyToken {
        token,
    });

    let delivery = enqueue_scalar_with_memory(
        &mut ipc,
        endpoint_id,
        caller,
        opcode,
        arg0,
        Some(token_cap),
        Some(server_memory_cap),
    )?;
    drop(ipc);
    deliver(delivery);
    Ok(call_cap)
}

/// What must be signalled after an enqueue, once the IPC lock is dropped:
/// blocked endpoint receivers, plus (for CQ-bound endpoints) a coalesced
/// readiness wake on the owner's completion queue.
struct Delivery {
    observers: Vec<Weak<dyn Observer>>,
    cq_wake: Option<(AddressSpaceId, crate::completion::CqId)>,
}

fn deliver(delivery: Delivery) {
    if let Some((asid, cq)) = delivery.cq_wake {
        crate::completion::wake(asid, cq);
    }
    signal_observers(delivery.observers);
}

fn enqueue_scalar(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<CapabilityId>,
) -> Result<Delivery, IpcError> {
    enqueue_message(ipc, endpoint_id, sender, opcode, arg0, reply, None, None)
}

fn enqueue_scalar_with_memory(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<CapabilityId>,
    memory: Option<MemoryObjectCap>,
) -> Result<Delivery, IpcError> {
    enqueue_message(ipc, endpoint_id, sender, opcode, arg0, reply, memory, None)
}

#[allow(clippy::too_many_arguments)]
fn enqueue_message(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<CapabilityId>,
    memory: Option<MemoryObjectCap>,
    connection: Option<CapabilityId>,
) -> Result<Delivery, IpcError> {
    let endpoint = ipc.endpoints.get_mut(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    if endpoint.queue.len() >= endpoint.capacity {
        return Err(IpcError::QueueFull);
    }
    let was_empty = endpoint.queue.is_empty();
    endpoint.queue.push_back(QueuedMessage {
        sender,
        opcode,
        arg0,
        reply,
        memory,
        connection,
    });
    // Coalesced readiness (§9.4): only the empty→nonempty transition posts a
    // CQ wake; further messages are observed when the receiver drains.
    let cq_wake = if was_empty {
        endpoint.notify_cq.map(|cq| (endpoint.owner, cq))
    } else {
        None
    };
    Ok(Delivery {
        observers: drain_observers(&endpoint.observers),
        cq_wake,
    })
}

fn reserve_endpoint_queue(
    ipc: &IpcRegistry,
    endpoint_id: EndpointId,
) -> Result<AddressSpaceId, IpcError> {
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    if endpoint.queue.len() >= endpoint.capacity {
        return Err(IpcError::QueueFull);
    }
    Ok(endpoint.owner)
}

pub fn receive(
    receiver: AddressSpaceId,
    endpoint_cap: CapabilityId,
) -> Result<ScalarMessage, IpcError> {
    let mut ipc = IPC.write();
    let endpoint_id = receive_endpoint_id(&ipc, receiver, endpoint_cap)?;

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
        memory: message.memory,
        connection: message.connection,
    })
}

pub fn wait_readable(receiver: AddressSpaceId, endpoint_cap: CapabilityId) -> Result<(), IpcError> {
    let endpoint_id = {
        let ipc = IPC.read();
        receive_endpoint_id(&ipc, receiver, endpoint_cap)?
    };
    if endpoint_is_readable_or_closed(endpoint_id)? {
        return Ok(());
    }

    let tid = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
        .read()
        .get_lp_scheduler()
        .lock()
        .get_tid()
        .ok_or(IpcError::NoMessage)?;
    let observable = EndpointObservable {
        endpoint: endpoint_id,
    };
    crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
        .write()
        .block_thread(tid, &observable)
        .map_err(|_| IpcError::NoMessage)?;

    // Lost-wake guard: if a sender enqueued after the fast-path check but
    // before observer registration completed, re-admit the thread immediately.
    if endpoint_is_readable_or_closed(endpoint_id)? {
        let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .write()
            .submit_ready_thread(tid);
    }

    crate::cpu::scheduler::yield_lp();
    Ok(())
}

fn receive_endpoint_id(
    ipc: &IpcRegistry,
    receiver: AddressSpaceId,
    endpoint_cap: CapabilityId,
) -> Result<EndpointId, IpcError> {
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
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    Ok(endpoint_id)
}

fn endpoint_is_readable_or_closed(endpoint_id: EndpointId) -> Result<bool, IpcError> {
    let ipc = IPC.read();
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    Ok(endpoint.closed || !endpoint.queue.is_empty())
}

struct EndpointObservable {
    endpoint: EndpointId,
}

impl Observable for EndpointObservable {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        let ipc = IPC.read();
        if let Some(endpoint) = ipc.endpoints.get(&self.endpoint) {
            let _ = endpoint.observers.push(observer);
        }
    }
}

pub fn reply(server: AddressSpaceId, reply_cap: CapabilityId, result: i64) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    complete_reply(&mut ipc, server, reply_cap, result, None, None)
}

pub fn reply_with_connection(
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    endpoint_cap: CapabilityId,
    rights: ConnectionRights,
    result: i64,
) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let (endpoint, granted) = mintable_endpoint(&ipc, server, endpoint_cap, rights)?;
    complete_reply(&mut ipc, server, reply_cap, result, Some((endpoint, granted)), None)
}

pub fn reply_with_memory_move(
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    memory_cap: MemoryObjectCap,
    result: i64,
) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    complete_reply(&mut ipc, server, reply_cap, result, None, Some(memory_cap))
}

fn complete_reply(
    ipc: &mut IpcRegistry,
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    result: i64,
    returned_connection: Option<(EndpointId, ConnectionRights)>,
    returned_memory: Option<MemoryObjectCap>,
) -> Result<(), IpcError> {
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
    let borrow = token.borrow;
    let call_id = token.call;
    let caller = ipc.pending_calls.get(&call_id).ok_or(IpcError::UnknownCapability)?.caller;
    let returned_memory_cap = if let Some(memory_cap) = returned_memory {
        Some(
            crate::memory::object::move_to(server, memory_cap, caller)
                .map_err(|_| IpcError::MemoryTransferFailed)?,
        )
    } else {
        None
    };
    if let Some(borrow) = borrow {
        revoke_memory_borrow(borrow).map_err(|_| IpcError::MemoryTransferFailed)?;
    }
    let returned_cap = returned_connection.map(|(endpoint, endpoint_rights)| {
        ipc.as_caps(caller).insert(Capability::Connection {
            endpoint,
            rights: endpoint_rights,
        })
    });
    ipc.reply_tokens.remove(&token_id);
    let call = ipc.pending_calls.get_mut(&call_id).ok_or(IpcError::UnknownCapability)?;
    call.result = Some(ReplyValue {
        result,
        cap: returned_cap,
        memory: returned_memory_cap,
    });
    let _ = ipc.remove_cap(server, reply_cap);
    Ok(())
}

pub fn poll_reply(
    caller: AddressSpaceId,
    call_cap: CapabilityId,
) -> Result<Option<ReplyValue>, IpcError> {
    let mut ipc = IPC.write();
    let call_id = match ipc.cap(caller, call_cap)? {
        Capability::PendingCall {
            call,
        } => call,
        _ => return Err(IpcError::WrongType),
    };
    let call = ipc.pending_calls.get_mut(&call_id).ok_or(IpcError::UnknownCapability)?;
    if call.caller != caller {
        return Err(IpcError::PermissionDenied);
    }
    if call.result.is_some() {
        // Once the caller has observed the result, any returned capabilities
        // are its own; closing the pending-call cap must not revoke them.
        call.observed = true;
    }
    Ok(call.result)
}

pub fn close_cap(asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let mut observers = Vec::new();
    let mut cq_wake = None;
    match ipc.remove_cap(asid, cap)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => {
            let queued = if let Some(endpoint) = ipc.endpoints.get_mut(&endpoint) {
                if endpoint.owner != asid {
                    Vec::new()
                } else {
                    endpoint.closed = true;
                    observers.extend(drain_observers(&endpoint.observers));
                    // A CQ-bound endpoint reports its closure as a readiness
                    // wake so a reactor blocked on one CQ wait observes it.
                    cq_wake = endpoint.notify_cq.map(|cq| (endpoint.owner, cq));
                    endpoint.queue.drain(..).collect()
                }
            } else {
                Vec::new()
            };
            for message in queued {
                if let Some(reply_cap) = message.reply {
                    consume_reply_cap(&mut ipc, asid, reply_cap, REPLY_ENDPOINT_CLOSED);
                }
                if let Some(memory_cap) = message.memory {
                    let _ = crate::memory::object::close_cap(asid, memory_cap);
                }
                if let Some(connection_cap) = message.connection {
                    let _ = ipc.remove_cap(asid, connection_cap);
                }
            }
        }
        Capability::PendingCall {
            call,
        } => {
            if let Some(pending) = ipc.pending_calls.remove(&call) {
                if let Some(reply) = pending.result {
                    // Revoke undelivered results only. Once the caller has
                    // observed the reply, the returned capabilities are its
                    // property and survive the pending-call close.
                    if !pending.observed {
                        if let Some(returned_cap) = reply.cap {
                            let _ = ipc.remove_cap(asid, returned_cap);
                        }
                        if let Some(memory_cap) = reply.memory {
                            let _ = crate::memory::object::close_cap(asid, memory_cap);
                        }
                    }
                } else {
                    cancel_queued_call(&mut ipc, call);
                }
            }
        }
        Capability::ReplyToken {
            token,
        } => {
            if let Some(token) = ipc.reply_tokens.remove(&token) {
                if let Some(borrow) = token.borrow {
                    let _ = revoke_memory_borrow(borrow);
                }
                if let Some(call) = ipc.pending_calls.get_mut(&token.call) {
                    call.result = Some(ReplyValue {
                        result: REPLY_CANCELLED,
                        cap: None,
                        memory: None,
                    });
                }
            }
        }
        Capability::Connection {
            ..
        } => {}
    }
    drop(ipc);
    if let Some((owner, cq)) = cq_wake {
        crate::completion::wake(owner, cq);
    }
    signal_observers(observers);
    Ok(())
}

fn drain_observers(queue: &ConcurrentQueue<Weak<dyn Observer>>) -> Vec<Weak<dyn Observer>> {
    queue.try_iter().collect()
}

fn signal_observers(observers: Vec<Weak<dyn Observer>>) {
    for observer in observers {
        if let Some(observer) = observer.upgrade() {
            observer.notify();
        }
    }
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
        if let Some(borrow) = token.borrow {
            let _ = revoke_memory_borrow(borrow);
        }
        if let Some(call) = ipc.pending_calls.get_mut(&token.call) {
            call.result = Some(ReplyValue {
                result,
                cap: None,
                memory: None,
            });
        }
    }
}

fn cancel_queued_call(ipc: &mut IpcRegistry, call: PendingCallId) {
    let tokens = ipc
        .reply_tokens
        .iter()
        .filter_map(|(token_id, token)| {
            if token.call == call {
                Some((
                    *token_id,
                    token.server,
                    token.borrow,
                    reply_cap_for_token(ipc, token.server, *token_id),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for (token, server, borrow, reply_cap) in tokens {
        if let Some(reply_cap) = reply_cap {
            cancel_queued_message_with_reply(ipc, server, reply_cap, borrow);
        } else if let Some(borrow) = borrow {
            let _ = revoke_memory_borrow(borrow);
        }
        ipc.reply_tokens.remove(&token);
        ipc.remove_matching_caps(
            server,
            Capability::ReplyToken {
                token,
            },
        );
    }
}

fn reply_cap_for_token(
    ipc: &IpcRegistry,
    server: AddressSpaceId,
    token: ReplyTokenId,
) -> Option<CapabilityId> {
    ipc.caps.get(&server).and_then(|caps| {
        caps.caps.iter().find_map(|(cap_id, cap)| {
            if *cap
                == (Capability::ReplyToken {
                    token,
                })
            {
                Some(*cap_id)
            } else {
                None
            }
        })
    })
}

fn cancel_queued_message_with_reply(
    ipc: &mut IpcRegistry,
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    borrow: Option<MemoryBorrow>,
) {
    for endpoint in ipc.endpoints.values_mut() {
        if endpoint.owner != server {
            continue;
        }
        if let Some(index) =
            endpoint.queue.iter().position(|message| message.reply == Some(reply_cap))
        {
            if let Some(message) = endpoint.queue.remove(index) {
                if let Some(borrow) = borrow {
                    let _ = revoke_memory_borrow(borrow);
                } else if let Some(memory_cap) = message.memory {
                    let _ = crate::memory::object::close_cap(server, memory_cap);
                }
                if let Some(connection_cap) = message.connection {
                    if let Some(caps) = ipc.caps.get_mut(&server) {
                        caps.caps.remove(&connection_cap);
                    }
                }
            }
            return;
        }
    }
    if let Some(borrow) = borrow {
        let _ = revoke_memory_borrow(borrow);
    }
}

fn revoke_memory_borrow(
    borrow: MemoryBorrow,
) -> Result<(), crate::memory::object::MemoryObjectError> {
    crate::memory::object::revoke_lend(
        borrow.owner,
        borrow.owner_cap,
        borrow.borrower,
        borrow.borrower_cap,
    )
}
