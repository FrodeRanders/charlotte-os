//! Experimental endpoint IPC.
//!
//! Xous-inspired messaging between protection domains, with scalar payloads
//! and capability attachments. Endpoints, connections, pending calls and reply
//! tokens have their own ownership and admission rules. Completion capabilities
//! and the LP-indexed mailbox smoke ABI are separate interfaces.

use alloc::{
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};
use core::{
    ops::BitOr,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use spin::LazyLock;

use crate::{
    cpu::multiprocessor::spin::rwlock::RwLock,
    klib::{
        collections::retirement_list::{
            AdmittedMap,
            PreparedEntry,
            RetiredEntry,
            RetirementList,
        },
        observer::{
            Observable,
            Observer,
            WaitRegistration,
            WaitSponsor,
            registration::{
                ListRef,
                NotificationBatch,
                ObserverList,
                RegistrationError,
            },
            waiter_budget,
        },
    },
    memory::{
        AddressSpaceId,
        object::MemoryObjectCap,
    },
};

mod attachments;
mod close;
pub(crate) use close::close_address_space_fixture;
pub use close::close_cap;
mod message;
use message::{
    Delivery,
    enqueue_message,
    enqueue_scalar,
    reserve_endpoint_queue,
};
pub use message::{
    receive,
    receive_vec,
};
pub(crate) mod budget;
mod call_storage_tests;
pub(crate) mod cancellation;
mod cap_storage;
pub(crate) mod claim_wait;
pub(crate) mod delivery_tests;
mod endpoint_close;
mod endpoint_storage;
pub(crate) mod namespace;
pub(crate) mod namespace_close;
mod preparation_tests;
mod publication_tests;
pub(crate) mod record_budget;
pub(crate) mod record_tests;
pub(crate) mod reply;
mod reply_storage_tests;
mod submission;
mod transaction;
use transaction::{
    PendingStorage,
    PreparedCall,
    PreparedConnection,
    ReplyCompletion,
    ReplyStorage,
    RetiredCall,
    RetiredReplies,
    RetiredReply,
};
mod readiness;
use attachments::MemoryAttachments;
use cap_storage::{
    PreparingCapability,
    RetiredMetadata,
};
pub(crate) use readiness::waiter_tests;
pub use readiness::{
    IPC_WAIT_ADMISSION_RETRIES,
    bind_reply_cq,
    wait_readable,
    wait_reply,
    wait_reply_timeout,
    watch_connection_closed,
};
use readiness::{
    ReplyCqInterest,
    endpoint_available,
    receive_endpoint_id,
};

type WaiterList = ListRef<waiter_budget::Charge>;
type WaitNotifications = NotificationBatch<waiter_budget::Charge>;

pub type CapabilityId = u64;
type EndpointId = u64;
type ReplyTokenId = u64;
type PendingCallId = u64;

pub const REPLY_CANCELLED: i64 = -3;
pub const REPLY_ENDPOINT_CLOSED: i64 = -7;

/// Upper bound on a single endpoint's queued-message capacity. Endpoint
/// storage is kernel heap, so an EL0-provided capacity must be clamped before
/// it can drive unbounded allocation.
pub const MAX_ENDPOINT_CAPACITY: usize = crate::resource_policy::POLICY.local.endpoint_capacity;

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
    ResourceLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionRights(u32);

/// Maximum capability vector entries per message (fits in one 4 KiB page
/// with the `count` header). The page holds `count: u16` at offset 0,
/// followed by up to this many entries.
pub use catten_syscall::CAP_VECTOR_MAX;
/// A single entry in a capability vector page. The sender packs an array
/// of these into a one-page memory object and passes it to
/// `ipc_vector_send` / `ipc_vector_call`. Each entry specifies a
/// memory-object capability and how it should be transferred.
pub use catten_syscall::CapVectorEntry;

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
    pub sender_generation: u64,
    pub sender_principal: u64,
    pub sender_roles: u32,
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
struct AdmittedCapability {
    payload: Capability,
    /// Queued/unobserved connection authority is still exclusively IPC-owned.
    delivery_pending: bool,
    // Present only after removal; retained with payload storage until post-IPC disposal.
    retired_authority: Option<crate::capability::RetiredRecord>,
}

#[derive(Debug)]
struct AsIpcCaps {
    caps: AdmittedMap<CapabilityId, AdmittedCapability, Option<record_budget::Charge>>,
    address_space: Option<crate::memory::AddressSpaceHandle>,
    endpoint_budget: Arc<budget::DomainBudget>,
    record_budget: Arc<record_budget::DomainBudget>,
}

#[derive(Debug)]
struct QueuedMessage {
    sender: AddressSpaceId,
    sender_generation: u64,
    sender_principal: u64,
    sender_roles: u32,
    opcode: u32,
    arg0: u64,
    /// Internal token identity. The receiver-visible capability is allocated
    /// only when this message is dequeued.
    reply: Option<ReplyTokenId>,
    memory: MemoryAttachments,
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
    /// Deepest the queue has been, for capacity policy.
    high_water: usize,
    queue: budget::AdmittedQueue<QueuedMessage>,
    /// Threads waiting for the endpoint to become readable. These observers
    /// fire on message arrival and endpoint closure.
    readiness_observers: WaiterList,
    /// Lifecycle observers installed through `watch_connection_closed`.
    /// Unlike readiness observers, ordinary message delivery must not wake
    /// these: they fire exclusively when the endpoint closes.
    close_observers: ListRef<crate::completion::watch_budget::Charge>,
    closed: bool,
    /// Owned explicit close fences queue use without reporting terminal closure.
    /// Abandonment retains this claim and the owner's exact root lease.
    closing: bool,
    claim_interest: Option<claim_wait::Source>,
    claim_epoch: u64,
    /// When bound, endpoint readiness is delivered to this completion queue
    /// of the owner as a coalesced wake (architecture doc §16.3). The receiver
    /// must inspect the endpoint for work. Posted on the empty→nonempty queue
    /// transition and on closure, so a shard can block on one CQ wait for
    /// both kernel completions and endpoint work (§7, Phase 7).
    notify_cq: Option<crate::completion::CqId>,
    /// Original namespace and classification also sponsor later queue growth.
    budget: Arc<budget::DomainBudget>,
    platform: bool,
}

#[derive(Debug)]
struct ReplyToken {
    server: AddressSpaceId,
    call: PendingCallId,
    /// Captured from the original reservation, including for claim sources.
    platform: bool,
    /// Exclusive reply/cancellation owner; Drop cannot force-clear this claim.
    completing: bool,
    claim_interest: Option<claim_wait::Source>,
    claim_epoch: u64,
    /// Cancellation reached uncertain physical cleanup; never deliver/reply it.
    cleanup_failed: bool,
    /// Borrowed minting authority protected by the in-flight reply claim.
    connection_source: Option<CapabilityId>,
    borrows: Vec<MemoryBorrow>,
}

#[derive(Debug)]
struct PendingCall {
    caller: AddressSpaceId,
    result: Option<ReplyValue>,
    observers: WaiterList,
    cq_interest: Option<ReplyCqInterest>,
    /// Set once the caller has seen the result through `poll_reply`. From
    /// that point the returned connection/memory capabilities belong to the
    /// caller, and closing the pending-call cap no longer revokes them
    /// (state `ResultObserved` in the operation state machine).
    observed: bool,
}

impl PendingCall {
    /// Prepare before attachment transfer or publishing any call capability.
    fn try_new(caller: AddressSpaceId, platform: bool) -> Result<Self, IpcError> {
        Ok(Self {
            caller,
            result: None,
            observers: ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
                .map_err(|_| IpcError::ResourceLimit)?,
            cq_interest: None,
            observed: false,
        })
    }
}

impl Drop for PendingCall {
    fn drop(&mut self) {
        // A retained token must not retain an entry after source destruction.
        // Normal close detaches and notifies outside IPC before this runs.
        drop(self.observers.close());
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        drop(self.readiness_observers.close());
        drop(self.close_observers.close());
    }
}

#[derive(Debug)]
struct IpcRegistry {
    next_endpoint: EndpointId,
    next_reply: ReplyTokenId,
    next_call: PendingCallId,
    endpoints: AdmittedMap<EndpointId, Endpoint, Option<budget::Charge>>,
    reply_tokens: AdmittedMap<ReplyTokenId, ReplyToken, Option<record_budget::Charge>>,
    pending_calls: AdmittedMap<PendingCallId, PendingCall, Option<record_budget::Charge>>,
    caps: crate::klib::collections::retirement_list::AdmittedMap<AddressSpaceId, AsIpcCaps>,
}

impl IpcRegistry {
    fn new() -> Self {
        Self {
            next_endpoint: 1,
            next_reply: 1,
            next_call: 1,
            endpoints: AdmittedMap::new(),
            reply_tokens: AdmittedMap::new(),
            pending_calls: AdmittedMap::new(),
            caps: crate::klib::collections::retirement_list::AdmittedMap::new(),
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
        self.caps.get_mut(&asid).expect("admitted IPC namespace missing")
    }

    /// Prevent capability publication into a namespace being drained, including
    /// its short IPC-only retirement interval before the registry is removed.
    fn accepting_namespace(&mut self, sponsor: AddressSpaceId) -> Result<bool, IpcError> {
        let identity = crate::memory::current_address_space_handle(sponsor);
        let platform_identity = crate::memory::budget::platform_identity(sponsor);
        if identity.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
            return Err(IpcError::PermissionDenied);
        }
        let namespace = self.caps.get(&sponsor).ok_or(IpcError::PermissionDenied)?;
        if namespace.address_space != identity || !namespace.record_budget.accepting() {
            return Err(IpcError::PermissionDenied);
        }
        // Synthetic namespaces are confined to kernel fixtures/adapters.
        // Names and userspace-supplied roles cannot obtain the platform reserve.
        Ok(sponsor == crate::memory::KERNEL_ASID
            || platform_identity.is_some_and(|handle| Some(handle) == namespace.address_space))
    }

    fn reserve_records(
        &mut self,
        sponsor: AddressSpaceId,
        amount: [u64; 3],
    ) -> Result<record_budget::Charge, IpcError> {
        let platform = self.accepting_namespace(sponsor)?;
        let namespace = self.as_caps(sponsor);
        record_budget::reserve(&namespace.record_budget, platform, amount)
            .map_err(|_| IpcError::ResourceLimit)
    }

    fn reserve_cap(
        &mut self,
        owner: AddressSpaceId,
    ) -> Result<crate::capability::Reservation, IpcError> {
        self.accepting_namespace(owner)?;
        crate::capability::reserve_captured(
            owner,
            crate::capability::ObjectKind::Ipc,
            self.as_caps(owner).address_space,
        )
        .map_err(cap_admission_error)
    }

    fn cap(&self, asid: AddressSpaceId, cap: CapabilityId) -> Result<Capability, IpcError> {
        let entry = self.cap_entry(asid, cap)?;
        if entry.delivery_pending {
            return Err(IpcError::UnknownCapability);
        }
        Ok(entry.payload)
    }

    fn cap_entry(
        &self,
        asid: AddressSpaceId,
        cap: CapabilityId,
    ) -> Result<&AdmittedCapability, IpcError> {
        if !crate::capability::contains(asid, cap, crate::capability::ObjectKind::Ipc) {
            return Err(IpcError::UnknownCapability);
        }
        self.caps.get(&asid).and_then(|caps| caps.caps.get(&cap)).ok_or(IpcError::UnknownCapability)
    }

    /// Only internal teardown can consume IPC-owned undelivered connections.
    fn cap_for_cleanup(
        &self,
        asid: AddressSpaceId,
        cap: CapabilityId,
    ) -> Result<Capability, IpcError> {
        Ok(self.cap_entry(asid, cap)?.payload)
    }

    /// Receive/first result observation holds IPC exclusively and owns the
    /// exact receipt. Public close/mint/use cannot consume its hidden grant.
    fn deliver_connection(&mut self, asid: AddressSpaceId, cap: CapabilityId) {
        let entry = self
            .caps
            .get_mut(&asid)
            .and_then(|caps| caps.caps.get_mut(&cap))
            .expect("IPC delivery connection missing");
        assert!(matches!(entry.payload, Capability::Connection { .. }) && entry.delivery_pending);
        entry.delivery_pending = false;
    }

    fn remove_cap(
        &mut self,
        asid: AddressSpaceId,
        cap: CapabilityId,
        retired: &mut RetiredMetadata,
    ) -> Result<Capability, IpcError> {
        let removed = cap_storage::retire(self, asid, cap, retired)?;
        // Internal cancellation also revokes connections. It must reclaim a
        // closed endpoint just as public capability closure does.
        if let Capability::Connection {
            endpoint,
            ..
        } = removed
            && self.endpoints.get(&endpoint).is_some_and(|record| record.closed)
            && !endpoint_referenced(self, endpoint)
        {
            retired.push(self.detach_endpoint(endpoint).expect("unreferenced endpoint missing"));
        }
        Ok(removed)
    }

    fn detach_endpoint(
        &mut self,
        endpoint: EndpointId,
    ) -> Option<endpoint_storage::RetiredEndpoint> {
        if endpoint_referenced(self, endpoint) {
            return None;
        }
        let record = self.endpoints.get(&endpoint)?;
        assert!(
            record.closed && !record.closing && record.queue.is_empty(),
            "endpoint metadata retirement before queue/claim completion"
        );
        self.endpoints.take(&endpoint)
    }

    /// These callers retire endpoint/call/reply control authority, never a
    /// connection that can hold the final reference to another endpoint.
    fn remove_non_connection_cap(
        &mut self,
        asid: AddressSpaceId,
        cap: CapabilityId,
        retired: &mut RetiredMetadata,
    ) -> Result<Capability, IpcError> {
        if matches!(self.cap_for_cleanup(asid, cap)?, Capability::Connection { .. }) {
            return Err(IpcError::WrongType);
        }
        self.remove_cap(asid, cap, retired)
    }

    fn remove_matching_caps(
        &mut self,
        asid: AddressSpaceId,
        target: Capability,
        retired: &mut RetiredMetadata,
    ) {
        while let Some(id) = self.caps.get(&asid).and_then(|caps| {
            caps.caps.iter().find_map(|(&id, cap)| (cap.payload == target).then_some(id))
        }) {
            self.remove_non_connection_cap(asid, id, retired)
                .expect("matching IPC control capability disappeared");
        }
    }
}

static IPC: LazyLock<RwLock<IpcRegistry>> = LazyLock::new(|| RwLock::new(IpcRegistry::new()));

fn cap_admission_error(error: crate::capability::AllocationError) -> IpcError {
    match error {
        crate::capability::AllocationError::Retired => IpcError::PermissionDenied,
        _ => IpcError::ResourceLimit,
    }
}

/// Kernel diagnostics retain the original namespace even after ASID reuse.
pub(crate) fn endpoint_admission(owner: AddressSpaceId) -> Option<Arc<budget::DomainBudget>> {
    IPC.read().caps.get(&owner).map(|caps| caps.endpoint_budget.clone())
}

pub fn endpoint_create(
    owner: AddressSpaceId,
    interface: u64,
    version: u32,
    capacity: usize,
) -> Result<CapabilityId, IpcError> {
    endpoint_storage::create(owner, interface, version, capacity)
}

/// Binds an endpoint's readiness to one of the owner's completion queues.
///
/// Binding also posts a wake when the endpoint is already readable. After
/// binding, the kernel posts a coalesced wake whenever the endpoint's message
/// queue transitions from empty to nonempty, and when the endpoint closes.
/// This lets a shard block on a single CQ wait for both
/// kernel/device completions and endpoint work (architecture doc §7,
/// Phase 7). The wake asks the shard to inspect the endpoint; it carries
/// no completion record (§16.3).
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
    if endpoint.closing {
        return Err(IpcError::Pending);
    }
    endpoint.notify_cq = Some(cq);
    let already_readable = endpoint_available(&ipc, &ipc.endpoints[&endpoint_id]);
    drop(ipc);
    // Close the bind-versus-enqueue race. An enqueue before the binding could
    // not signal this CQ; an enqueue after it observes `notify_cq`. If the
    // queue was already non-empty while holding IPC's write lock, publish the
    // missing readiness edge now.
    if already_readable {
        crate::completion::wake(owner, cq);
    }
    Ok(())
}

