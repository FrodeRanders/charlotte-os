//! Per-capability payload storage accompanies authority before publication.
//! Detached payload and authority metadata leave IPC in the same owning list.
use super::*;

type Storage = PreparedEntry<(CapabilityId, AdmittedCapability), Option<record_budget::Charge>>;

struct Resources {
    captured: Option<(AddressSpaceId, Option<crate::memory::AddressSpaceHandle>)>,
    preparation: Option<crate::capability::PreparedReservation>,
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
            captured: None,
            preparation: None,
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
        // Guarded callers keep their early retirement rejection before factories.
        ipc.accepting_namespace(owner)?;
        let identity = ipc.caps.get(&owner).ok_or(IpcError::PermissionDenied)?.address_space;
        self.prepare_authority_storage(owner, identity)?;
        self.reserve(ipc)?;
        before(1)?;
        self.prepare_payload()?;
        before(2)
    }

    /// The containing transaction retains the exact root while this factory
    /// runs outside IPC. No live capability identity is admitted here.
    pub(super) fn prepare_authority_storage(
        &mut self,
        owner: AddressSpaceId,
        identity: Option<crate::memory::AddressSpaceHandle>,
    ) -> Result<(), IpcError> {
        assert!(self.0.captured.is_none() && self.0.preparation.is_none());
        self.0.captured = Some((owner, identity));
        self.0.preparation = Some(
            crate::capability::PreparedReservation::try_new(
                owner,
                crate::capability::ObjectKind::Ipc,
                identity,
            )
            .map_err(cap_admission_error)?,
        );
        Ok(())
    }

    /// Revalidate the captured IPC namespace before relinking prepared shared
    /// authority. Rejection leaves unused metadata in this owner for finish.
    pub(super) fn reserve(&mut self, ipc: &mut IpcRegistry) -> Result<(), IpcError> {
        assert!(self.0.authority.is_none());
        let (owner, identity) = self.0.captured.unwrap();
        if ipc.caps.get(&owner).map(|caps| caps.address_space) != Some(identity) {
            return Err(IpcError::PermissionDenied);
        }
        ipc.accepting_namespace(owner)?;
        self.0.authority = Some(
            self.0.preparation.as_mut().unwrap().reserve_captured().map_err(cap_admission_error)?,
        );
        Ok(())
    }

    /// Shared count admission precedes this allocation. The exact-root parent
    /// owns its reservation through the unlocked interval and final revalidation.
    pub(super) fn prepare_payload(&mut self) -> Result<(), IpcError> {
        assert!(self.0.authority.is_some() && self.0.storage.is_none());
        self.0.storage = Some(Storage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit)?);
        Ok(())
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
        if let Some(preparation) = self.0.preparation.take() {
            preparation.finish();
        }
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
