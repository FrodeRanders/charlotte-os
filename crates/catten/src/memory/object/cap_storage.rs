//! Prepared memory capability/namespace nodes and paired metadata retirement.
use super::*;

pub(super) mod tests;

type Storage = PreparedEntry<(MemoryObjectCap, Record)>;
type NamespaceStorage = PreparedEntry<(AddressSpaceId, AddressSpaceCaps)>;

#[derive(Debug)]
struct Record {
    payload: MemoryCap,
    authority: Option<crate::capability::RetiredRecord>,
}

/// Payload borrows remain plain metadata; detached records carry their owner.
#[derive(Debug)]
pub(super) struct Caps(AdmittedMap<MemoryObjectCap, Record>);
impl Caps {
    pub(super) const fn new() -> Self {
        Self(AdmittedMap::new())
    }

    pub(super) fn get(&self, cap: &MemoryObjectCap) -> Option<&MemoryCap> {
        self.0.get(cap).map(|record| &record.payload)
    }

    pub(super) fn get_mut(&mut self, cap: &MemoryObjectCap) -> Option<&mut MemoryCap> {
        self.0.get_mut(cap).map(|record| &mut record.payload)
    }

    pub(super) fn contains_key(&self, cap: &MemoryObjectCap) -> bool {
        self.get(cap).is_some()
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &MemoryCap> {
        self.0.values().map(|record| &record.payload)
    }

    pub(super) fn take(&mut self, cap: &MemoryObjectCap) -> Option<RetiredCapability> {
        self.0.take(cap).map(RetiredCapability)
    }
}
impl core::ops::Index<&MemoryObjectCap> for Caps {
    type Output = MemoryCap;

    fn index(&self, cap: &MemoryObjectCap) -> &MemoryCap {
        self.get(cap).expect("memory payload capability missing")
    }
}

/// Only unused nodes remain in these fields after install. Existing containing
/// owners drop them after their local memory guard, before reservation refund.
/// Source transfer escrow/pin cleanup remains the parent's existing policy.
pub(super) struct Preparing {
    payload: Option<Storage>,
    namespace: Option<NamespaceStorage>,
    // Rust drops fields in declaration order: storage before authority refund.
    authority: crate::capability::Reservation,
    handle: super::super::AddressSpaceHandle,
}
impl Preparing {
    pub(super) fn new(
        authority: crate::capability::Reservation,
        handle: super::super::AddressSpaceHandle,
    ) -> Result<Self, MemoryObjectError> {
        Self::with(authority, handle, |_| Ok(()))
    }

    pub(super) fn with(
        authority: crate::capability::Reservation,
        handle: super::super::AddressSpaceHandle,
        mut before: impl FnMut(u8) -> Result<(), MemoryObjectError>,
    ) -> Result<Self, MemoryObjectError> {
        // Shared authority count admission precedes either payload factory.
        // Function locals cancel a rejected unused prefix in reverse order.
        before(0)?;
        let payload = Storage::try_new().map_err(|_| MemoryObjectError::ResourceLimit)?;
        before(1)?;
        let namespace =
            NamespaceStorage::try_new().map_err(|_| MemoryObjectError::ResourceLimit)?;
        before(2)?;
        Ok(Self {
            payload: Some(payload),
            namespace: Some(namespace),
            authority,
            handle,
        })
    }

    pub(super) fn identity(&self) -> MemoryObjectCap {
        self.authority.identity()
    }

    pub(super) fn authority(&mut self) -> &mut crate::capability::Reservation {
        &mut self.authority
    }