/// Resize an owned endpoint's admission bound, clamped to the platform
/// maximum. Existing queued messages are preserved; a shrunken bound only
/// rejects new sends until the queue drains. Returns the effective capacity.
pub fn endpoint_resize(
    owner: AddressSpaceId,
    endpoint_cap: CapabilityId,
    new_capacity: usize,
) -> Result<usize, IpcError> {
    endpoint_storage::resize(owner, endpoint_cap, new_capacity)
}

/// Read an owned endpoint's `(capacity, depth, high-water depth)`.
pub fn endpoint_status(
    owner: AddressSpaceId,
    endpoint_cap: CapabilityId,
) -> Result<(usize, usize, usize), IpcError> {
    let ipc = IPC.read();
    let endpoint_id = match ipc.cap(owner, endpoint_cap)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => endpoint,
        _ => return Err(IpcError::WrongType),
    };
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.owner != owner {
        return Err(IpcError::PermissionDenied);
    }
    Ok((endpoint.capacity, endpoint.queue.len(), endpoint.high_water))
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
    let mut prepared = None;
    let result = (|| {
        let mut ipc = IPC.write();
        let (endpoint, granted) = mintable_endpoint(&ipc, owner, endpoint_cap, rights)?;
        ipc.accepting_namespace(target)?;
        PreparedConnection::prepare_into(
            &mut prepared,
            &mut ipc,
            owner,
            target,
            endpoint,
            granted,
        )?;
        let prepared = prepared.as_mut().unwrap();
        prepared.authority.publish()?;
        let cap = prepared.install(&mut ipc);
        ipc.deliver_connection(target, cap);
        Ok(cap)
    })();
    if let Some(prepared) = prepared {
        prepared.finish();
    }
    result
}

