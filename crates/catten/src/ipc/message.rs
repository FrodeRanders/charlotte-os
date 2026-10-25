//! Queue publication, owning post-IPC notification and receive transactions.
//! Hidden attachments become usable only at receive commit; speculative reply
//! authority and retired metadata remain owned through ordinary rejection.
use super::*;

/// What must be signalled after an enqueue, once the IPC lock is dropped:
/// blocked endpoint receivers, plus (for CQ-bound endpoints) a coalesced
/// readiness wake on the owner's completion queue.
pub(super) struct Delivery {
    observers: WaitNotifications,
    cq_wake: Option<(AddressSpaceId, crate::completion::CqId)>,
}

impl Delivery {
    pub(super) fn new(
        observers: WaitNotifications,
        cq_wake: Option<(AddressSpaceId, crate::completion::CqId)>,
    ) -> Self {
        Self {
            observers,
            cq_wake,
        }
    }

    pub(super) fn notify(self) {
        if let Some((asid, cq)) = self.cq_wake {
            crate::completion::wake(asid, cq);
        }
        self.observers.notify();
    }
}

pub(super) fn enqueue_scalar(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<ReplyTokenId>,
) -> Result<Delivery, IpcError> {
    enqueue_message(
        ipc,
        endpoint_id,
        sender,
        opcode,
        arg0,
        reply,
        MemoryAttachments::default(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn enqueue_message(
    ipc: &mut IpcRegistry,
    endpoint_id: EndpointId,
    sender: AddressSpaceId,
    opcode: u32,
    arg0: u64,
    reply: Option<ReplyTokenId>,
    memory: MemoryAttachments,
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
    if endpoint.closing {
        return Err(IpcError::Pending);
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

pub(super) fn reserve_endpoint_queue(
    ipc: &IpcRegistry,
    endpoint_id: EndpointId,
) -> Result<AddressSpaceId, IpcError> {
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    if endpoint.closing {
        return Err(IpcError::Pending);
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
    ReceiveResources::receive(receiver, endpoint_cap, None)
}

/// The complete receive preparation survives guard exit on every ordinary path.
/// Both fields retain their allocations and original charges on abandonment.
struct ReceiveResources {
    authority: PreparingCapability,
    retired: RetiredMetadata,
}
impl ReceiveResources {
    fn receive(
        receiver: AddressSpaceId,
        endpoint_cap: CapabilityId,
        result_page: Option<MemoryObjectCap>,
    ) -> Result<ScalarMessage, IpcError> {
        let mut owner = Self {
            authority: PreparingCapability::new(),
            retired: RetiredMetadata::new(),
        };
        let result = (|| {
            let mut ipc = IPC.write();
            let endpoint_id = receive_endpoint_id(&ipc, receiver, endpoint_cap)?;
            ipc.accepting_namespace(receiver)?;
            let prepared = PreparedReceive::new(
                &mut ipc,
                receiver,
                endpoint_id,
                &mut owner.authority,
                &mut owner.retired,
            )?;
            if let Some(page) = result_page {
                prepared.write_result(page)?;
            }
            Ok(prepared.commit())
        })();
        owner.finish();
        result
    }

    fn finish(mut self) {
        self.retired.release();
        self.authority.finish();
    }
}

/// Own speculative reply authority until dequeue commits. IPC stays exclusively
/// borrowed, so guessed handles cannot use that authority before delivery. A
/// result-page failure drops only this capability. The queued call/token and
/// its loans stay intact, so delivered attachments need no reverse cleanup.
#[must_use]
struct PreparedReceive<'a> {
    ipc: &'a mut IpcRegistry,
    receiver: AddressSpaceId,
    endpoint: EndpointId,
    reply: Option<CapabilityId>,
    retired: &'a mut RetiredMetadata,
}

impl<'a> PreparedReceive<'a> {
    fn new(
        ipc: &'a mut IpcRegistry,
        receiver: AddressSpaceId,
        endpoint: EndpointId,
        authority: &mut PreparingCapability,
        retired: &'a mut RetiredMetadata,
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
            authority.prepare(ipc, receiver)?;
            authority.publish()?;
            Some(authority.install(
                ipc,
                receiver,
                Capability::ReplyToken {
                    token,
                },
                None,
                false,
            ))
        } else {
            None
        };
        Ok(Self {
            ipc,
            receiver,
            endpoint,
            reply,
            retired,
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
        let (message, interface, version) = {
            let endpoint = self.ipc.endpoints.get_mut(&self.endpoint).unwrap();
            (
                endpoint.queue.pop_front().expect("prepared receive retains queue ownership"),
                endpoint.interface,
                endpoint.version,
            )
        };
        if let Some(cap) = message.connection {
            self.ipc.deliver_connection(self.receiver, cap);
        }
        crate::memory::object::deliver_memory(self.receiver, &message.memory);
        ScalarMessage {
            sender: message.sender,
            sender_generation: message.sender_generation,
            sender_principal: message.sender_principal,
            sender_roles: message.sender_roles,
            interface,
            version,
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
            self.ipc
                .remove_non_connection_cap(self.receiver, cap, self.retired)
                .expect("prepared reply authority disappeared");
        }
    }
}

/// Receive a message like [`receive`], but also fill a result page with
/// the cap IDs of all delivered memory objects (up to [`CAP_VECTOR_MAX`]).
pub fn receive_vec(
    receiver: AddressSpaceId,
    endpoint_cap: CapabilityId,
    result_page: MemoryObjectCap,
) -> Result<ScalarMessage, IpcError> {
    ReceiveResources::receive(receiver, endpoint_cap, Some(result_page))
}
