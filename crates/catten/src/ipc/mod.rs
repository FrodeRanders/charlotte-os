//! Experimental endpoint IPC.
//!
//! This is the first Xous-inspired cross-protection-domain IPC substrate. It is
//! deliberately scalar-only: endpoints, connections, pending calls, and reply
//! tokens are separate from completion capabilities and from the LP-indexed
//! mailbox smoke ABI.

use alloc::{
    collections::BTreeMap,
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
    klib::observer::{
        Observable,
        Observer,
        WaitRegistration,
        WaitSponsor,
        registration::{
            NotificationBatch,
            ObserverList,
            RegistrationError,
        },
        waiter_budget,
    },
    memory::{
        AddressSpaceId,
        object::MemoryObjectCap,
    },
};

pub(crate) mod budget;
pub(crate) mod cancellation;
pub(crate) mod record_budget;
pub(crate) mod record_tests;
pub(crate) mod reply;
pub(crate) mod waiter_tests;

type WaiterList = Arc<ObserverList<waiter_budget::Charge>>;
type WaitNotifications = NotificationBatch<waiter_budget::Charge>;

/// Debugger-visible cooperative admission retries: readable wait, reply wait.
/// Diagnostic counters only; they grant no authority and do not drive policy.
#[unsafe(no_mangle)]
pub static IPC_WAIT_ADMISSION_RETRIES: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

pub type CapabilityId = u64;
type EndpointId = u64;
type ReplyTokenId = u64;
type PendingCallId = u64;

pub const REPLY_CANCELLED: i64 = -3;
pub const REPLY_ENDPOINT_CLOSED: i64 = -7;

/// Upper bound on a single endpoint's queued-message capacity. Endpoint
/// storage is kernel heap, so an EL0-provided capacity must be clamped before
/// it can drive unbounded allocation.
pub const MAX_ENDPOINT_CAPACITY: usize = 4096;

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

/// The kernel fills this struct into the receiver's result page during
/// `ipc_recv_vec`. At most `count` caps were delivered; the receiver
/// must close each one after use.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VectorMessage {
    /// Number of memory-object caps returned.
    pub memory_count: u16,
    /// Reserved.
    pub _pad: [u16; 3],
    /// Cap IDs of the transferred memory objects.
    pub memory_caps: [u64; CAP_VECTOR_MAX],
}

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
    // Lookup copies only payload authority. The unique stored entry retains
    // sponsorship until removal, including internal cancellation paths.
    _connection_charge: Option<record_budget::Charge>,
}

#[derive(Debug)]
struct AsIpcCaps {
    caps: BTreeMap<CapabilityId, AdmittedCapability>,
    address_space: Option<crate::memory::AddressSpaceHandle>,
    endpoint_budget: Arc<budget::DomainBudget>,
    record_budget: Arc<record_budget::DomainBudget>,
}

impl AsIpcCaps {
    fn new(asid: AddressSpaceId) -> Self {
        Self {
            caps: BTreeMap::new(),
            address_space: crate::memory::current_address_space_handle(asid),
            endpoint_budget: budget::DomainBudget::new(),
            record_budget: record_budget::DomainBudget::new(),
        }
    }

    /// The caller has published this identity under IPC serialization. No
    /// fallible admission may remain after mutating its associated payload.
    fn insert_admitted(
        &mut self,
        id: CapabilityId,
        cap: Capability,
        charge: Option<record_budget::Charge>,
    ) -> CapabilityId {
        assert_eq!(matches!(cap, Capability::Connection { .. }), charge.is_some());
        self.caps.insert(
            id,
            AdmittedCapability {
                payload: cap,
                _connection_charge: charge,
            },
        );
        id
    }
}

#[derive(Debug, Clone)]
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
    memory: Vec<MemoryObjectCap>,
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
    close_observers: Arc<ObserverList<crate::completion::watch_budget::Charge>>,
    closed: bool,
    /// When bound, endpoint readiness is delivered to this completion queue
    /// of the owner as a coalesced wake (architecture doc §16.3: readiness is
    /// a notification, not a completion). Posted on the empty→nonempty queue
    /// transition and on closure, so a shard can block on one CQ wait for
    /// both kernel completions and endpoint work (§7, Phase 7).
    notify_cq: Option<crate::completion::CqId>,
    // Last field: retained subobjects drop before metadata admission returns.
    metadata_charge: budget::Charge,
}

#[derive(Debug)]
struct ReplyToken {
    server: AddressSpaceId,
    call: PendingCallId,
    /// Exclusive reply/cancellation owner; Drop cannot force-clear this claim.
    completing: bool,
    /// Cancellation reached uncertain physical cleanup; never deliver/reply it.
    cleanup_failed: bool,
    /// Borrowed minting authority protected by the in-flight reply claim.
    connection_source: Option<CapabilityId>,
    borrows: Vec<MemoryBorrow>,
    _charge: record_budget::Charge,
}

#[derive(Debug)]
struct PendingCall {
    caller: AddressSpaceId,
    result: Option<ReplyValue>,
    observers: WaiterList,
    /// Set once the caller has seen the result through `poll_reply`. From
    /// that point the returned connection/memory capabilities belong to the
    /// caller, and closing the pending-call cap no longer revokes them
    /// (state `ResultObserved` in the operation state machine).
    observed: bool,
    // Last: retained waiter/result metadata is dropped before admission returns.
    _charge: record_budget::Charge,
}

