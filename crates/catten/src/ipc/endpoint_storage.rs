//! Endpoint preparation, queue growth and complete post-IPC metadata disposal.
use core::mem::ManuallyDrop;

use super::*;
use crate::memory::{
    AddressSpaceHandle,
    KERNEL_ASID,
    operation::AddressSpaceOperation,
};

type Storage = PreparedEntry<(EndpointId, Endpoint), Option<budget::Charge>>;
pub(super) type RetiredEndpoint = RetiredEntry<(EndpointId, Endpoint), Option<budget::Charge>>;

struct Root {
    owner: AddressSpaceId,
    identity: Option<AddressSpaceHandle>,
    operation: Option<AddressSpaceOperation>,
}

impl Root {
    fn capture(owner: AddressSpaceId) -> Result<Self, IpcError> {
        let identity = IPC.read().caps.get(&owner).ok_or(IpcError::PermissionDenied)?.address_space;
        let operation = if owner == KERNEL_ASID {
            None
        } else {
            match identity {
                Some(handle) => Some(
                    AddressSpaceOperation::acquire(handle)
                        .map_err(|_| IpcError::PermissionDenied)?,
                ),
                None if owner == namespace::fixture_id(owner) => None,
                None => return Err(IpcError::PermissionDenied),
            }
        };
        Ok(Self {
            owner,
            identity,
            operation,
        })
    }

    fn validate(&self, ipc: &mut IpcRegistry) -> Result<bool, IpcError> {
        let namespace = ipc.caps.get(&self.owner).ok_or(IpcError::PermissionDenied)?;
        if namespace.address_space != self.identity {
            return Err(IpcError::PermissionDenied);
        }
        ipc.accepting_namespace(self.owner)
    }

    fn release(&mut self) {
        if let Some(operation) = self.operation.take() {
            operation.release().expect("endpoint preparation lost its exact-root lease");
        }
    }
}

struct Resources {
    root: Root,
    budget: Option<Arc<budget::DomainBudget>>,
    platform: bool,
    metadata_charge: Option<budget::Charge>,
    queue_charge: Option<budget::Charge>,
    storage: Option<Storage>,
    queue: Option<budget::AdmittedQueue<QueuedMessage>>,
    close_observers: Option<ListRef<crate::completion::watch_budget::Charge>>,
    readiness_observers: Option<WaiterList>,
    authority: Option<PreparingCapability>,
}

#[must_use]
struct PreparingEndpoint(ManuallyDrop<Resources>);