/// Resolve the owner of the endpoint named by a connection capability.
///
/// This is used by the kernel supervisor to bind an upgrade request to the
/// service the requesting manager can actually call. The caller must hold a
/// live connection with CALL authority; raw ASIDs supplied by userspace are
/// never trusted.
pub(crate) fn connection_endpoint_owner(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
) -> Result<AddressSpaceId, IpcError> {
    let ipc = IPC.read();
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
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    Ok(endpoint.owner)
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
    if ipc.endpoints.get(&endpoint).is_some_and(|endpoint| endpoint.closing) {
        return Err(IpcError::Pending);
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
    delivery.notify();
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
    let mut transfer = crate::memory::object::prepare_move(sender, memory_cap, server)
        .map_err(|_| IpcError::MemoryTransferFailed)?;
    let server_memory_cap = transfer.target_cap();
    let memory = MemoryAttachments::single(server_memory_cap)?;
    crate::memory::object::commit_undelivered_transfers(core::slice::from_mut(&mut transfer))
        .map_err(|_| IpcError::MemoryTransferFailed)?;
    drop(transfer);
    let delivery = enqueue_message(&mut ipc, endpoint_id, sender, opcode, arg0, None, memory, None)
        .expect("reserved memory-send endpoint changed under IPC ownership");
    drop(ipc);
    delivery.notify();
    Ok(())
}

pub fn scalar_send_with_memory_copy(
    sender: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
) -> Result<(), IpcError> {
    submission::send_copy(sender, connection_cap, opcode, arg0, memory_cap)
}

pub fn scalar_call(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
) -> Result<CapabilityId, IpcError> {
    PreparedCall::run(caller, connection_cap, |prepared| {
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

        reserve_endpoint_queue(&ipc, endpoint_id)?;
        prepared.stage(&mut ipc, caller)?;
        prepared.commit(&mut ipc, endpoint_id, opcode, arg0)
    })
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
    if let Some(memory_cap) = copied_memory {
        return submission::call_copy(
            caller,
            connection_cap,
            opcode,
            arg0,
            memory_cap,
            Some((delegate_cap, delegate_rights)),
        );
    }
    PreparedCall::run(caller, connection_cap, |prepared| {
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
        let (delegated_endpoint, granted) = if delegate_cap == 0 {
            (0, ConnectionRights(0))
        } else {
            mintable_endpoint(&ipc, caller, delegate_cap, delegate_rights)?
        };

        let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
        prepared.stage(&mut ipc, caller)?;
        PreparedConnection::prepare_into(
            &mut prepared.connection,
            &mut ipc,
            caller,
            server,
            delegated_endpoint,
            granted,
        )?;
        prepared.commit(&mut ipc, endpoint_id, opcode, arg0)
    })
}

pub fn scalar_call_with_memory_move(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
) -> Result<CapabilityId, IpcError> {
    PreparedCall::run(caller, connection_cap, |prepared| {
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
        prepared.stage(&mut ipc, caller)?;
        prepared.attach(
            crate::memory::object::prepare_move(caller, memory_cap, server)
                .map_err(|_| IpcError::MemoryTransferFailed)?,
        )?;
        prepared.commit(&mut ipc, endpoint_id, opcode, arg0)
    })
}

pub fn scalar_call_with_memory_copy(
    caller: AddressSpaceId,
    connection_cap: CapabilityId,
    opcode: u32,
    arg0: u64,
    memory_cap: MemoryObjectCap,
) -> Result<CapabilityId, IpcError> {
    submission::call_copy(caller, connection_cap, opcode, arg0, memory_cap, None)
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
    PreparedCall::run(caller, connection_cap, |prepared| {
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
        prepared.stage(&mut ipc, caller)?;
        prepared.attach(
            crate::memory::object::prepare_loan(caller, memory_cap, server, writable)
                .map_err(|_| IpcError::MemoryTransferFailed)?,
        )?;
        prepared.commit(&mut ipc, endpoint_id, opcode, arg0)
    })
}

pub fn reply(server: AddressSpaceId, reply_cap: CapabilityId, result: i64) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let token = match ipc.cap(server, reply_cap)? {
        Capability::ReplyToken {
            token,
        } => token,
        _ => return Err(IpcError::WrongType),
    };
    if !ipc.reply_tokens.get(&token).ok_or(IpcError::UnknownCapability)?.borrows.is_empty() {
        drop(ipc);
        return reply::complete(server, reply_cap, result);
    }
    let mut completed = ReplyCompletion::empty();
    let result = completed.publish(&mut ipc, server, reply_cap, result, None, None);
    drop(ipc);
    completed.finish();
    result
}