impl PendingCall {
    /// Prepare before attachment transfer or publishing any call capability.
    fn try_new(caller: AddressSpaceId, charge: record_budget::Charge) -> Result<Self, IpcError> {
        Ok(Self {
            caller,
            result: None,
            observers: ObserverList::try_new(waiter_budget::SOURCE_LIMIT)
                .map_err(|_| IpcError::ResourceLimit)?,
            observed: false,
            _charge: charge,
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

/// Own fresh connection authority and its sponsor before any attachment commits.
struct PreparedConnection {
    target: AddressSpaceId,
    endpoint: EndpointId,
    rights: ConnectionRights,
    authority: crate::capability::Reservation,
    charge: record_budget::Charge,
}

impl PreparedConnection {
    fn new(
        ipc: &mut IpcRegistry,
        sponsor: AddressSpaceId,
        target: AddressSpaceId,
        endpoint: EndpointId,
        rights: ConnectionRights,
    ) -> Result<Self, IpcError> {
        let charge = ipc.reserve_records(sponsor, [1, 0, 0])?;
        let authority = ipc.reserve_cap(target)?;
        Ok(Self {
            target,
            endpoint,
            rights,
            authority,
            charge,
        })
    }

    /// Only after joint authority publication under IPC serialization.
    fn install(self, ipc: &mut IpcRegistry) -> CapabilityId {
        ipc.as_caps(self.target).insert_admitted(
            self.authority.identity(),
            Capability::Connection {
                endpoint: self.endpoint,
                rights: self.rights,
            },
            Some(self.charge),
        )
    }
}

/// One owner for unpublished call authority, metadata and all attachments.
/// Field destruction cancels exact reservations and restores prepared sources;
/// it never operates on live receiver-controlled aliases.
struct PreparedCall {
    authority: crate::capability::Reservation,
    pending: PendingCall,
    reply_charge: record_budget::Charge,
    connection: Option<PreparedConnection>,
    transfers: Vec<crate::memory::object::PreparedTransfer>,
    memory: Vec<MemoryObjectCap>,
    borrows: Vec<MemoryBorrow>,
}

impl PreparedCall {
    fn attach(
        &mut self,
        transfer: crate::memory::object::PreparedTransfer,
    ) -> Result<(), IpcError> {
        self.transfers.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
        self.memory.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
        if let Some((owner, owner_cap)) = transfer.loan_origin() {
            self.borrows.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
            self.borrows.push(MemoryBorrow {
                owner,
                owner_cap,
                borrower: transfer.target(),
                borrower_cap: transfer.target_cap(),
            });
        }
        self.memory.push(transfer.target_cap());
        self.transfers.push(transfer);
        Ok(())
    }

    fn attach_vector(
        &mut self,
        transfers: Vec<crate::memory::object::PreparedTransfer>,
        memory: Vec<MemoryObjectCap>,
    ) -> Result<(), IpcError> {
        self.borrows.try_reserve_exact(transfers.len()).map_err(|_| IpcError::ResourceLimit)?;
        for transfer in &transfers {
            if let Some((owner, owner_cap)) = transfer.loan_origin() {
                self.borrows.push(MemoryBorrow {
                    owner,
                    owner_cap,
                    borrower: transfer.target(),
                    borrower_cap: transfer.target_cap(),
                });
            }
        }
        self.transfers = transfers;
        self.memory = memory;
        Ok(())
    }

    fn commit(
        mut self,
        ipc: &mut IpcRegistry,
        endpoint: EndpointId,
        opcode: u32,
        arg0: u64,
    ) -> Result<(CapabilityId, Delivery), IpcError> {
        let server = reserve_endpoint_queue(ipc, endpoint)?;
        let call_cap = self.authority.identity();
        if let Some(connection) = &mut self.connection {
            crate::memory::object::commit_transfers_with_authority(
                &mut self.transfers,
                &mut [&mut self.authority, &mut connection.authority],
            )
        } else {
            crate::memory::object::commit_transfers_with_authority(
                &mut self.transfers,
                &mut [&mut self.authority],
            )
        }
        .map_err(|_| IpcError::MemoryTransferFailed)?;
        // Pins must end before the newly published objects become writable.
        self.transfers.clear();
        let caller = self.pending.caller;
        let attached = self.connection.take().map(|connection| connection.install(ipc));
        let call = ipc.alloc_call();
        ipc.pending_calls.insert(call, self.pending);
        ipc.as_caps(caller).insert_admitted(
            call_cap,
            Capability::PendingCall {
                call,
            },
            None,
        );
        let token = ipc.alloc_reply();
        ipc.reply_tokens.insert(
            token,
            ReplyToken {
                server,
                call,
                completing: false,
                cleanup_failed: false,
                connection_source: None,
                borrows: self.borrows,
                _charge: self.reply_charge,
            },
        );
        let delivery = enqueue_message(
            ipc,
            endpoint,
            caller,
            opcode,
            arg0,
            Some(token),
            self.memory,
            attached,
        )
        .expect("prepared call retains endpoint queue ownership");
        Ok((call_cap, delivery))
    }
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
        self.caps.entry(asid).or_insert_with(|| AsIpcCaps::new(asid))
    }

    /// Prevent capability publication into a namespace being drained, including
    /// its short IPC-only retirement interval before the registry is removed.
    fn accepting_namespace(&mut self, sponsor: AddressSpaceId) -> Result<bool, IpcError> {
        let identity = crate::memory::current_address_space_handle(sponsor);
        let platform_identity = crate::memory::budget::platform_identity(sponsor);
        if identity.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
            return Err(IpcError::PermissionDenied);
        }
        let namespace = self.as_caps(sponsor);
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

    /// All call authority and fallible metadata precede attachment preparation.
    fn stage_call(&mut self, caller: AddressSpaceId) -> Result<PreparedCall, IpcError> {
        let mut charge = self.reserve_records(caller, [0, 1, 1])?;
        let reply_charge = charge.split([0, 0, 1]);
        let authority = self.reserve_cap(caller)?;
        Ok(PreparedCall {
            authority,
            pending: PendingCall::try_new(caller, charge)?,
            reply_charge,
            connection: None,
            transfers: Vec::new(),
            memory: Vec::new(),
            borrows: Vec::new(),
        })
    }

    fn cap(&self, asid: AddressSpaceId, cap: CapabilityId) -> Result<Capability, IpcError> {
        if !crate::capability::contains(asid, cap, crate::capability::ObjectKind::Ipc) {
            return Err(IpcError::UnknownCapability);
        }
        self.caps
            .get(&asid)
            .and_then(|caps| caps.caps.get(&cap))
            .map(|entry| entry.payload)
            .ok_or(IpcError::UnknownCapability)
    }

    fn remove_cap(
        &mut self,
        asid: AddressSpaceId,
        cap: CapabilityId,
    ) -> Result<Capability, IpcError> {
        let entry = self
            .caps
            .get_mut(&asid)
            .and_then(|caps| caps.caps.remove(&cap))
            .ok_or(IpcError::UnknownCapability)?;
        let revoked = crate::capability::remove(asid, cap, crate::capability::ObjectKind::Ipc);
        assert!(revoked, "IPC payload capability was absent from unified table");
        let removed = entry.payload;
        drop(entry);
        // Internal cancellation also revokes connections. It must reclaim a
        // closed endpoint just as public capability closure does.
        if let Capability::Connection {
            endpoint,
            ..
        } = removed
            && self.endpoints.get(&endpoint).is_some_and(|record| record.closed)
            && !endpoint_referenced(self, endpoint)
        {
            self.endpoints.remove(&endpoint);
        }
        Ok(removed)
    }

    fn remove_matching_caps(&mut self, asid: AddressSpaceId, target: Capability) {
        while let Some(id) = self.caps.get(&asid).and_then(|caps| {
            caps.caps.iter().find_map(|(&id, cap)| (cap.payload == target).then_some(id))
        }) {
            self.remove_cap(asid, id).expect("matching IPC capability disappeared");
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
    if capacity == 0 {
        return Err(IpcError::QueueFull);
    }
    let capacity = capacity.min(MAX_ENDPOINT_CAPACITY);
    let identity = crate::memory::current_address_space_handle(owner);
    let platform_identity = crate::memory::budget::platform_identity(owner);
    let mut ipc = IPC.write();
    if identity.is_some_and(|handle| !crate::memory::budget::accepting(handle)) {
        return Err(IpcError::PermissionDenied);
    }
    let namespace = ipc.as_caps(owner);
    if namespace.address_space != identity {
        return Err(IpcError::PermissionDenied);
    }
    // Synthetic namespaces exist only at the kernel test/adapter boundary;
    // syscalls always supply the authenticated live caller ASID.
    let platform = owner == crate::memory::KERNEL_ASID
        || platform_identity.is_some_and(|handle| Some(handle) == namespace.address_space);
    let metadata_charge = budget::reserve(&namespace.endpoint_budget, platform, [1, 0])
        .map_err(|_| IpcError::ResourceLimit)?;
    let queue = budget::AdmittedQueue::new(&namespace.endpoint_budget, platform, capacity)
        .map_err(|_| IpcError::ResourceLimit)?;
    let close_observers =
        ObserverList::try_new(crate::completion::watch_budget::MAX_ENDPOINT_WATCHES)
            .map_err(|_| IpcError::ResourceLimit)?;
    let readiness_observers =
        ObserverList::try_new(waiter_budget::SOURCE_LIMIT).map_err(|_| IpcError::ResourceLimit)?;
    let reservation = ipc.reserve_cap(owner)?;
    let cap = reservation.publish().map_err(cap_admission_error)?;
    // Everything after publication is infallible under this IPC write guard.
    let endpoint = ipc.alloc_endpoint();
    ipc.endpoints.insert(
        endpoint,
        Endpoint {
            owner,
            interface,
            version,
            capacity,
            high_water: 0,
            queue,
            metadata_charge,
            readiness_observers,
            close_observers,
            closed: false,
            notify_cq: None,
        },
    );
    Ok(ipc.as_caps(owner).insert_admitted(
        cap,
        Capability::Endpoint {
            endpoint,
            rights: ConnectionRights::ALL,
        },
        None,
    ))
}

/// Binds an endpoint's readiness to one of the owner's completion queues.
///
/// Binding also posts a wake when the endpoint is already readable. After
/// binding, the kernel posts a coalesced wake whenever the endpoint's message
/// queue transitions from empty to nonempty, and when the endpoint closes.
/// This lets a shard block on a single CQ wait for both
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
    let already_readable = endpoint.closed || !endpoint.queue.is_empty();
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
    if new_capacity == 0 {
        return Err(IpcError::QueueFull);
    }
    let capacity = new_capacity.min(MAX_ENDPOINT_CAPACITY);
    let identity = crate::memory::current_address_space_handle(owner);
    let mut ipc = IPC.write();
    if identity.is_some_and(|handle| !crate::memory::budget::accepting(handle))
        || ipc.caps.get(&owner).is_some_and(|caps| caps.address_space != identity)
    {
        return Err(IpcError::PermissionDenied);
    }
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
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    if capacity > endpoint.queue.capacity() {
        // Charge both backing allocations during growth. A failed admission
        // or allocation leaves the policy and queued messages unchanged.
        let mut grown = budget::AdmittedQueue::new(
            endpoint.metadata_charge.domain(),
            endpoint.metadata_charge.platform(),
            capacity,
        )
        .map_err(|_| IpcError::ResourceLimit)?;
        grown.extend(endpoint.queue.drain(..));
        endpoint.queue = grown;
    }
    endpoint.capacity = capacity;
    Ok(capacity)
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
    let mut ipc = IPC.write();
    let (endpoint, granted) = mintable_endpoint(&ipc, owner, endpoint_cap, rights)?;
    ipc.accepting_namespace(target)?;
    let charge = ipc.reserve_records(owner, [1, 0, 0])?;
    let reservation = ipc.reserve_cap(target)?;
    let cap = reservation.publish().map_err(cap_admission_error)?;
    Ok(ipc.as_caps(target).insert_admitted(
        cap,
        Capability::Connection {
            endpoint,
            rights: granted,
        },
        Some(charge),
    ))
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

    reserve_endpoint_queue(&ipc, endpoint_id)?;
    let prepared = ipc.stage_call(caller)?;
    let (call_cap, delivery) = prepared.commit(&mut ipc, endpoint_id, opcode, arg0)?;
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
    let (delegated_endpoint, granted) = if delegate_cap == 0 {
        (0, ConnectionRights(0))
    } else {
        mintable_endpoint(&ipc, caller, delegate_cap, delegate_rights)?
    };

    let server = reserve_endpoint_queue(&ipc, endpoint_id)?;
    let mut prepared = ipc.stage_call(caller)?;
    prepared.connection =
        Some(PreparedConnection::new(&mut ipc, caller, server, delegated_endpoint, granted)?);
    if let Some(memory_cap) = copied_memory {
        prepared.attach(
            crate::memory::object::prepare_copy(caller, memory_cap, server)
                .map_err(|_| IpcError::MemoryTransferFailed)?,
        )?;
    }
    let (call_cap, delivery) = prepared.commit(&mut ipc, endpoint_id, opcode, arg0)?;
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
    let mut prepared = ipc.stage_call(caller)?;
    prepared.attach(
        crate::memory::object::prepare_move(caller, memory_cap, server)
            .map_err(|_| IpcError::MemoryTransferFailed)?,
    )?;
    let (call_cap, delivery) = prepared.commit(&mut ipc, endpoint_id, opcode, arg0)?;
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
    let mut prepared = ipc.stage_call(caller)?;
    prepared.attach(
        crate::memory::object::prepare_copy(caller, memory_cap, server)
            .map_err(|_| IpcError::MemoryTransferFailed)?,
    )?;
    let (call_cap, delivery) = prepared.commit(&mut ipc, endpoint_id, opcode, arg0)?;
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
    let mut prepared = ipc.stage_call(caller)?;
    prepared.attach(
        crate::memory::object::prepare_loan(caller, memory_cap, server, writable)
            .map_err(|_| IpcError::MemoryTransferFailed)?,
    )?;
    let (call_cap, delivery) = prepared.commit(&mut ipc, endpoint_id, opcode, arg0)?;
    drop(ipc);
    deliver(delivery);
    Ok(call_cap)
}

/// What must be signalled after an enqueue, once the IPC lock is dropped:
/// blocked endpoint receivers, plus (for CQ-bound endpoints) a coalesced
/// readiness wake on the owner's completion queue.
struct Delivery {
    observers: WaitNotifications,
    cq_wake: Option<(AddressSpaceId, crate::completion::CqId)>,
}

fn deliver(delivery: Delivery) {
    if let Some((asid, cq)) = delivery.cq_wake {
        crate::completion::wake(asid, cq);
    }
    delivery.observers.notify();
}

fn enqueue_scalar(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<ReplyTokenId>,
) -> Result<Delivery, IpcError> {
    enqueue_message(ipc, endpoint_id, sender, opcode, arg0, reply, Vec::new(), None)
}

fn enqueue_scalar_with_memory(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<ReplyTokenId>,
    memory: Option<MemoryObjectCap>,
) -> Result<Delivery, IpcError> {
    let memory_vec: Vec<MemoryObjectCap> = memory.into_iter().collect();
    enqueue_message(ipc, endpoint_id, sender, opcode, arg0, reply, memory_vec, None)
}

#[allow(clippy::too_many_arguments)]
fn enqueue_message(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<ReplyTokenId>,
    memory: Vec<MemoryObjectCap>,
    connection: Option<CapabilityId>,
) -> Result<Delivery, IpcError> {
    let (sender_generation, sender_principal, sender_roles) =
        if sender == crate::memory::KERNEL_ASID {
            (
                1,
                1,
                catten_syscall::domain_roles::POLICY_ADMIN
                    | catten_syscall::domain_roles::SERVICE_MANAGER,
            )
        } else if let Some(authority) = crate::memory::domain_authority(sender) {
            (authority.address_space.generation() as u64, authority.principal, authority.roles)
        } else {
            let generation = crate::memory::current_address_space_handle(sender)
                .map_or(0, |handle| handle.generation() as u64);
            (generation, 0, 0)
        };
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
        sender_generation,
        sender_principal,
        sender_roles,
        opcode,
        arg0,
        reply,
        memory,
        connection,
    });
    endpoint.high_water = endpoint.high_water.max(endpoint.queue.len());
    // Coalesced readiness (§9.4): only the empty→nonempty transition posts a
    // CQ wake; further messages are observed when the receiver drains.
    let cq_wake = if was_empty {
        endpoint.notify_cq.map(|cq| (endpoint.owner, cq))
    } else {
        None
    };
    Ok(Delivery {
        observers: endpoint.readiness_observers.drain(),
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
    ipc.accepting_namespace(receiver)?;
    Ok(PreparedReceive::new(&mut ipc, receiver, endpoint_id)?.commit())
}

/// Own speculative reply authority until dequeue commits. IPC stays exclusively
/// borrowed, so guessed handles cannot use that authority before delivery. A
/// result-page failure drops only this capability, not the queued call/token or
/// its loans. No reverse cleanup of delivered attachments is necessary.
#[must_use]
struct PreparedReceive<'a> {
    ipc: &'a mut IpcRegistry,
    receiver: AddressSpaceId,
    endpoint: EndpointId,
    reply: Option<CapabilityId>,
}

impl<'a> PreparedReceive<'a> {
    fn new(
        ipc: &'a mut IpcRegistry,
        receiver: AddressSpaceId,
        endpoint: EndpointId,
    ) -> Result<Self, IpcError> {
        let token = ipc
            .endpoints
            .get(&endpoint)
            .ok_or(IpcError::UnknownCapability)?
            .queue
            .front()
            .ok_or(IpcError::NoMessage)?
            .reply;
        let reply = if let Some(token) = token {
            let reply = ipc.reply_tokens.get(&token).ok_or(IpcError::UnknownCapability)?;
            if reply.server != receiver {
                return Err(IpcError::PermissionDenied);
            }
            if reply.completing || reply.cleanup_failed {
                return Err(IpcError::Pending);
            }
            let reservation = ipc.reserve_cap(receiver)?;
            let cap = reservation.publish().map_err(cap_admission_error)?;
            Some(ipc.as_caps(receiver).insert_admitted(
                cap,
                Capability::ReplyToken {
                    token,
                },
                None,
            ))
        } else {
            None
        };
        Ok(Self {
            ipc,
            receiver,
            endpoint,
            reply,
        })
    }

    fn write_result(&self, result_page: MemoryObjectCap) -> Result<(), IpcError> {
        let queued = self.ipc.endpoints.get(&self.endpoint).unwrap().queue.front().unwrap();
        let count = queued.memory.len().min(CAP_VECTOR_MAX);
        // Bounded stack storage: no allocation after reply admission.
        let mut result = [0u8; 2 + CAP_VECTOR_MAX * core::mem::size_of::<u64>()];
        result[..2].copy_from_slice(&(count as u16).to_le_bytes());
        for (index, cap) in queued.memory.iter().take(count).enumerate() {
            let offset = 2 + index * core::mem::size_of::<u64>();
            result[offset..offset + 8].copy_from_slice(&cap.to_le_bytes());
        }
        crate::memory::object::write_bytes(self.receiver, result_page, &result[..2 + count * 8])
            .map_err(|_| IpcError::MemoryTransferFailed)
    }

    fn commit(mut self) -> ScalarMessage {
        let endpoint = self.ipc.endpoints.get_mut(&self.endpoint).unwrap();
        let message = endpoint.queue.pop_front().expect("prepared receive retains queue ownership");
        ScalarMessage {
            sender: message.sender,
            sender_generation: message.sender_generation,
            sender_principal: message.sender_principal,
            sender_roles: message.sender_roles,
            interface: endpoint.interface,
            version: endpoint.version,
            opcode: message.opcode,
            arg0: message.arg0,
            reply: self.reply.take(),
            // Scalar receive exposes the first attachment; receive_vec also
            // writes the complete memory-capability vector into its result page.
            memory: message.memory.first().copied(),
            connection: message.connection,
        }
    }
}

impl Drop for PreparedReceive<'_> {
    fn drop(&mut self) {
        if let Some(cap) = self.reply.take() {
            self.ipc.remove_cap(self.receiver, cap).expect("prepared reply authority disappeared");
        }
    }
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
    let generation = loop {
        if endpoint_is_readable_or_closed(endpoint_id)? {
            return Ok(());
        }
        let registration = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .block_thread_with_constraint_generation(
                tid,
                &observable,
                crate::cpu::scheduler::threads::MigrationConstraint::EndpointWait,
            );
        match registration {
            Ok(generation) => break generation,
            Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed) => {
                // Preserve the untimed receive contract under pressure without
                // parking after a rejected registration or killing the service.
                IPC_WAIT_ADMISSION_RETRIES[0].fetch_add(1, Ordering::Relaxed);
                crate::cpu::scheduler::yield_lp();
            }
            Err(_) => return Err(IpcError::NoMessage),
        }
    };

    // Lost-wake guard: if a sender enqueued after the fast-path check but
    // before observer registration completed, re-admit the thread immediately.
    let readable = match endpoint_is_readable_or_closed(endpoint_id) {
        Ok(readable) => readable,
        Err(error) => {
            // The endpoint was removed while this thread was parked; re-admit
            // it before returning so it is not left Blocked forever.
            let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                .read()
                .submit_woken_thread(tid, generation);
            return Err(error);
        }
    };
    if readable {
        let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .submit_woken_thread(tid, generation);
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
    Ok(endpoint_available(&ipc, endpoint))
}

fn endpoint_available(ipc: &IpcRegistry, endpoint: &Endpoint) -> bool {
    endpoint.closed
        || endpoint.queue.front().is_some_and(|message| {
            message.reply.is_none_or(|token| {
                ipc.reply_tokens
                    .get(&token)
                    .is_some_and(|token| !token.completing && !token.cleanup_failed)
            })
        })
}

struct EndpointObservable {
    endpoint: EndpointId,
}

struct EndpointCloseCompletionObserver {
    asid: AddressSpaceId,
    cap: crate::completion::CompletionCap,
    completion: Weak<crate::completion::Completion>,
}

impl Observer for EndpointCloseCompletionObserver {
    fn notify(self: Arc<Self>) {
        if let Some(completion) = self.completion.upgrade() {
            let _ = crate::completion::complete_registered(
                self.asid,
                self.cap,
                completion,
                crate::completion::OpResult::Ok(REPLY_ENDPOINT_CLOSED),
            );
        }
    }
}

/// Return a completion capability that becomes ready when the endpoint named
/// by `connection_cap` closes. Registration and the closed-state check share
/// the IPC lock, closing the lost-wake race between those operations.
pub fn watch_connection_closed(
    asid: AddressSpaceId,
    connection_cap: CapabilityId,
) -> Result<crate::completion::CompletionCap, IpcError> {
    let endpoint_id = {
        let ipc = IPC.read();
        match ipc.cap(asid, connection_cap)? {
            Capability::Connection {
                endpoint,
                ..
            } => endpoint,
            _ => return Err(IpcError::WrongType),
        }
    };
    let mut submission =
        crate::completion::EventSubmission::new(asid).map_err(|_| IpcError::QueueFull)?;
    let cap = submission.cap();
    let completion = submission.completion().clone();
    let observer: Arc<dyn Observer> = Arc::try_new(EndpointCloseCompletionObserver {
        asid,
        cap,
        completion: Arc::downgrade(&completion),
    })
    .map_err(|_| IpcError::ResourceLimit)?;

    let registration = {
        let ipc = IPC.read();
        // Revalidate after staging; a revoked/reused connection must not
        // authorize registration against the previously resolved endpoint.
        if !matches!(ipc.cap(asid, connection_cap)?, Capability::Connection { endpoint, .. } if endpoint == endpoint_id)
        {
            return Err(IpcError::UnknownCapability);
        }
        let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
        if endpoint.closed {
            None
        } else {
            Some(
                endpoint
                    .close_observers
                    .register(Arc::downgrade(&observer), submission.take_charge())
                    .map_err(|_| IpcError::ResourceLimit)?,
            )
        }
    };
    if let Some(registration) = registration {
        if submission
            .install_watch_observation(observer, registration)
            .map_err(|_| IpcError::UnknownCapability)?
        {
            let _ = crate::completion::complete_registered(
                asid,
                cap,
                completion,
                crate::completion::OpResult::Cancelled,
            );
        }
    } else {
        observer.notify();
    }
    Ok(submission.commit())
}

impl Observable for EndpointObservable {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        let ipc = IPC.read();
        let endpoint = ipc.endpoints.get(&self.endpoint).ok_or(RegistrationError::Closed)?;
        if endpoint_available(&ipc, endpoint) {
            return Ok(WaitRegistration::ready());
        }
        sponsor.register(&endpoint.readiness_observers, observer)
    }
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
    let observers = complete_reply(&mut ipc, server, reply_cap, result, None, None)?;
    drop(ipc);
    signal_observers(observers);
    Ok(())
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
    let observers =
        complete_reply(&mut ipc, server, reply_cap, result, Some((endpoint, granted)), None)?;
    drop(ipc);
    signal_observers(observers);
    Ok(())
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
    let observers = complete_reply(&mut ipc, server, reply_cap, result, None, Some(memory_cap))?;
    drop(ipc);
    signal_observers(observers);
    Ok(())
}

fn complete_reply(
    ipc: &mut IpcRegistry,
    server: AddressSpaceId,
    reply_cap: CapabilityId,
    result: i64,
    returned_connection: Option<(EndpointId, ConnectionRights)>,
    returned_memory: Option<MemoryObjectCap>,
) -> Result<WaitNotifications, IpcError> {
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
    if token.completing {
        return Err(IpcError::ReplyAlreadyUsed);
    }
    if token.cleanup_failed {
        return Err(IpcError::MemoryTransferFailed);
    }
    let call_id = token.call;
    let caller = ipc.pending_calls.get(&call_id).ok_or(IpcError::UnknownCapability)?.caller;
    // A reply is solicited by this caller. Charge returned authority to that
    // requester, so repeated lookups cannot spend the serving grantor's budget.
    // Reject before consuming the reply token, revoking a loan, or moving memory.
    let mut connection = returned_connection
        .map(|(endpoint, rights)| PreparedConnection::new(ipc, caller, caller, endpoint, rights))
        .transpose()?;
    let mut returned_move = if let Some(memory_cap) = returned_memory {
        Some(
            crate::memory::object::prepare_move(server, memory_cap, caller)
                .map_err(|_| IpcError::MemoryTransferFailed)?,
        )
    } else {
        None
    };
    // Record each successful revocation immediately. A later unmap failure or
    // retired destination leaves only the still-live loans for retry/cancel.
    while let Some(borrow) = ipc.reply_tokens.get(&token_id).unwrap().borrows.last().copied() {
        revoke_memory_borrow(borrow).map_err(|_| IpcError::MemoryTransferFailed)?;
        ipc.reply_tokens.get_mut(&token_id).unwrap().borrows.pop();
    }
    let returned_memory_cap = returned_move.as_ref().map(|transfer| transfer.target_cap());
    if let Some(connection) = &mut connection {
        crate::memory::object::commit_transfers_with_authority(
            returned_move.as_mut_slice(),
            &mut [&mut connection.authority],
        )
    } else {
        crate::memory::object::commit_transfers(returned_move.as_mut_slice())
    }
    .map_err(|_| IpcError::MemoryTransferFailed)?;
    drop(returned_move);
    let returned_cap = connection.map(|connection| connection.install(ipc));
    ipc.reply_tokens.remove(&token_id);
    let call = ipc.pending_calls.get_mut(&call_id).ok_or(IpcError::UnknownCapability)?;
    call.result = Some(ReplyValue {
        result,
        cap: returned_cap,
        memory: returned_memory_cap,
    });
    let observers = call.observers.close();
    let _ = ipc.remove_cap(server, reply_cap);
    Ok(observers)
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
    if result.is_some() {
        call.observed = true;
    }
    Ok(result)
}

pub fn wait_reply(caller: AddressSpaceId, call_cap: CapabilityId) -> Result<(), IpcError> {
    let call_id = pending_call_id(caller, call_cap)?;
    let observable = PendingCallObservable {
        call: call_id,
    };
    loop {
        if pending_call_is_ready(call_id)? {
            return Ok(());
        }

        let tid = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .get_lp_scheduler()
            .lock()
            .get_tid()
            .ok_or(IpcError::NoMessage)?;
        let registration = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .block_thread_with_constraint_generation(
                tid,
                &observable,
                crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
            );
        let generation = match registration {
            Ok(generation) => generation,
            Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed) => {
                // A borrowed PendingCall must retain its borrow until reply
                // or cancellation revokes the server's access. Do not let
                // admission pressure masquerade as a terminal wait result.
                IPC_WAIT_ADMISSION_RETRIES[1].fetch_add(1, Ordering::Relaxed);
                crate::cpu::scheduler::yield_lp();
                continue;
            }
            Err(_) => return Err(IpcError::NoMessage),
        };

        // Close the check/register race. Notifications are hints rather than
        // proof that this particular pending call completed, so after every
        // wake the loop checks the call and parks again when necessary.
        let ready = match pending_call_is_ready(call_id) {
            Ok(ready) => ready,
            Err(error) => {
                // The call was removed while this thread was parked. Its
                // observers may have been dropped without a wake, so re-admit
                // the thread before returning rather than leaving it Blocked.
                let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                    .read()
                    .submit_woken_thread(tid, generation);
                return Err(error);
            }
        };
        if ready {
            let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                .read()
                .submit_woken_thread(tid, generation);
        }
        crate::cpu::scheduler::yield_lp();
    }
}

