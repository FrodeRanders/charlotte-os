//! Complete global namespace metadata preparation and post-guard retirement.
//! Captured identities are not root leases or physical cleanup receipts.

use super::*;

#[must_use = "finish unused namespace preparation after local guards leave"]
pub(in crate::completion) struct Preparing(ManuallyDrop<Resources>);

struct Resources {
    asid: AddressSpaceId,
    node: Option<PreparedEntry<(AddressSpaceId, Namespace)>>,
    entries: Option<Namespace>,
}

impl Preparing {
    pub(in crate::completion) fn new(
        asid: AddressSpaceId,
        capacity: usize,
    ) -> Result<Self, CqOpenError> {
        Self::new_with(asid, capacity, |_| Ok(()))
    }

    fn new_with(
        asid: AddressSpaceId,
        capacity: usize,
        mut before_allocate: impl FnMut(u8) -> Result<(), core::alloc::AllocError>,
    ) -> Result<Self, CqOpenError> {
        // Capture once before any allocation. None is confined to permanent
        // kernel/serialized raw fixtures, never a replacement user generation.
        let root = crate::memory::current_address_space_handle(asid);
        let mut preparing = Self(ManuallyDrop::new(Resources {
            asid,
            node: None,
            entries: None,
        }));
        let result = (|| {
            before_allocate(0)?;
            preparing.0.node = Some(PreparedEntry::try_new()?);
            preparing.0.entries = Some(Namespace::try_new(root, capacity, &mut before_allocate)?);
            Ok::<_, core::alloc::AllocError>(())
        })();
        if result.is_err() {
            preparing.finish();
            return Err(CqOpenError::AllocationFailed);
        }
        Ok(preparing)
    }

    pub(in crate::completion) fn is_published(&self) -> bool {
        self.0.entries.is_none()
    }

    pub(in crate::completion) fn entries(&self) -> &Namespace {
        self.0.entries.as_ref().expect("namespace preparation published twice")
    }

    pub(in crate::completion) fn entries_mut(&mut self) -> &mut Namespace {
        self.0.entries.as_mut().expect("namespace preparation published twice")
    }

    pub(in crate::completion) fn validate(&self) -> Result<(), CqOpenError> {
        if self.entries().address_space.is_some_and(|root| !crate::memory::budget::accepting(root))
        {
            return Err(CqOpenError::RetiringAddressSpace);
        }
        Ok(())
    }

    /// Caller validates the captured namespace under this same registry hold,
    /// before any borrowed physical-ring write. No later fallible admission.
    pub(in crate::completion) fn publish_into(
        &mut self,
        registry: &mut Registry,
    ) -> Option<RetiredNamespace> {
        let resources = &mut *self.0;
        let mut previous = registry.take(&resources.asid).map(RetiredNamespace::from_node);
        registry.insert(
            resources.node.take().unwrap(),
            resources.asid,
            resources.entries.take().unwrap(),
        );
        if let Some(previous) = &mut previous {
            previous.revoke_authority(resources.asid);
        }
        previous
    }

    pub(in crate::completion) fn publish(
        &mut self,
    ) -> Result<Option<RetiredNamespace>, CqOpenError> {
        let mut registry = COMPLETIONS.write();
        self.validate()?;
        Ok(self.publish_into(&mut registry))
    }

    pub(in crate::completion) fn finish(self) {
        let Resources {
            node,
            entries,
            ..
        } = ManuallyDrop::into_inner(self.0);
        // Even unused metadata leaves before the payload's original accounts.
        drop(node);
        if let Some(entries) = entries {
            entries.finish();
        }
    }
}

/// One shared node retains the entire namespace. Its fallback never frees
/// metadata, closes sources or refunds any sibling's original admission.
#[must_use = "finish the complete retired namespace after registry unlock"]
pub(in crate::completion) struct RetiredNamespace(RetiredEntry<(AddressSpaceId, Namespace)>);

impl RetiredNamespace {
    pub(in crate::completion) fn from_node(
        node: RetiredEntry<(AddressSpaceId, Namespace)>,
    ) -> Self {
        Self(node)
    }

    pub(in crate::completion) fn finish(self) {
        let mut entries = None;
        self.0.release_with(|(_, payload)| entries = Some(payload));
        // Global node deallocation precedes all payload/account disposal.
        entries.unwrap().finish();
    }
}

impl Deref for RetiredNamespace {
    type Target = Namespace;

    fn deref(&self) -> &Namespace {
        &self.0.value().1
    }
}

impl DerefMut for RetiredNamespace {
    fn deref_mut(&mut self) -> &mut Namespace {
        &mut self.0.value_mut().1
    }
}

pub(in crate::completion) fn open(
    asid: AddressSpaceId,
    capacity: usize,
) -> Result<(), CqOpenError> {
    let mut preparing = Preparing::new(asid, capacity)?;
    let result = preparing.publish();
    preparing.finish();
    if let Some(previous) = result? {
        previous.finish();
    }
    Ok(())
}

mod tests;

pub(in crate::completion) fn test_preparation() {
    tests::run();
}
