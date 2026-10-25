//! Prepared call/connection publication and reply-retirement transactions.
//!
//! Each complete owner carries its captured roots, authority, storage, charges
//! and attachments from preparation through publication or ordinary rejection.
//! Explicit finish follows local IPC unlock; abandonment retains the original
//! complete transaction. This boundary adds no cleanup/replay authority and does
//! not qualify a caller's outer IRQ masks or unrelated guards.
use core::{
    mem::ManuallyDrop,
    ops::{
        Deref,
        DerefMut,
    },
};

use super::*;

/// Own fresh connection authority and its sponsor before any attachment commits.
pub(super) struct PreparedConnection {
    target: AddressSpaceId,
    endpoint: EndpointId,
    rights: ConnectionRights,
    pub(super) authority: PreparingCapability,
    charge: ManuallyDrop<Option<record_budget::Charge>>,
}

impl PreparedConnection {
    pub(super) fn empty(
        target: AddressSpaceId,
        endpoint: EndpointId,
        rights: ConnectionRights,
    ) -> Self {
        Self {
            target,
            endpoint,
            rights,
            authority: PreparingCapability::new(),
            charge: ManuallyDrop::new(None),
        }
    }

    pub(super) fn prepare(
        &mut self,
        ipc: &mut IpcRegistry,
        sponsor: AddressSpaceId,
    ) -> Result<(), IpcError> {
        *self.charge = Some(ipc.reserve_records(sponsor, [1, 0, 0])?);
        self.authority.prepare(ipc, self.target)
    }

    pub(super) fn prepare_into(
        slot: &mut Option<Self>,
        ipc: &mut IpcRegistry,
        sponsor: AddressSpaceId,
        target: AddressSpaceId,
        endpoint: EndpointId,
        rights: ConnectionRights,
    ) -> Result<(), IpcError> {
        assert!(slot.is_none());
        *slot = Some(Self::empty(target, endpoint, rights));
        slot.as_mut().unwrap().prepare(ipc, sponsor)
    }

    /// Joint authority publication and payload installation share IPC exclusion.
    pub(super) fn install(&mut self, ipc: &mut IpcRegistry) -> CapabilityId {
        self.authority.install(
            ipc,
            self.target,
            Capability::Connection {
                endpoint: self.endpoint,
                rights: self.rights,
            },
            self.charge.take(),
            true,
        )
    }

    pub(super) fn finish(mut self) {
        self.authority.finish();
        drop(self.charge.take());
    }
}

pub(super) type PendingStorage =
    PreparedEntry<(PendingCallId, PendingCall), Option<record_budget::Charge>>;
pub(super) type ReplyStorage =
    PreparedEntry<(ReplyTokenId, ReplyToken), Option<record_budget::Charge>>;
pub(super) type RetiredReply =
    RetiredEntry<(ReplyTokenId, ReplyToken), Option<record_budget::Charge>>;
pub(super) type RetiredCall =
    RetiredEntry<(PendingCallId, PendingCall), Option<record_budget::Charge>>;

/// The close operation reuses detached nodes as its bounded retirement list.
/// Fallback retains the complete chain; disposal is explicit after IPC unlock.
#[must_use]
pub(super) struct RetiredReplies(
    RetirementList<(ReplyTokenId, ReplyToken), Option<record_budget::Charge>>,
);

impl RetiredReplies {
    pub(super) fn new() -> Self {
        Self(RetirementList::new())
    }

    pub(super) fn push(&mut self, node: RetiredReply) {
        self.0.push(node);
    }

    pub(super) fn release(mut self) {
        while let Some(node) = self.0.pop() {
            node.release();
        }
    }
}

#[must_use]
pub(super) struct ReplyCompletion(ManuallyDrop<ReplyCompletionResources>);

struct ReplyCompletionResources {
    token: Option<RetiredReply>,
    observers: Option<WaitNotifications>,
    connection: Option<PreparedConnection>,
    memory: Option<crate::memory::object::PreparedTransfer>,
    retired: RetiredMetadata,
}