/// Block the calling thread until the pending call's reply arrives or
/// `timeout_ms` elapses. Returns `true` if the reply is ready, `false` on
/// timeout or failed waiter admission. Like [`wait_reply`], this parks the thread on the
/// pending-call observable (event-driven) with a timer watchdog, so a caller that must
/// fail loudly on a hang can assert on the result instead of busy-polling
/// with `yield_lp`.
pub fn wait_reply_timeout(
    caller: AddressSpaceId,
    call_cap: CapabilityId,
    timeout_ms: u64,
) -> Result<bool, IpcError> {
    let call_id = pending_call_id(caller, call_cap)?;
    if pending_call_is_ready(call_id)? {
        return Ok(true);
    }
    let observable = PendingCallObservable {
        call: call_id,
    };
    Ok(crate::cpu::scheduler::block_until(&observable, timeout_ms, || {
        pending_call_is_ready(call_id).unwrap_or(false)
    }))
}

fn pending_call_id(
    caller: AddressSpaceId,
    call_cap: CapabilityId,
) -> Result<PendingCallId, IpcError> {
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
    Ok(call_id)
}

fn pending_call_is_ready(call_id: PendingCallId) -> Result<bool, IpcError> {
    let ipc = IPC.read();
    Ok(ipc.pending_calls.get(&call_id).ok_or(IpcError::UnknownCapability)?.result.is_some())
}