impl PreparingEndpoint {
    fn prepare(
        owner: AddressSpaceId,
        capacity: usize,
        mut before: impl FnMut(u8) -> Result<(), IpcError>,
    ) -> Result<Self, IpcError> {
        let root = Root::capture(owner)?;
        let mut prepared = Self(ManuallyDrop::new(Resources {
            root,
            budget: None,
            platform: false,
            metadata_charge: None,
            queue_charge: None,
            storage: None,
            queue: None,
            close_observers: None,
            readiness_observers: None,
            authority: None,
        }));
        let result = (|| {
            let slots = budget::AdmittedQueue::<QueuedMessage>::required_slots(capacity)
                .map_err(|_| IpcError::ResourceLimit)?;
            {
                let mut ipc = IPC.write();
                let platform = prepared.0.root.validate(&mut ipc)?;
                let domain = ipc.caps[&owner].endpoint_budget.clone();
                let mut charge = budget::reserve(&domain, platform, [1, slots as u64])
                    .map_err(|_| IpcError::ResourceLimit)?;
                prepared.0.platform = charge.platform();
                prepared.0.budget = Some(charge.domain().clone());
                prepared.0.queue_charge = Some(charge.split([0, slots as u64]));
                prepared.0.metadata_charge = Some(charge);
            }
            before(1)?;
            prepared.0.storage =
                Some(Storage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit)?);
            let resources = &mut *prepared.0;
            resources
                .storage
                .as_mut()
                .unwrap()
                .bind_charge(resources.metadata_charge.take().unwrap());
            before(2)?;
            prepared.0.queue =
                Some(budget::AdmittedQueue::reserved(prepared.0.queue_charge.take().unwrap()));
            before(3)?;
            prepared
                .0
                .queue
                .as_mut()
                .unwrap()
                .allocate_backing(capacity)
                .map_err(|_| IpcError::ResourceLimit)?;
            before(4)?;
            prepared.0.close_observers = Some(
                ObserverList::try_new(
                    crate::completion::watch_budget::MAX_ENDPOINT_WATCHES,
                    prepared.0.platform,
                )
                .map_err(|_| IpcError::ResourceLimit)?,
            );
            before(5)?;
            prepared.0.readiness_observers = Some(
                ObserverList::try_new(waiter_budget::SOURCE_LIMIT, prepared.0.platform)
                    .map_err(|_| IpcError::ResourceLimit)?,
            );
            before(6)?;
            prepared.0.authority = Some(PreparingCapability::new());
            before(7)?;
            let identity = prepared.0.root.identity;
            prepared.0.authority.as_mut().unwrap().prepare_authority_storage(owner, identity)?;
            before(8)?;
            {
                let mut ipc = IPC.write();
                prepared.0.root.validate(&mut ipc)?;
                prepared.0.authority.as_mut().unwrap().reserve(&mut ipc)?;
            }
            before(9)?;
            prepared.0.authority.as_mut().unwrap().prepare_payload()?;
            before(10)
        })();
        if let Err(error) = result {
            prepared.finish();
            return Err(error);
        }
        Ok(prepared)
    }

    fn publish(
        &mut self,
        ipc: &mut IpcRegistry,
        interface: u64,
        version: u32,
        capacity: usize,
    ) -> Result<CapabilityId, IpcError> {
        self.0.root.validate(ipc)?;
        let resources = &mut *self.0;
        let owner = resources.root.owner;
        let authority = resources.authority.as_mut().unwrap();
        authority.publish()?;
        let id = ipc.alloc_endpoint();
        let storage = self.0.storage.take().unwrap();
        let payload = self.take_payload(interface, version, capacity);
        ipc.endpoints.insert(storage, id, payload);
        Ok(self.0.authority.as_mut().unwrap().install(
            ipc,
            owner,
            Capability::Endpoint {
                endpoint: id,
                rights: ConnectionRights::ALL,
            },
            None,
            false,
        ))
    }

    fn take_payload(&mut self, interface: u64, version: u32, capacity: usize) -> Endpoint {
        let resources = &mut *self.0;
        Endpoint {
            owner: resources.root.owner,
            interface,
            version,
            capacity,
            high_water: 0,
            queue: resources.queue.take().unwrap(),
            readiness_observers: resources.readiness_observers.take().unwrap(),
            close_observers: resources.close_observers.take().unwrap(),
            closed: false,
            closing: false,
            claim_interest: None,
            claim_epoch: 0,
            notify_cq: None,
            budget: resources.budget.take().unwrap(),
            platform: resources.platform,
        }
    }

    fn finish(mut self) {
        if let Some(authority) = self.0.authority.take() {
            authority.finish();
        }
        drop(self.0.readiness_observers.take());
        drop(self.0.close_observers.take());
        drop(self.0.queue.take());
        drop(self.0.queue_charge.take());
        drop(self.0.storage.take());
        drop(self.0.metadata_charge.take());
        drop(self.0.budget.take());
        self.0.root.release();
    }
}

pub(super) fn create(
    owner: AddressSpaceId,
    interface: u64,
    version: u32,
    capacity: usize,
) -> Result<CapabilityId, IpcError> {
    if capacity == 0 {
        return Err(IpcError::QueueFull);
    }
    let capacity = capacity.min(MAX_ENDPOINT_CAPACITY);
    let mut prepared = PreparingEndpoint::prepare(owner, capacity, |_| Ok(()))?;
    let result = prepared.publish(&mut IPC.write(), interface, version, capacity);
    prepared.finish();
    result
}

struct GrowthResources {
    root: Root,
    endpoint: Option<EndpointId>,
    budget: Option<Arc<budget::DomainBudget>>,
    platform: bool,
    charge: Option<budget::Charge>,
    queue: Option<budget::AdmittedQueue<QueuedMessage>>,
    old_queue: Option<budget::AdmittedQueue<QueuedMessage>>,
}