    pub(super) fn validate(
        &self,
        registry: &MemoryObjectRegistry,
    ) -> Result<(), MemoryObjectError> {
        if registry.caps.get(&self.handle.id()).is_some_and(|caps| caps.handle != self.handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        Ok(())
    }

    pub(super) fn publish(&mut self) -> Result<(), MemoryObjectError> {
        crate::capability::publish_batch(&mut [crate::capability::Publication {
            destination: self.authority(),
            source: None,
        }])
        .map_err(capability_error)
    }

    pub(super) fn install(&mut self, registry: &mut MemoryObjectRegistry, payload: MemoryCap) {
        let owner = self.handle.id();
        if !registry.caps.contains_key(&owner) {
            registry.caps.insert(
                self.namespace.take().expect("prepared memory namespace missing"),
                owner,
                AddressSpaceCaps {
                    handle: self.handle,
                    caps: Caps::new(),
                },
            );
        }
        let namespace = registry.caps.get_mut(&owner).unwrap();
        assert_eq!(namespace.handle, self.handle, "captured memory namespace changed");
        namespace.caps.0.insert(
            self.payload.take().expect("prepared memory capability missing"),
            self.identity(),
            Record {
                payload,
                authority: None,
            },
        );
    }
}

/// The source-move case leaves authority in its containing SourceEscrow until
/// this payload node has been freed. Every other retirement pairs it here.
#[must_use]
pub(super) struct RetiredCapability(RetiredEntry<(MemoryObjectCap, Record)>);
impl core::fmt::Debug for RetiredCapability {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetiredCapability").finish_non_exhaustive()
    }
}
impl RetiredCapability {
    pub(super) fn release(self) {
        let mut authority = None;
        self.0.release_with(|(_, record)| authority = record.authority);
        if let Some(authority) = authority {
            authority.release();
        }
    }

    fn pair(&mut self, owner: AddressSpaceId, teardown: bool) {
        let (cap, record) = self.0.value_mut();
        assert!(record.authority.is_none());
        record.authority = Some(
            if teardown {
                crate::capability::detach_for_teardown(
                    owner,
                    *cap,
                    crate::capability::ObjectKind::Memory,
                )
            } else {
                crate::capability::detach(owner, *cap, crate::capability::ObjectKind::Memory)
            }
            .expect("memory payload capability was absent from unified table"),
        );
    }
}

pub(super) fn retire(
    registry: &mut MemoryObjectRegistry,
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
) -> RetiredCapability {
    let mut node = registry
        .caps
        .get_mut(&owner)
        .unwrap()
        .caps
        .take(&cap)
        .expect("validated memory capability missing");
    node.pair(owner, false);
    node
}

#[must_use]
pub(super) struct RetiredCapabilities(RetirementList<(MemoryObjectCap, Record)>);
impl core::fmt::Debug for RetiredCapabilities {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetiredCapabilities").finish_non_exhaustive()
    }
}
impl RetiredCapabilities {
    pub(super) const fn new() -> Self {
        Self(RetirementList::new())
    }

    pub(super) fn push(&mut self, node: RetiredCapability) {
        self.0.push(node.0);
    }

    pub(super) fn append(&mut self, mut other: Self) {
        while let Some(node) = other.0.pop() {
            self.0.push(node);
        }
    }

    pub(super) fn release(&mut self) {
        while let Some(node) = self.0.pop() {
            RetiredCapability(node).release();
        }
    }
}

/// Detachment transfers complete namespace/payload/authority storage without
/// allocator work. The caller retains its existing closing root/proof.
#[must_use]
pub(super) struct RetiredNamespace(RetiredEntry<(AddressSpaceId, AddressSpaceCaps)>);
impl RetiredNamespace {
    pub(super) fn release(mut self) {
        while let Some((&cap, _)) = self.0.value().1.caps.0.first_key_value() {
            self.0.value_mut().1.caps.take(&cap).unwrap().release();
        }
        self.0.release();
    }
}

pub(super) fn retire_namespace(
    registry: &mut MemoryObjectRegistry,
    owner: AddressSpaceId,
) -> Option<RetiredNamespace> {
    let mut namespace = registry.caps.take(&owner)?;
    for (cap, record) in namespace.value_mut().1.caps.0.iter_mut() {
        assert!(record.authority.is_none());
        record.authority = Some(
            crate::capability::detach_for_teardown(
                owner,
                *cap,
                crate::capability::ObjectKind::Memory,
            )
            .expect("memory payload capability was absent from unified table"),
        );
    }
    Some(RetiredNamespace(namespace))
}

pub(super) fn retire_object(
    registry: &mut MemoryObjectRegistry,
    object: MemoryObjectId,
) -> RetiredCapabilities {
    let mut retired = RetiredCapabilities::new();
    for (&owner, namespace) in registry.caps.iter_mut() {
        loop {
            let cap = namespace
                .caps
                .0
                .iter()
                .find_map(|(&cap, record)| (record.payload.object == object).then_some(cap));
            let Some(cap) = cap else {
                break;
            };
            let mut node = namespace.caps.take(&cap).unwrap();
            node.pair(owner, true);
            retired.push(node);
        }
    }
    retired
}