impl ReplyCompletion {
    /// Publish this solicited reply under the caller's existing IPC guard.
    /// Keep all returned resources and detached metadata in this same owner
    /// until explicit post-guard finish, including ordinary rejection.
    pub(super) fn publish(
        &mut self,
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
        if let Some((endpoint, rights)) = returned_connection {
            PreparedConnection::prepare_into(
                &mut self.0.connection,
                ipc,
                caller,
                caller,
                endpoint,
                rights,
            )?;
        }
        if let Some(memory_cap) = returned_memory {
            self.0.memory = Some(
                crate::memory::object::prepare_move(server, memory_cap, caller)
                    .map_err(|_| IpcError::MemoryTransferFailed)?,
            );
        }
        // Record each successful revocation immediately. A later unmap failure or
        // retired destination leaves only the still-live loans for retry/cancel.
        while let Some(borrow) = ipc.reply_tokens.get(&token_id).unwrap().borrows.last().copied() {
            revoke_memory_borrow(borrow).map_err(|_| IpcError::MemoryTransferFailed)?;
            ipc.reply_tokens.get_mut(&token_id).unwrap().borrows.pop();
        }
        let returned_memory_cap = self.0.memory.as_ref().map(|transfer| transfer.target_cap());
        let resources = &mut *self.0;
        if let Some(connection) = &mut resources.connection {
            crate::memory::object::commit_undelivered_transfers_with_authority(
                resources.memory.as_mut_slice(),
                &mut [connection.authority.authority()],
            )
        } else {
            crate::memory::object::commit_undelivered_transfers(resources.memory.as_mut_slice())
        }
        .map_err(|_| IpcError::MemoryTransferFailed)?;
        drop(self.0.memory.take());
        let returned_cap = self.0.connection.as_mut().map(|connection| connection.install(ipc));
        let token = ipc.reply_tokens.take(&token_id).expect("completed reply token missing");
        let call = ipc.pending_calls.get_mut(&call_id).expect("completed call missing");
        call.result = Some(ReplyValue {
            result,
            cap: returned_cap,
            memory: returned_memory_cap,
        });
        let observers = call.observers.close();
        ipc.remove_non_connection_cap(server, reply_cap, &mut self.0.retired)
            .expect("completed reply cap missing");
        self.0.token = Some(token);
        self.0.observers = Some(observers);
        Ok(())
    }

    pub(super) fn new(token: RetiredReply, observers: WaitNotifications) -> Self {
        Self(ManuallyDrop::new(ReplyCompletionResources {
            token: Some(token),
            observers: Some(observers),
            connection: None,
            memory: None,
            retired: RetiredMetadata::new(),
        }))
    }

    pub(super) fn empty() -> Self {
        Self(ManuallyDrop::new(ReplyCompletionResources {
            token: None,
            observers: None,
            connection: None,
            memory: None,
            retired: RetiredMetadata::new(),
        }))
    }

    pub(super) fn finish(mut self) {
        drop(self.0.memory.take());
        if let Some(connection) = self.0.connection.take() {
            connection.finish();
        }
        self.0.retired.release();
        if let Some(token) = self.0.token.take() {
            token.release();
        }
        if let Some(observers) = self.0.observers.take() {
            signal_observers(observers);
        }
    }
}

/// Own unpublished call storage, authority and attachments. Ordinary finish
/// runs after IPC unlock; fallback retains the complete partial transaction.
#[must_use]
pub(super) struct PreparedCall(ManuallyDrop<CallResources>);