/// Return attenuated connection authority. If loan cleanup needs an unlocked
/// interval, a connection source must already be delivered/observed; queued or
/// unobserved-result authority returns Pending without changing the reply/loans.
pub fn reply_with_connection(
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    endpoint_cap: CapabilityId,
    rights: ConnectionRights,
    result: i64,
) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let (endpoint, granted) = mintable_endpoint(&ipc, server, endpoint_cap, rights)?;
    let token = match ipc.cap(server, reply_cap)? {
        Capability::ReplyToken {
            token,
        } => token,
        _ => return Err(IpcError::WrongType),
    };
    if !ipc.reply_tokens.get(&token).ok_or(IpcError::UnknownCapability)?.borrows.is_empty() {
        drop(ipc);
        return reply::complete_with_connection(server, reply_cap, endpoint_cap, rights, result);
    }
    let mut completed = ReplyCompletion::empty();
    let result =
        completed.publish(&mut ipc, server, reply_cap, result, Some((endpoint, granted)), None);
    drop(ipc);
    completed.finish();
    result
}

pub fn reply_with_memory_move(
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    memory_cap: MemoryObjectCap,
    result: i64,
) -> Result<(), IpcError> {
    let mut ipc = IPC.write();
    let token = match ipc.cap(server, reply_cap)? {
        Capability::ReplyToken {
            token,
        } => token,
        _ => return Err(IpcError::WrongType),
    };
    if !ipc.reply_tokens.get(&token).ok_or(IpcError::UnknownCapability)?.borrows.is_empty() {
        drop(ipc);
        return reply::complete_with_memory(server, reply_cap, memory_cap, result);
    }
    let mut completed = ReplyCompletion::empty();
    let result = completed.publish(&mut ipc, server, reply_cap, result, None, Some(memory_cap));
    drop(ipc);
    completed.finish();
    result
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
    // Mark observation under this single write hold, before returning, so a
    // concurrent close_cap cannot revoke the returned capabilities after a
    // poller has seen them. Polling is repeatable by contract: later polls
    // return the same reply (callers that adopt returned capabilities track
    // their own one-shot state).
    let result = call.result;
    let observe = !call.observed;
    if let Some(reply) = result
        && observe
    {
        if let Some(cap) = reply.cap {
            ipc.deliver_connection(caller, cap);
        }
        crate::memory::object::deliver_memory(caller, reply.memory.as_slice());
        ipc.pending_calls.get_mut(&call_id).unwrap().observed = true;
    }
    Ok(result)
}