struct PendingCallObservable {
    call: PendingCallId,
}

impl Observable for PendingCallObservable {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        let ipc = IPC.read();
        let call = ipc.pending_calls.get(&self.call).ok_or(RegistrationError::Closed)?;
        if call.result.is_some() {
            return Ok(WaitRegistration::ready());
        }
        sponsor.register(&call.observers, observer)
    }
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

pub fn close_cap(asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
    close_cap_with_wait(asid, cap, crate::cpu::scheduler::yield_lp)
}

fn close_cap_with_wait(
    asid: AddressSpaceId,
    cap: CapabilityId,
    wait: impl FnMut(),
) -> Result<(), IpcError> {
    close_cap_in_mode(asid, cap, true, wait)
}

/// Root cleanup retains lifecycle and has already drained operation leases.
/// It must never acquire another lifecycle lease underneath that guard.
fn close_cap_serialized(asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
    close_cap_in_mode(asid, cap, false, crate::cpu::scheduler::yield_lp)
}

fn close_cap_in_mode(
    asid: AddressSpaceId,
    cap: CapabilityId,
    detached: bool,
    wait: impl FnMut(),
) -> Result<(), IpcError> {
    close_cap_in_mode_with_revoker(asid, cap, detached, wait, revoke_memory_borrow)
}