pub(super) struct CallResources {
    caller: AddressSpaceId,
    identity: Option<crate::memory::AddressSpaceHandle>,
    root: Option<crate::memory::operation::AddressSpaceOperation>,
    call_charge: Option<record_budget::Charge>,
    pub(super) platform: bool,
    pub(super) storage: Option<PendingStorage>,
    pub(super) reply_storage: Option<ReplyStorage>,
    pub(super) authority: Option<PreparingCapability>,
    pub(super) pending: Option<PendingCall>,
    reply_charge: Option<record_budget::Charge>,
    pub(super) connection: Option<PreparedConnection>,
    attaching: Option<crate::memory::object::PreparedTransfer>,
    pub(super) transfers: Vec<crate::memory::object::PreparedTransfer>,
    pub(super) memory: Vec<MemoryObjectCap>,
    pub(super) borrows: Vec<MemoryBorrow>,
    attempted: bool,
}

impl Deref for PreparedCall {
    type Target = CallResources;

    fn deref(&self) -> &CallResources {
        &self.0
    }
}
impl DerefMut for PreparedCall {
    fn deref_mut(&mut self) -> &mut CallResources {
        &mut self.0
    }
}

impl PreparedCall {
    /// Revalidate this captured preparation before attachment work. The caller
    /// retains its existing IPC write guard through staging and publication.
    pub(super) fn stage(
        &mut self,
        ipc: &mut IpcRegistry,
        caller: AddressSpaceId,
    ) -> Result<(), IpcError> {
        assert!(self.authority.is_some() && self.pending.is_some());
        if self.caller != caller
            || ipc.caps.get(&caller).and_then(|caps| caps.address_space) != self.identity
        {
            return Err(IpcError::PermissionDenied);
        }
        ipc.accepting_namespace(caller)?;
        Ok(())
    }

