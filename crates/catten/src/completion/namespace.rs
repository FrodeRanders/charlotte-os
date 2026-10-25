//! Complete completion-namespace lifetime: prepared accounts/storage,
//! authority retirement and post-registry payload disposal. NamespaceState is
//! the shared view for existing serialized cross-family publication; the owner
//! retains its entire payload/accounts on abandonment. Identities are not leases.
use core::{
    mem::ManuallyDrop,
    ops::{
        Deref,
        DerefMut,
    },
};

use super::*;

mod preparation;
pub(super) use preparation::{
    Preparing,
    RetiredNamespace,
    test_preparation,
};
pub(super) type Registry = AdmittedMap<AddressSpaceId, Namespace>;

/// Complete namespace fallback retains all payloads and original accounts.
/// Only explicit post-registry finish may dispose sources, nodes and backing.
#[must_use = "explicitly finish the complete namespace after registry unlock"]
pub(super) struct Namespace(ManuallyDrop<NamespaceState>);

pub(super) struct NamespaceState {
    pub(super) table: AdmittedMap<CompletionCap, CapabilityRecord>,
    /// Shared upper bound for capability records, in-flight detached
    /// operations, and completed detached records awaiting CQ delivery.
    pub(super) capacity: usize,
    pub(super) live: usize,
    /// Live capability-free operations, keyed by their stable operation id.
    /// These count toward `capacity` exactly like capability-backed ones.
    /// Their admitted nodes transfer to terminal/backlog/refund owners.
    pub(super) detached: AdmittedMap<OperationId, DetachedOp>,
    /// The address space's completion queues, keyed by [`CqId`].
    pub(super) cqs: AdmittedMap<CqId, CqState>,
    /// Separate lifetime budget: a cancelled timer on another LP stays charged
    /// even after its completion slot has been reclaimed.
    pub(super) timer_budget: Arc<crate::timers::budget::DomainBudget>,
    pub(super) record_budget: Arc<budget::DomainBudget>,
    pub(super) cq_budget: Arc<cq_budget::DomainBudget>,
    pub(super) watch_budget: Arc<watch_budget::DomainBudget>,
    pub(super) address_space: Option<crate::memory::AddressSpaceHandle>,
}

impl Deref for Namespace {
    type Target = NamespaceState;

    fn deref(&self) -> &NamespaceState {
        &self.0
    }
}
impl DerefMut for Namespace {
    fn deref_mut(&mut self) -> &mut NamespaceState {
        &mut self.0
    }
}

impl Namespace {
    // Mandatory private boot fixture; runtime publication uses fallible Preparing.
    pub(super) fn for_fixture(asid: AddressSpaceId, capacity: usize) -> Self {
        Self::try_new(crate::memory::current_address_space_handle(asid), capacity, &mut |_| Ok(()))
            .expect("mandatory completion namespace allocation failed")
    }

    fn try_new(
        root: Option<crate::memory::AddressSpaceHandle>,
        capacity: usize,
        before_allocate: &mut impl FnMut(u8) -> Result<(), core::alloc::AllocError>,
    ) -> Result<Self, core::alloc::AllocError> {
        let capacity = capacity.min(budget::MAX_DOMAIN_RECORDS);
        // No payload/source/authority is published. Partial account construction
        // releases only unused Arcs outside local registry/lifecycle guards.
        before_allocate(1)?;
        let timer_budget = crate::timers::budget::DomainBudget::try_new(capacity)?;
        before_allocate(2)?;
        let record_budget = budget::DomainBudget::try_new(capacity)?;
        before_allocate(3)?;
        let cq_budget = cq_budget::DomainBudget::try_new()?;
        before_allocate(4)?;
        let watch_budget = watch_budget::DomainBudget::try_new(capacity)?;
        Ok(Self(ManuallyDrop::new(NamespaceState {
            table: AdmittedMap::new(),
            capacity,
            live: 0,
            detached: AdmittedMap::new(),
            cqs: AdmittedMap::new(),
            timer_budget,
            record_budget,
            cq_budget,
            watch_budget,
            address_space: root,
        })))
    }

    /// Retain this whole namespace and the caller's registry hold through
    /// authority removal; dispose payload/source metadata only after unlock.
    pub(super) fn revoke_authority(&mut self, asid: AddressSpaceId) {
        for (cap, record) in self.table.iter_mut() {
            record.revoke(asid, *cap);
        }
    }

    pub(super) fn finish(self) {
        let mut entries = ManuallyDrop::into_inner(self.0);
        // Public watches are revoked even if a kernel waiter retains their
        // completion object. Only independent list locks are entered here.
        for completion in entries.table.values() {
            completion.release_event_observation();
            drop(completion.waiters.close());
            drop(completion.detach_callbacks());
        }
        while let Some((&cap, _)) = entries.table.first_key_value() {
            RetiredCapability::from_node(entries.table.take(&cap).unwrap()).finish();
        }
        while let Some((&operation, _)) = entries.detached.first_key_value() {
            dispose_detached_node(entries.detached.take(&operation).unwrap());
        }
        while let Some((&cq, _)) = entries.cqs.first_key_value() {
            RetiredCq::from_node(entries.cqs.take(&cq).unwrap()).finish();
        }
        drop(entries);
    }
}

// SAFETY: `CqState::ring` has two backing modes. Heap-backed queues retain their
// aligned allocation in `_buf: Some(Vec<u64>)`. Physical queues use `_buf: None`; their
// frame is owned by the user address-space mapping, and address-space teardown
// calls `completion::close_address_space` before removing that mapping/table.
// Consequently either backing allocation outlives its registry entry.
//
// All safe kernel dereferences of `ring` and mutations of CQ producer state are
// serialized by the `COMPLETIONS` write lock. The mapped EL0 consumer may update
// the ring tail concurrently; the ring implementation uses volatile accesses
// and acquire/release fences for that producer/consumer protocol. The only raw
// pointer escape, `cq_ring_of`, is unsafe and places its quiescence/lifetime
// obligation on its test/inspection caller. These invariants make moving or
// sharing the registry between kernel threads sound.
unsafe impl Send for Namespace {}
unsafe impl Sync for Namespace {}

/// Mandatory kernel/demo/serialized-fixture adapter. Runtime domain setup uses
/// the fallible CQ namespace entry points below; all share prepared storage.
pub fn open_address_space(asid: AddressSpaceId, capacity: usize) {
    preparation::open(asid, capacity).expect("mandatory completion namespace publication failed");
}

/// Closes an address space's capability table and frees its CQ ring (if any).
pub fn close_address_space(asid: AddressSpaceId) {
    let retired = {
        let mut registry = COMPLETIONS.write();
        let mut retired = registry.take(&asid).map(RetiredNamespace::from_node);
        if let Some(retired) = &mut retired {
            retired.revoke_authority(asid);
        }
        retired
    };
    if let Some(retired) = retired {
        retired.finish();
    }
}