// The boot fixture supplies an actual prepared-loan failure. Production uses
// only the serialized revocation adapter, never arbitrary callback work.
fn close_cap_in_mode_with_revoker(
    asid: AddressSpaceId,
    cap: CapabilityId,
    detached: bool,
    mut wait: impl FnMut(),
    mut revoke: impl FnMut(MemoryBorrow) -> Result<(), crate::memory::object::MemoryObjectError>,
) -> Result<(), IpcError> {
    // Keep the caller's borrow alive until a claimed reply has finished its
    // detached cleanup. Never wait with IPC serialization held. Domain cleanup
    // reaches here only after the reply's exact-generation root leases drain.
    let mut ipc = loop {
        let ipc = IPC.write();
        let busy = match ipc.cap(asid, cap)? {
            Capability::PendingCall {
                call,
            } => ipc.reply_tokens.values().any(|token| token.call == call && token.completing),
            Capability::ReplyToken {
                token,
            } => ipc.reply_tokens.get(&token).is_some_and(|token| token.completing),
            Capability::Endpoint {
                endpoint,
                ..
            } => {
                ipc.reply_tokens.values().any(|token| {
                    token.completing && token.server == asid && token.connection_source == Some(cap)
                }) || ipc.endpoints.get(&endpoint).is_some_and(|endpoint| {
                    endpoint.queue.iter().any(|message| {
                        message.reply.is_some_and(|token| {
                            ipc.reply_tokens.get(&token).is_some_and(|token| token.completing)
                        })
                    })
                })
            }
            Capability::Connection {
                ..
            } => ipc.reply_tokens.values().any(|token| {
                token.completing && token.server == asid && token.connection_source == Some(cap)
            }),
        };
        if !busy {
            if detached && cancellation::has_loans(&ipc, asid, cap)? {
                drop(ipc);
                // Revalidate identity and claim after admitting both roots.
                // Another close/reply can win this preparation interval.
                match cancellation::close(asid, cap) {
                    Err(IpcError::Pending) => {
                        wait();
                        continue;
                    }
                    result => return result,
                }
            }
            break ipc;
        }
        drop(ipc);
        wait();
    };
    // Preserve the close capability, queue and pending record until every loan
    // has confirmed cleanup. Partial success is recorded in place; a failed
    // token stays fenced and cannot be delivered or replied to again.
    prepare_serialized_close(&mut ipc, asid, cap, &mut revoke)?;
    let mut observers = WaitNotifications::empty();
    let mut cq_wake = None;
    let mut close_watches = None;
    match ipc.remove_cap(asid, cap)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => {
            let mut queued = if let Some(endpoint) = ipc.endpoints.get_mut(&endpoint) {
                if endpoint.owner != asid {
                    budget::AdmittedQueue::default()
                } else {
                    endpoint.closed = true;
                    observers.append(endpoint.readiness_observers.close());
                    close_watches = Some(endpoint.close_observers.close());
                    // A CQ-bound endpoint reports its closure as a readiness
                    // wake so a reactor blocked on one CQ wait observes it.
                    cq_wake = endpoint.notify_cq.map(|cq| (endpoint.owner, cq));
                    core::mem::take(&mut endpoint.queue)
                }
            } else {
                budget::AdmittedQueue::default()
            };
            for message in queued.drain(..) {
                if let Some(token) = message.reply {
                    consume_reply_token(&mut ipc, token, REPLY_ENDPOINT_CLOSED, &mut observers)
                        .expect("preflighted endpoint loan cleanup changed under IPC");
                }
                for memory_cap in &message.memory {
                    let _ = crate::memory::object::try_close_cap(asid, *memory_cap);
                }
                if let Some(connection_cap) = message.connection {
                    let _ = ipc.remove_cap(asid, connection_cap);
                }
            }
            // Retire the registry entry once no capability can name it, so
            // endpoint create/close cycles cannot grow the registry forever.
            if !endpoint_referenced(&ipc, endpoint) {
                ipc.endpoints.remove(&endpoint);
            }
        }
        Capability::PendingCall {
            call,
        } => {
            if let Some(pending) = ipc.pending_calls.remove(&call) {
                // A thread parked in wait_reply holds its only waker here.
                observers.append(pending.observers.close());
                if let Some(reply) = pending.result {
                    // Revoke undelivered results only. Once the caller has
                    // observed the reply, the returned capabilities are its
                    // property and survive the pending-call close.
                    if !pending.observed {
                        if let Some(returned_cap) = reply.cap {
                            let _ = ipc.remove_cap(asid, returned_cap);
                        }
                        if let Some(memory_cap) = reply.memory {
                            let _ = crate::memory::object::try_close_cap(asid, memory_cap);
                        }
                    }
                } else {
                    cancel_queued_call(&mut ipc, call, &mut observers, &mut cq_wake);
                }
            }
        }
        Capability::ReplyToken {
            token,
        } => {
            consume_reply_token(&mut ipc, token, REPLY_CANCELLED, &mut observers)
                .expect("preflighted reply loan cleanup changed under IPC");
        }
        Capability::Connection {
            endpoint,
            ..
        } => {
            // The last capability naming a closed endpoint retires it.
            if !endpoint_referenced(&ipc, endpoint) {
                ipc.endpoints.remove(&endpoint);
            }
        }
    }
    drop(ipc);
    if let Some(watches) = close_watches {
        watches.notify();
    }
    if let Some((owner, cq)) = cq_wake {
        crate::completion::wake(owner, cq);
    }
    signal_observers(observers);
    Ok(())
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