/// Whether any live capability still names `endpoint`. The unified `Endpoint`
/// cap and delegated `Connection` caps are the only holders of an endpoint id;
/// queued messages and reply tokens are drained before an endpoint is retired.
fn endpoint_referenced(ipc: &IpcRegistry, endpoint: EndpointId) -> bool {
    ipc.caps.values().any(|as_caps| {
        as_caps.caps.values().any(|cap| match &cap.payload {
            Capability::Endpoint {
                endpoint: id,
                ..
            }
            | Capability::Connection {
                endpoint: id,
                ..
            } => *id == endpoint,
            Capability::ReplyToken {
                ..
            }
            | Capability::PendingCall {
                ..
            } => false,
        })
    })
}

pub(crate) fn connection_close_watch_count(
    asid: AddressSpaceId,
    cap: CapabilityId,
) -> Result<usize, IpcError> {
    let ipc = IPC.read();
    let endpoint = match ipc.cap(asid, cap)? {
        Capability::Connection {
            endpoint,
            ..
        } => endpoint,
        _ => return Err(IpcError::WrongType),
    };
    Ok(ipc
        .endpoints
        .get(&endpoint)
        .ok_or(IpcError::UnknownCapability)?
        .close_observers
        .registered())
}

fn signal_observers(observers: WaitNotifications) {
    observers.notify();
}

