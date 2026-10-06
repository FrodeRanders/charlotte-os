//! Explicit endpoint retirement retains its server root while queued calls
//! complete one at a time. Each call owns its caller lease and loan receipts;
//! no whole-queue teardown snapshot or new live lease under IPC is needed.

use super::*;
use crate::memory::{
    AddressSpaceHandle,
    KERNEL_ASID,
    object::{
        LoanRevocation,
        MemoryObjectError,
    },
    operation::AddressSpaceOperation,
};

/// Drop retains the endpoint claim and server lease. It never closes authority,
/// wakes callers or releases potentially reachable backing under unknown locks.
#[must_use]
pub(super) struct PreparedEndpointClose {
    asid: AddressSpaceId,
    cap: CapabilityId,
    endpoint: EndpointId,
    handle: AddressSpaceHandle,
    namespace: Option<AddressSpaceOperation>,
}

fn resolve(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
) -> Result<(EndpointId, AddressSpaceHandle), IpcError> {
    let Capability::Endpoint {
        endpoint,
        ..
    } = ipc.cap(asid, cap)?
    else {
        return Err(IpcError::WrongType);
    };
    let record = ipc.endpoints.get(&endpoint).ok_or(IpcError::UnknownCapability)?;
    if record.owner != asid {
        return Err(IpcError::PermissionDenied);
    }
    if record.closed {
        return Err(IpcError::EndpointClosed);
    }
    if record.closing
        || ipc.reply_tokens.values().any(|token| {
            token.completing && token.server == asid && token.connection_source == Some(cap)
        })
    {
        return Err(IpcError::Pending);
    }
    let handle = ipc
        .caps
        .get(&asid)
        .and_then(|caps| caps.address_space)
        .ok_or(IpcError::UnknownCapability)?;
    Ok((endpoint, handle))
}

impl PreparedEndpointClose {
    pub(super) fn prepare(asid: AddressSpaceId, cap: CapabilityId) -> Result<Self, IpcError> {
        let (endpoint, handle) = resolve(&IPC.read(), asid, cap)?;
        let namespace = if asid == KERNEL_ASID {
            None
        } else {
            Some(AddressSpaceOperation::acquire(handle).map_err(|_| IpcError::ResourceLimit)?)
        };
        let owner = Self {
            asid,
            cap,
            endpoint,
            handle,
            namespace,
        };
        let prepared = {
            let mut ipc = IPC.write();
            match resolve(&ipc, asid, cap) {
                Ok(identity) if identity == (endpoint, handle) => {
                    ipc.endpoints.get_mut(&endpoint).unwrap().closing = true;
                    Ok(())
                }
                Ok(_) => Err(IpcError::Pending),
                Err(error) => Err(error),
            }
        };
        if let Err(error) = prepared {
            owner.release_namespace()?;
            return Err(error);
        }
        Ok(owner)
    }

    pub(super) fn asid(&self) -> AddressSpaceId {
        self.asid
    }

    pub(super) fn cap(&self) -> CapabilityId {
        self.cap
    }

    pub(super) fn handle(&self) -> AddressSpaceHandle {
        self.handle
    }

    fn release_namespace(mut self) -> Result<(), IpcError> {
        if let Some(namespace) = self.namespace.take() {
            namespace.release().map_err(|_| IpcError::UnknownCapability)?;
        }
        Ok(())
    }

    fn reject(self, error: IpcError) -> Result<(), IpcError> {
        // All current call receipts have terminated or were never claimed.
        // Failed loan pins/token fences survive independently; untouched calls
        // can still be cancelled. Ordinary error does not abandon our root lease.
        let delivery = {
            let mut ipc = IPC.write();
            assert_eq!(ipc.caps[&self.asid].address_space, Some(self.handle));
            {
                let endpoint =
                    ipc.endpoints.get_mut(&self.endpoint).expect("claimed endpoint missing");
                assert!(endpoint.closing && !endpoint.closed);
                endpoint.closing = false;
            }
            // A reader may have parked while our claim hid a readable front.
            // Ordinary rejection must restore that readiness edge after unlock.
            let ready = endpoint_available(&ipc, &ipc.endpoints[&self.endpoint]);
            let endpoint = ipc.endpoints.get_mut(&self.endpoint).unwrap();
            Delivery {
                observers: if ready {
                    endpoint.readiness_observers.drain()
                } else {
                    WaitNotifications::empty()
                },
                cq_wake: if ready {
                    endpoint.notify_cq.map(|cq| (endpoint.owner, cq))
                } else {
                    None
                },
            }
        };
        self.release_namespace()?;
        deliver(delivery);
        Err(error)
    }