/// Serialized root cleanup may retire admission and confirm some loans before
/// rejection. It returns no successful namespace retirement on uncertain cleanup;
/// the caller must retain its exact root and closing fence on error.
pub fn close_address_space(asid: AddressSpaceId) -> Result<(), IpcError> {
    {
        let mut ipc = IPC.write();
        assert!(
            !ipc.reply_tokens.values().any(|token| token.completing
                && (token.server == asid
                    || ipc.pending_calls.get(&token.call).is_some_and(|call| call.caller == asid))),
            "IPC namespace cleanup before completion leases drained"
        );
        if let Some(namespace) = ipc.caps.get(&asid) {
            namespace.record_budget.retire();
        }
        // Preflight every token involving this namespace, including delivered
        // replies and foreign callers. Do not remove any capability first:
        // root cleanup must not lose the receipt for an uncertain loan.
        let mut cursor = 0;
        loop {
            let next = ipc
                .reply_tokens
                .range((core::ops::Bound::Excluded(cursor), core::ops::Bound::Unbounded))
                .find_map(|(&id, token)| {
                    (token.server == asid
                        || ipc
                            .pending_calls
                            .get(&token.call)
                            .is_some_and(|call| call.caller == asid))
                    .then_some(id)
                });
            let Some(token) = next else {
                break;
            };
            revoke_serialized_token(&mut ipc, token, &mut revoke_memory_borrow)?;
            cursor = token;
        }
    }
    // Use admitted registry storage as the work list, not an infallibly
    // allocated teardown snapshot. Closing one call may also remove a reply cap.
    loop {
        let next = IPC.read().caps.get(&asid).and_then(|caps| caps.caps.keys().next().copied());
        let Some(cap) = next else {
            break;
        };
        close_cap_serialized(asid, cap)?;
    }
    IPC.write().caps.remove(&asid);
    let mut ipc = IPC.write();
    while let Some(endpoint) = ipc.endpoints.iter().find_map(|(&id, endpoint)| {
        (endpoint.closed && !endpoint_referenced(&ipc, id)).then_some(id)
    }) {
        ipc.endpoints.remove(&endpoint);
    }
    Ok(())
}

