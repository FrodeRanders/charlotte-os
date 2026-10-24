//! Per-capability payload storage accompanies authority before publication.
//! Detached payload and authority metadata leave IPC in the same owning list.
use super::*;

type Storage = PreparedEntry<(CapabilityId, AdmittedCapability), Option<record_budget::Charge>>;

struct Resources {
    authority: Option<crate::capability::Reservation>,
    storage: Option<Storage>,
}

/// Existing containing operations explicitly finish unused preparation after
/// their IPC guard leaves. Abandonment retains both authority and payload storage.
#[must_use]
pub(super) struct PreparingCapability(ManuallyDrop<Resources>);

impl PreparingCapability {
    pub(super) fn new() -> Self {
        Self(ManuallyDrop::new(Resources {
            authority: None,
            storage: None,
        }))
    }

    pub(super) fn prepare(
        &mut self,
        ipc: &mut IpcRegistry,
        owner: AddressSpaceId,
    ) -> Result<(), IpcError> {
        self.prepare_with(ipc, owner, |_| Ok(()))
    }

    fn prepare_with(
        &mut self,
        ipc: &mut IpcRegistry,
        owner: AddressSpaceId,
        mut before: impl FnMut(u8) -> Result<(), IpcError>,
    ) -> Result<(), IpcError> {
        assert!(self.0.authority.is_none() && self.0.storage.is_none());
        self.0.authority = Some(ipc.reserve_cap(owner)?);
        before(1)?;
        self.0.storage = Some(Storage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit)?);
        before(2)
    }

    pub(super) fn identity(&self) -> CapabilityId {
        self.0.authority.as_ref().unwrap().identity()
    }

    pub(super) fn authority(&mut self) -> &mut crate::capability::Reservation {
        self.0.authority.as_mut().unwrap()
    }

    pub(super) fn publish(&mut self) -> Result<(), IpcError> {
        crate::capability::publish_batch(&mut [crate::capability::Publication {
            destination: self.authority(),
            source: None,
        }])
        .map_err(cap_admission_error)
    }

    pub(super) fn install(
        &mut self,
        ipc: &mut IpcRegistry,
        owner: AddressSpaceId,
        payload: Capability,
        charge: Option<record_budget::Charge>,
        hidden: bool,
    ) -> CapabilityId {
        assert_eq!(matches!(payload, Capability::Connection { .. }), charge.is_some());
        let id = self.identity();
        assert!(
            crate::capability::contains(owner, id, crate::capability::ObjectKind::Ipc),
            "IPC payload installed before authority publication"
        );
        let mut storage = self.0.storage.take().unwrap();
        if let Some(charge) = charge {
            storage.bind_charge(charge);
        }
        ipc.as_caps(owner).caps.insert(
            storage,
            id,
            AdmittedCapability {
                payload,
                delivery_pending: hidden,
                retired_authority: None,
            },
        );
        id
    }

    pub(super) fn finish(mut self) {
        drop(self.0.storage.take());
        drop(self.0.authority.take());
    }
}

/// Reuse detached nodes as temporary storage in the existing cleanup owners.
/// No new registry or teardown allocation is needed; fallback retains both lists.
#[must_use]
pub(super) struct RetiredMetadata {
    endpoints: RetirementList<(EndpointId, Endpoint), Option<budget::Charge>>,
    capabilities: RetirementList<(CapabilityId, AdmittedCapability), Option<record_budget::Charge>>,
}

impl RetiredMetadata {
    pub(super) fn new() -> Self {
        Self {
            endpoints: RetirementList::new(),
            capabilities: RetirementList::new(),
        }
    }

    pub(super) fn push(&mut self, node: endpoint_storage::RetiredEndpoint) {
        self.endpoints.push(node);
    }

    fn push_capability(
        &mut self,
        node: RetiredEntry<(CapabilityId, AdmittedCapability), Option<record_budget::Charge>>,
    ) {
        self.capabilities.push(node);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.endpoints.is_empty() && self.capabilities.is_empty()
    }

    pub(super) fn release(&mut self) {
        while let Some(node) = self.capabilities.pop() {
            node.release_with(|(_, mut payload)| {
                payload.retired_authority.take().unwrap().release();
            });
        }
        while let Some(node) = self.endpoints.pop() {
            node.release();
        }
    }
}

pub(super) fn retire(
    ipc: &mut IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
    retired: &mut RetiredMetadata,
) -> Result<Capability, IpcError> {
    let mut node = ipc
        .caps
        .get_mut(&asid)
        .and_then(|caps| caps.caps.take(&cap))
        .ok_or(IpcError::UnknownCapability)?;
    let payload = &mut node.value_mut().1;
    payload.retired_authority = Some(
        crate::capability::detach(asid, cap, crate::capability::ObjectKind::Ipc)
            .expect("IPC payload capability was absent from unified table"),
    );
    let removed = payload.payload;
    retired.push_capability(node);
    Ok(removed)
}

mod tests;
pub(super) fn test_storage() {
    tests::run();
}