#[must_use]
struct Growth(ManuallyDrop<GrowthResources>);

fn resize_target(
    ipc: &IpcRegistry,
    owner: AddressSpaceId,
    cap: CapabilityId,
) -> Result<EndpointId, IpcError> {
    let Capability::Endpoint {
        endpoint,
        ..
    } = ipc.cap(owner, cap)?
    else {
        return Err(IpcError::WrongType);
    };
    let record = ipc.endpoints.get(&endpoint).ok_or(IpcError::UnknownCapability)?;
    if record.owner != owner {
        return Err(IpcError::PermissionDenied);
    }
    if record.closed {
        return Err(IpcError::EndpointClosed);
    }
    if record.closing {
        return Err(IpcError::Pending);
    }
    Ok(endpoint)
}

impl Growth {
    fn capture(owner: AddressSpaceId) -> Result<Self, IpcError> {
        Ok(Self(ManuallyDrop::new(GrowthResources {
            root: Root::capture(owner)?,
            endpoint: None,
            budget: None,
            platform: false,
            charge: None,
            queue: None,
            old_queue: None,
        })))
    }

    fn apply(
        &mut self,
        cap: CapabilityId,
        capacity: usize,
        mut before: impl FnMut(u8) -> Result<(), IpcError>,
    ) -> Result<(), IpcError> {
        {
            let mut ipc = IPC.write();
            self.0.root.validate(&mut ipc)?;
            let id = resize_target(&ipc, self.0.root.owner, cap)?;
            let record = ipc.endpoints.get_mut(&id).unwrap();
            if capacity <= record.queue.capacity() {
                record.capacity = capacity;
                return Ok(());
            }
            self.0.endpoint = Some(id);
            self.0.budget = Some(record.budget.clone());
            self.0.platform = record.platform;
        }
        before(1)?;
        let slots = budget::AdmittedQueue::<QueuedMessage>::required_slots(capacity)
            .map_err(|_| IpcError::ResourceLimit)?;
        self.0.charge = Some(
            budget::reserve(self.0.budget.as_ref().unwrap(), self.0.platform, [0, slots as u64])
                .map_err(|_| IpcError::ResourceLimit)?,
        );
        self.0.queue = Some(budget::AdmittedQueue::reserved(self.0.charge.take().unwrap()));
        before(2)?;
        self.0
            .queue
            .as_mut()
            .unwrap()
            .allocate_backing(capacity)
            .map_err(|_| IpcError::ResourceLimit)?;
        before(3)?;
        let mut ipc = IPC.write();
        self.0.root.validate(&mut ipc)?;
        let id = resize_target(&ipc, self.0.root.owner, cap)?;
        if Some(id) != self.0.endpoint {
            return Err(IpcError::UnknownCapability);
        }
        let record = ipc.endpoints.get_mut(&id).unwrap();
        if capacity > record.queue.capacity() {
            let mut replacement = self.0.queue.take().unwrap();
            assert!(record.queue.len() <= replacement.capacity());
            replacement.extend(record.queue.drain(..));
            self.0.old_queue = Some(core::mem::replace(&mut record.queue, replacement));
        }
        record.capacity = capacity;
        Ok(())
    }

    fn finish(mut self) {
        drop(self.0.old_queue.take());
        drop(self.0.queue.take());
        drop(self.0.charge.take());
        drop(self.0.budget.take());
        self.0.root.release();
    }
}

pub(super) fn resize(
    owner: AddressSpaceId,
    cap: CapabilityId,
    capacity: usize,
) -> Result<usize, IpcError> {
    if capacity == 0 {
        return Err(IpcError::QueueFull);
    }
    let capacity = capacity.min(MAX_ENDPOINT_CAPACITY);
    let mut growth = Growth::capture(owner)?;
    let result = growth.apply(cap, capacity, |_| Ok(()));
    growth.finish();
    result.map(|()| capacity)
}

mod tests;

pub(super) fn test_storage() {
    tests::run();
}