    pub(super) fn try_new(caller: AddressSpaceId) -> Result<Self, IpcError> {
        Self::prepare(
            caller,
            || PendingStorage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit),
            || ReplyStorage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit),
        )
    }

    pub(super) fn prepare(
        caller: AddressSpaceId,
        allocate: impl FnOnce() -> Result<PendingStorage, IpcError>,
        allocate_reply: impl FnOnce() -> Result<ReplyStorage, IpcError>,
    ) -> Result<Self, IpcError> {
        Self::prepare_checked(caller, None, allocate, allocate_reply)
    }

    pub(super) fn prepare_checked(
        caller: AddressSpaceId,
        connection: Option<CapabilityId>,
        allocate: impl FnOnce() -> Result<PendingStorage, IpcError>,
        allocate_reply: impl FnOnce() -> Result<ReplyStorage, IpcError>,
    ) -> Result<Self, IpcError> {
        Self::prepare_checked_with(caller, connection, allocate, allocate_reply, |_| Ok(()))
    }

    pub(super) fn prepare_checked_with(
        caller: AddressSpaceId,
        connection: Option<CapabilityId>,
        allocate: impl FnOnce() -> Result<PendingStorage, IpcError>,
        allocate_reply: impl FnOnce() -> Result<ReplyStorage, IpcError>,
        mut before: impl FnMut(u8) -> Result<(), IpcError>,
    ) -> Result<Self, IpcError> {
        let identity = {
            let ipc = IPC.read();
            let identity = ipc.caps.get(&caller).ok_or(IpcError::PermissionDenied)?.address_space;
            if let Some(connection) = connection {
                let Capability::Connection {
                    endpoint,
                    rights,
                } = ipc.cap(caller, connection)?
                else {
                    return Err(IpcError::WrongType);
                };
                if !rights.contains(ConnectionRights::CALL) {
                    return Err(IpcError::PermissionDenied);
                }
                reserve_endpoint_queue(&ipc, endpoint)?;
            }
            identity
        };
        let mut owner = Self(ManuallyDrop::new(CallResources {
            caller,
            identity,
            root: None,
            call_charge: None,
            platform: false,
            storage: None,
            reply_storage: None,
            authority: None,
            pending: None,
            reply_charge: None,
            connection: None,
            attaching: None,
            transfers: Vec::new(),
            memory: Vec::new(),
            borrows: Vec::new(),
            attempted: false,
        }));
        let prepared = (|| {
            if caller != crate::memory::KERNEL_ASID {
                match identity {
                    Some(handle) => {
                        owner.root = Some(
                            crate::memory::operation::AddressSpaceOperation::acquire(handle)
                                .map_err(|_| IpcError::PermissionDenied)?,
                        )
                    }
                    None if caller == namespace::fixture_id(caller) => {}
                    None => return Err(IpcError::PermissionDenied),
                }
            }
            {
                let mut ipc = IPC.write();
                if ipc.caps.get(&caller).and_then(|caps| caps.address_space) != identity {
                    return Err(IpcError::PermissionDenied);
                }
                let mut charge = ipc.reserve_records(caller, [0, 1, 1])?;
                owner.platform = charge.platform();
                owner.reply_charge = Some(charge.split([0, 0, 1]));
                owner.call_charge = Some(charge);
            }
            owner.storage = Some(allocate()?);
            let resources = &mut *owner.0;
            resources.storage.as_mut().unwrap().bind_charge(resources.call_charge.take().unwrap());
            owner.reply_storage = Some(allocate_reply()?);
            let resources = &mut *owner.0;
            resources
                .reply_storage
                .as_mut()
                .unwrap()
                .bind_charge(resources.reply_charge.take().unwrap());
            owner.authority = Some(PreparingCapability::new());
            before(1)?;
            owner.authority.as_mut().unwrap().prepare_authority_storage(caller, identity)?;
            before(2)?;
            {
                let mut ipc = IPC.write();
                owner.authority.as_mut().unwrap().reserve(&mut ipc)?;
            }
            before(3)?;
            owner.authority.as_mut().unwrap().prepare_payload()?;
            before(4)?;
            owner.pending = Some(PendingCall::try_new(caller, owner.platform)?);
            before(5)?;
            Ok(())
        })();
        if let Err(error) = prepared {
            owner.finish();
            return Err(error);
        }
        Ok(owner)
    }

    /// Work's IPC guards leave on every ordinary return before unused resources
    /// finish. Abandonment retains this complete transaction without cleanup.
    pub(super) fn run(
        caller: AddressSpaceId,
        connection: CapabilityId,
        work: impl FnOnce(&mut Self) -> Result<(CapabilityId, Delivery), IpcError>,
    ) -> Result<CapabilityId, IpcError> {
        let mut owner = Self::prepare_checked(
            caller,
            Some(connection),
            || PendingStorage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit),
            || ReplyStorage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit),
        )?;
        let result = work(&mut owner);
        owner.finish();
        let (cap, delivery) = result?;
        deliver(delivery);
        Ok(cap)
    }

    pub(super) fn finish(mut self) {
        if let Some(connection) = self.0.connection.take() {
            connection.finish();
        }
        if let Some(authority) = self.0.authority.take() {
            authority.finish();
        }
        drop(core::mem::take(&mut self.0.transfers));
        drop(self.0.attaching.take());
        drop(core::mem::take(&mut self.0.memory));
        drop(core::mem::take(&mut self.0.borrows));
        drop(self.0.pending.take());
        drop(self.0.reply_storage.take());
        drop(self.0.reply_charge.take());
        drop(self.0.storage.take());
        drop(self.0.call_charge.take());
        if let Some(root) = self.0.root.take() {
            root.release().expect("call preparation lost its exact-root lease");
        }
    }

    pub(super) fn attach(
        &mut self,
        transfer: crate::memory::object::PreparedTransfer,
    ) -> Result<(), IpcError> {
        let resources = &mut *self.0;
        assert!(resources.attaching.is_none());
        resources.attaching = Some(transfer);
        resources.transfers.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
        resources.memory.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
        let transfer = resources.attaching.as_ref().unwrap();
        if let Some((owner, owner_cap)) = transfer.loan_origin() {
            resources.borrows.try_reserve_exact(1).map_err(|_| IpcError::ResourceLimit)?;
            resources.borrows.push(MemoryBorrow {
                owner,
                owner_cap,
                borrower: transfer.target(),
                borrower_cap: transfer.target_cap(),
            });
        }
        resources.memory.push(transfer.target_cap());
        resources.transfers.push(resources.attaching.take().unwrap());
        Ok(())
    }

    pub(super) fn attach_vector(
        &mut self,
        transfers: Vec<crate::memory::object::PreparedTransfer>,
        memory: Vec<MemoryObjectCap>,
    ) -> Result<(), IpcError> {
        let resources = &mut *self.0;
        resources.transfers = transfers;
        resources.memory = memory;
        resources
            .borrows
            .try_reserve_exact(resources.transfers.len())
            .map_err(|_| IpcError::ResourceLimit)?;
        for transfer in &resources.transfers {
            if let Some((owner, owner_cap)) = transfer.loan_origin() {
                resources.borrows.push(MemoryBorrow {
                    owner,
                    owner_cap,
                    borrower: transfer.target(),
                    borrower_cap: transfer.target_cap(),
                });
            }
        }
        Ok(())
    }

    pub(super) fn commit(
        &mut self,
        ipc: &mut IpcRegistry,
        endpoint: EndpointId,
        opcode: u32,
        arg0: u64,
    ) -> Result<(CapabilityId, Delivery), IpcError> {
        self.commit_with_memory(ipc, endpoint, opcode, arg0, MemoryAttachments::try_new)
    }

    pub(super) fn commit_with_memory(
        &mut self,
        ipc: &mut IpcRegistry,
        endpoint: EndpointId,
        opcode: u32,
        arg0: u64,
        prepare_memory: impl FnOnce(Vec<MemoryObjectCap>) -> Result<MemoryAttachments, IpcError>,
    ) -> Result<(CapabilityId, Delivery), IpcError> {
        assert!(!self.0.attempted, "call publication attempted twice");
        self.0.attempted = true;
        let resources = &mut *self.0;
        let (server, memory) = (|| {
            let server = reserve_endpoint_queue(ipc, endpoint)?;
            let memory = prepare_memory(core::mem::take(&mut resources.memory))?;
            if let Some(connection) = &mut resources.connection {
                crate::memory::object::commit_undelivered_transfers_with_authority(
                    &mut resources.transfers,
                    &mut [
                        resources.authority.as_mut().unwrap().authority(),
                        connection.authority.authority(),
                    ],
                )
            } else {
                crate::memory::object::commit_undelivered_transfers_with_authority(
                    &mut resources.transfers,
                    &mut [resources.authority.as_mut().unwrap().authority()],
                )
            }
            .map_err(|_| IpcError::MemoryTransferFailed)?;
            Ok((server, memory))
        })()?;
        let call_cap = resources.authority.as_ref().unwrap().identity();
        // Pins must end before the newly published objects become writable.
        resources.transfers.clear();
        let caller = resources.pending.as_ref().unwrap().caller;
        let attached = resources.connection.as_mut().map(|connection| connection.install(ipc));
        let call = ipc.alloc_call();
        ipc.pending_calls.insert(
            resources.storage.take().unwrap(),
            call,
            resources.pending.take().unwrap(),
        );
        resources.authority.as_mut().unwrap().install(
            ipc,
            caller,
            Capability::PendingCall {
                call,
            },
            None,
            false,
        );
        let token = ipc.alloc_reply();
        ipc.reply_tokens.insert(
            resources.reply_storage.take().unwrap(),
            token,
            ReplyToken {
                server,
                call,
                platform: resources.platform,
                completing: false,
                claim_interest: None,
                claim_epoch: 0,
                cleanup_failed: false,
                connection_source: None,
                borrows: core::mem::take(&mut resources.borrows),
            },
        );
        let delivery =
            enqueue_message(ipc, endpoint, caller, opcode, arg0, Some(token), memory, attached)
                .expect("prepared call retains endpoint queue ownership");
        Ok((call_cap, delivery))
    }
}