fn revoke_memory_borrow(
    borrow: MemoryBorrow,
) -> Result<(), crate::memory::object::MemoryObjectError> {
    crate::memory::object::revoke_lend_under_ipc(
        borrow.owner,
        borrow.owner_cap,
        borrow.borrower,
        borrow.borrower_cap,
    )
}

/// Send a vector of memory objects through a connection. Each entry in
/// the cap vector page specifies a capability and transfer mode. The
/// kernel validates and transfers each entry before enqueuing.
pub fn vector_send(
    sender: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    cap_vector: MemoryObjectCap,
) -> Result<(), IpcError> {
    submission::vector(sender, connection, opcode, arg0, cap_vector, false).map(|_| ())
}

pub fn vector_call(
    caller: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    cap_vector: MemoryObjectCap,
) -> Result<CapabilityId, IpcError> {
    submission::vector(caller, connection, opcode, arg0, cap_vector, true)
        .map(|call| call.expect("vector call did not publish call authority"))
}

fn read_vector_page(
    sender: AddressSpaceId,
    cap_vector_page: MemoryObjectCap,
    target: AddressSpaceId,
    is_call: bool,
    out: &mut Vec<MemoryObjectCap>,
) -> Result<Vec<crate::memory::object::PreparedTransfer>, IpcError> {
    let vector_bytes = crate::memory::object::snapshot_bytes(
        sender,
        cap_vector_page,
        2 + CAP_VECTOR_MAX * core::mem::size_of::<CapVectorEntry>(),
    )
    .map_err(|_| IpcError::UnknownCapability)?;
    let count = u16::from_le_bytes(
        vector_bytes
            .get(..2)
            .ok_or(IpcError::MemoryTransferFailed)?
            .try_into()
            .map_err(|_| IpcError::MemoryTransferFailed)?,
    ) as usize;
    if count == 0 || count > CAP_VECTOR_MAX {
        return Err(IpcError::MemoryTransferFailed);
    }
    let entries_ptr = unsafe { vector_bytes.as_ptr().add(2) } as *const CapVectorEntry;
    let mut entries = Vec::new();
    entries.try_reserve_exact(count).map_err(|_| IpcError::ResourceLimit)?;
    for i in 0..count {
        let entry = unsafe { core::ptr::read_unaligned(entries_ptr.add(i)) };
        if entry.reserved != 0 || entry.mode > 3 || (!is_call && entry.mode >= 2) {
            return Err(IpcError::MemoryTransferFailed);
        }
        if entries.iter().any(|prior: &CapVectorEntry| prior.cap == entry.cap) {
            return Err(IpcError::MemoryTransferFailed);
        }
        entries.push(entry);
    }

    let mut transfers = Vec::new();
    transfers.try_reserve_exact(count).map_err(|_| IpcError::ResourceLimit)?;
    out.try_reserve_exact(count).map_err(|_| IpcError::ResourceLimit)?;
    for entry in entries {
        let cap = entry.cap as MemoryObjectCap;
        let transfer = match entry.mode {
            0 => crate::memory::object::prepare_copy(sender, cap, target),
            1 => crate::memory::object::prepare_move(sender, cap, target),
            2 => crate::memory::object::prepare_loan(sender, cap, target, false),
            3 => crate::memory::object::prepare_loan(sender, cap, target, true),
            _ => unreachable!(),
        }
        .map_err(|_| IpcError::MemoryTransferFailed)?;
        out.push(transfer.target_cap());
        transfers.push(transfer);
    }
    Ok(transfers)
}