fn revoke_serialized_token(
    ipc: &mut IpcRegistry,
    token: ReplyTokenId,
    revoke: &mut impl FnMut(MemoryBorrow) -> Result<(), crate::memory::object::MemoryObjectError>,
) -> Result<(), IpcError> {
    if let Some(reply) = ipc.reply_tokens.get(&token) {
        if reply.completing {
            return Err(IpcError::Pending);
        }
        if reply.cleanup_failed {
            return Err(IpcError::MemoryTransferFailed);
        }
    }
    while let Some(borrow) =
        ipc.reply_tokens.get(&token).and_then(|token| token.borrows.last().copied())
    {
        if revoke(borrow).is_err() {
            ipc.reply_tokens.get_mut(&token).unwrap().cleanup_failed = true;
            crate::logln!(
                "[IPC] bulk cancellation retained token {} after uncertain loan cleanup",
                token
            );
            return Err(IpcError::MemoryTransferFailed);
        }
        assert_eq!(ipc.reply_tokens.get_mut(&token).unwrap().borrows.pop(), Some(borrow));
    }
    Ok(())
}

fn prepare_serialized_close(
    ipc: &mut IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
    revoke: &mut impl FnMut(MemoryBorrow) -> Result<(), crate::memory::object::MemoryObjectError>,
) -> Result<(), IpcError> {
    match ipc.cap(asid, cap)? {
        Capability::Endpoint {
            endpoint,
            ..
        } => {
            let mut index = 0;
            loop {
                let message = ipc
                    .endpoints
                    .get(&endpoint)
                    .filter(|endpoint| endpoint.owner == asid)
                    .and_then(|endpoint| endpoint.queue.get(index));
                let Some(message) = message else {
                    break;
                };
                if let Some(token) = message.reply {
                    revoke_serialized_token(ipc, token, revoke)?;
                }
                index += 1;
            }
        }
        Capability::PendingCall {
            call,
        } => {
            // Keep the token in the registry until its loans are confirmed.
            let mut cursor = 0;
            loop {
                let next = ipc
                    .reply_tokens
                    .range((core::ops::Bound::Excluded(cursor), core::ops::Bound::Unbounded))
                    .find_map(|(&id, token)| (token.call == call).then_some(id));
                let Some(token) = next else {
                    break;
                };
                revoke_serialized_token(ipc, token, revoke)?;
                cursor = token;
            }
        }
        Capability::ReplyToken {
            token,
        } => revoke_serialized_token(ipc, token, revoke)?,
        Capability::Connection {
            ..
        } => {}
    }
    Ok(())
}