    pub(super) fn finish_with(
        self,
        finish: impl FnMut(LoanRevocation) -> Result<(), MemoryObjectError>,
        wait: impl FnMut(),
    ) -> Result<(), IpcError> {
        self.finish_with_cleanup(finish, wait, attachments::release_one)
    }

    pub(super) fn finish_with_cleanup(
        self,
        mut finish: impl FnMut(LoanRevocation) -> Result<(), MemoryObjectError>,
        mut wait: impl FnMut(),
        mut release: impl FnMut(crate::memory::object::RetiredMemory),
    ) -> Result<(), IpcError> {
        loop {
            let memory = {
                let mut ipc = IPC.write();
                assert_eq!(ipc.caps[&self.asid].address_space, Some(self.handle));
                let endpoint =
                    ipc.endpoints.get_mut(&self.endpoint).expect("claimed endpoint missing");
                assert!(endpoint.closing && !endpoint.closed);
                let Some(front) = endpoint.queue.front() else {
                    break;
                };
                if front.reply.is_none() {
                    // No borrow is attached to an asynchronous send. Its
                    // undelivered move/copy caps cannot acquire application
                    // mappings, loans or DMA/copy pins before this cleanup.
                    let mut message = endpoint.queue.pop_front().unwrap();
                    message.memory.retire(self.asid);
                    if let Some(cap) = message.connection {
                        let _ = ipc.remove_cap(self.asid, cap);
                    }
                    Some(message.memory)
                } else {
                    None
                }
            };
            if let Some(memory) = memory {
                memory.release_with(&mut release);
                continue;
            }
            // Lifecycle admission precedes IPC. A competing caller cancellation
            // can win between capture and claim; re-resolve the current front.
            let result = match cancellation::PreparedCancellation::prepare_endpoint_front(&self) {
                Ok(call) => call.finish_with_cleanup(&mut finish, &mut release),
                Err(IpcError::Pending) => {
                    wait();
                    continue;
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                return self.reject(error);
            }
        }

        // The queue is empty and admission remains fenced through publication.
        // Never clear the claim before acquiring IPC again: a new loan could
        // otherwise enter and fall back to serialized physical cleanup.
        let (observers, watches, cq_wake) = {
            let mut ipc = IPC.write();
            assert_eq!(ipc.caps[&self.asid].address_space, Some(self.handle));
            let endpoint = ipc.endpoints.get_mut(&self.endpoint).expect("claimed endpoint missing");
            assert!(endpoint.closing && !endpoint.closed && endpoint.queue.is_empty());
            endpoint.closed = true;
            endpoint.closing = false;
            let observers = endpoint.readiness_observers.close();
            let watches = endpoint.close_observers.close();
            let cq_wake = endpoint.notify_cq.map(|cq| (endpoint.owner, cq));
            assert!(
                matches!(ipc.remove_cap(self.asid, self.cap)?, Capability::Endpoint { endpoint, .. }
                if endpoint == self.endpoint)
            );
            if !endpoint_referenced(&ipc, self.endpoint) {
                ipc.endpoints.remove(&self.endpoint);
            }
            (observers, watches, cq_wake)
        };
        let released = self.release_namespace();
        watches.notify();
        if let Some((asid, cq)) = cq_wake {
            crate::completion::wake(asid, cq);
        }
        signal_observers(observers);
        released
    }
}

pub(super) fn close(asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
    PreparedEndpointClose::prepare(asid, cap)?
        .finish_with(LoanRevocation::finish, crate::cpu::scheduler::yield_lp)
}