fn consume_reply_token(
    ipc: &mut IpcRegistry,
    token: ReplyTokenId,
    result: i64,
    observers: &mut WaitNotifications,
) -> Result<(), IpcError> {
    revoke_serialized_token(ipc, token, &mut revoke_memory_borrow)?;
    if let Some(token) = ipc.reply_tokens.remove(&token) {
        assert!(
            !token.completing && token.borrows.is_empty(),
            "consuming an unfinished completion"
        );
        if let Some(call) = ipc.pending_calls.get_mut(&token.call) {
            call.result = Some(ReplyValue {
                result,
                cap: None,
                memory: None,
            });
            observers.append(call.observers.close());
        }
    }
    Ok(())
}

fn cancel_queued_call(
    ipc: &mut IpcRegistry,
    call: PendingCallId,
    observers: &mut WaitNotifications,
    cq_wake: &mut Option<(AddressSpaceId, crate::completion::CqId)>,
) {
    while let Some((token, server)) = ipc
        .reply_tokens
        .iter()
        .find_map(|(&id, token)| (token.call == call).then_some((id, token.server)))
    {
        let retired = ipc.reply_tokens.remove(&token).unwrap();
        assert!(
            !retired.completing && !retired.cleanup_failed && retired.borrows.is_empty(),
            "cancelling an unfinished reply"
        );
        cancel_queued_message_with_token(ipc, server, token, observers, cq_wake);
        ipc.remove_matching_caps(
            server,
            Capability::ReplyToken {
                token,
            },
        );
    }
}

fn cancel_queued_message_with_token(
    ipc: &mut IpcRegistry,
    server: AddressSpaceId,
    token: ReplyTokenId,
    observers: &mut WaitNotifications,
    cq_wake: &mut Option<(AddressSpaceId, crate::completion::CqId)>,
) {
    for endpoint in ipc.endpoints.values_mut() {
        if endpoint.owner != server {
            continue;
        }
        if let Some(index) = endpoint.queue.iter().position(|message| message.reply == Some(token))
        {
            observers.append(endpoint.readiness_observers.drain());
            *cq_wake = endpoint.notify_cq.map(|cq| (endpoint.owner, cq));
            if let Some(message) = endpoint.queue.remove(index) {
                for memory_cap in &message.memory {
                    // Confirmed loans are already gone; copies/moves are still
                    // queued and must be released. No uncertain loan reaches here.
                    let _ = crate::memory::object::try_close_cap(server, *memory_cap);
                }
                if let Some(connection_cap) = message.connection {
                    ipc.remove_cap(server, connection_cap)
                        .expect("queued connection disappeared during cancellation");
                }
            }
            return;
        }
    }
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

/// Receive a message like [`receive`], but also fill a result page with
/// the cap IDs of all delivered memory objects (up to [`CAP_VECTOR_MAX`]).
pub fn receive_vec(
    receiver: AddressSpaceId,
    endpoint_cap: CapabilityId,
    result_page: MemoryObjectCap,
) -> Result<ScalarMessage, IpcError> {
    let mut ipc = IPC.write();
    let endpoint_id = receive_endpoint_id(&ipc, receiver, endpoint_cap)?;
    ipc.accepting_namespace(receiver)?;
    let prepared = PreparedReceive::new(&mut ipc, receiver, endpoint_id)?;
    // Rights/capacity failure returns speculative reply admission and leaves
    // the queued call and all attachments available for retry/cancellation.
    prepared.write_result(result_page)?;
    Ok(prepared.commit())
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
    let mut ipc = IPC.write();
    let (endpoint_id, rights) = match ipc.cap(sender, connection)? {
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

    let mut memory_caps = Vec::new();
    let mut transfers = read_vector_page(sender, cap_vector, server, false, &mut memory_caps)?;
    crate::memory::object::commit_transfers(&mut transfers)
        .map_err(|_| IpcError::MemoryTransferFailed)?;
    drop(transfers);
    // Queue availability was validated under this same IPC write guard; none
    // of attachment preparation/publication changes the endpoint or its queue.
    let delivery =
        enqueue_message(&mut ipc, endpoint_id, sender, opcode, arg0, None, memory_caps, None)
            .expect("reserved vector-send endpoint changed under IPC ownership");
    drop(ipc);
    deliver(delivery);

    let _ = crate::memory::object::close_cap(sender, cap_vector);
    Ok(())
}

pub fn vector_call(
    caller: AddressSpaceId,
    connection: CapabilityId,
    opcode: u32,
    arg0: u64,
    cap_vector: MemoryObjectCap,
) -> Result<CapabilityId, IpcError> {
    let mut ipc = IPC.write();
    let (endpoint_id, rights) = match ipc.cap(caller, connection)? {
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
    let mut prepared = ipc.stage_call(caller)?;

    let mut memory_caps = Vec::new();
    let transfers = read_vector_page(caller, cap_vector, server, true, &mut memory_caps)?;
    prepared.attach_vector(transfers, memory_caps)?;
    let (call_cap, delivery) = prepared.commit(&mut ipc, endpoint_id, opcode, arg0)?;
    drop(ipc);
    deliver(delivery);

    let _ = crate::memory::object::close_cap(caller, cap_vector);
    Ok(call_cap)
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
    let mut entries = Vec::with_capacity(count);
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
