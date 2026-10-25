//! CQ payload preparation and partial-error disposal outside COMPLETIONS.
//! Physical frames remain borrowed from their mapping/quiescent caller.

use super::*;
use crate::klib::observer::{
    registration::{
        ListRef,
        ObserverList,
        RegistrationError,
    },
    waiter_budget,
};

type Waiters = ListRef<waiter_budget::Charge>;
type RingStorage = (Vec<u64>, *mut cq::CompletionQueueRing);

struct Target {
    asid: AddressSpaceId,
    root: Option<crate::memory::AddressSpaceHandle>,
    account: Arc<cq_budget::DomainBudget>,
    capacity: usize,
    platform: bool,
}

impl Target {
    fn from_namespace(
        asid: AddressSpaceId,
        entries: &Namespace,
        platform: Option<crate::memory::AddressSpaceHandle>,
    ) -> Result<Self, CqOpenError> {
        if entries.address_space.is_some_and(|root| !crate::memory::budget::accepting(root)) {
            return Err(CqOpenError::RetiringAddressSpace);
        }
        Ok(Self {
            asid,
            root: entries.address_space,
            account: entries.cq_budget.clone(),
            capacity: entries.capacity,
            platform: asid == crate::memory::KERNEL_ASID
                || platform.is_some_and(|root| Some(root) == entries.address_space),
        })
    }

    fn capture(asid: AddressSpaceId) -> Result<Self, CqOpenError> {
        let platform = crate::memory::budget::platform_identity(asid);
        let registry = COMPLETIONS.read();
        Self::from_namespace(
            asid,
            registry.get(&asid).ok_or(CqOpenError::UnknownAddressSpace)?,
            platform,
        )
    }

    fn validate(&self, entries: &Namespace) -> Result<(), CqOpenError> {
        if self.root != entries.address_space
            || !Arc::ptr_eq(&self.account, &entries.cq_budget)
            || self.capacity != entries.capacity
        {
            return Err(CqOpenError::UnknownAddressSpace);
        }
        if self.root.is_some_and(|root| !crate::memory::budget::accepting(root)) {
            return Err(CqOpenError::RetiringAddressSpace);
        }
        Ok(())
    }
}

/// Borrow publication so every rejected payload stays owned until guard exit.
/// A physical state's null ring is private until serialized initialization.
#[must_use]
pub(in crate::completion) struct Prepared(ManuallyDrop<Resources>);

struct Resources {
    target: Target,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
    state: Option<CqState>,
    node: Option<PreparedEntry<(CqId, CqState)>>,
    charge: Option<cq_budget::Charge>,
}

impl Prepared {
    fn new(
        target: Target,
        cq_entries: u32,
        frame: Option<crate::memory::physical::PAddr>,
    ) -> Result<Self, CqOpenError> {
        Self::new_with(
            target,
            cq_entries,
            frame,
            |platform| ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform),
            cq::CompletionQueueRing::new_page,
            PreparedEntry::try_new,
        )
    }

    fn new_with(
        target: Target,
        cq_entries: u32,
        frame: Option<crate::memory::physical::PAddr>,
        allocate_list: impl FnOnce(bool) -> Result<Waiters, RegistrationError>,
        allocate_ring: impl FnOnce(u32) -> Result<RingStorage, cq::CqRingError>,
        allocate_node: impl FnOnce() -> Result<PreparedEntry<(CqId, CqState)>, core::alloc::AllocError>,
    ) -> Result<Self, CqOpenError> {
        if cq_entries < 2 {
            return Err(CqOpenError::Ring(cq::CqRingError::CapacityTooSmall));
        }
        if frame.is_some_and(|frame| {
            !crate::cpu::isa::interface::memory::address::Address::is_aligned_to(&frame, 4096)
        }) {
            return Err(CqOpenError::Ring(cq::CqRingError::FrameMisaligned));
        }
        let slots = charlotte_lifecycle::resources::queue_backing_slots(target.capacity)
            .ok_or(CqOpenError::ResourceLimit)?;
        let bytes = slots
            .checked_mul(core::mem::size_of::<BacklogEntry>())
            .and_then(|bytes| {
                bytes.checked_add(
                    if frame.is_none() {
                        4096
                    } else {
                        0
                    },
                )
            })
            .ok_or(CqOpenError::ResourceLimit)?;
        let mut prepared = Self(ManuallyDrop::new(Resources {
            target,
            cq_entries,
            frame,
            state: None,
            node: None,
            charge: None,
        }));
        let result = (|| {
            let resources = &mut *prepared.0;
            resources.charge = Some(
                cq_budget::reserve(
                    &resources.target.account,
                    resources.target.platform,
                    [1, bytes as u64],
                )
                .map_err(|_| CqOpenError::ResourceLimit)?,
            );
            resources.node = Some(allocate_node().map_err(|_| CqOpenError::AllocationFailed)?);
            resources.state = Some(CqState {
                ring: core::ptr::null_mut(),
                ring_head: 0,
                ring_capacity: cq::CompletionQueueRing::capacity_for(cq_entries),
                backlog: VecDeque::new(),
                retained_limit: resources.target.capacity,
                work_generation: 0,
                last_seen_generation: 0,
                waiters: allocate_list(resources.target.platform)
                    .map_err(|_| CqOpenError::AllocationFailed)?,
                _buf: None,
                _storage_charge: resources.charge.take().unwrap(),
            });
            let state = resources.state.as_mut().unwrap();
            state.backlog.try_reserve_exact(slots).map_err(|_| CqOpenError::AllocationFailed)?;
            if state.backlog.capacity() > slots {
                return Err(CqOpenError::AllocationFailed);
            }
            if frame.is_none() {
                let (buffer, ring) = allocate_ring(cq_entries).map_err(CqOpenError::Ring)?;
                state._buf = Some(buffer);
                state.ring = ring;
            }
            Ok(())
        })();
        if let Err(error) = result {
            prepared.finish();
            return Err(error);
        }
        Ok(prepared)
    }

    fn initialize(
        frame: Option<crate::memory::physical::PAddr>,
        cq_entries: u32,
        state: &mut CqState,
    ) -> Result<(), CqOpenError> {
        if let Some(frame) = frame {
            // The caller retains the frame and quiescent consumer. Exact
            // identity/retirement/alias validation precedes every frame write.
            state.ring = unsafe { cq::CompletionQueueRing::init_at_phys(frame, cq_entries) }
                .map_err(CqOpenError::Ring)?;
        }
        Ok(())
    }

    fn publish(&mut self, cq: CqId) -> Result<Option<RetiredCq>, CqOpenError> {
        let mut registry = COMPLETIONS.write();
        let resources = &mut *self.0;
        let entries =
            registry.get(&resources.target.asid).ok_or(CqOpenError::UnknownAddressSpace)?;
        resources.target.validate(entries)?;
        check_ring_alias(&registry, resources.target.asid, cq, resources.frame)?;
        // Keep both fields in their owner even if physical init rejects.
        Self::initialize(
            resources.frame,
            resources.cq_entries,
            resources.state.as_mut().expect("CQ preparation published twice"),
        )?;
        Ok(replace_cq(
            registry.get_mut(&resources.target.asid).unwrap(),
            cq,
            resources.node.take().unwrap(),
            resources.state.take().unwrap(),
        ))
    }

    fn finish(self) {
        let mut resources = ManuallyDrop::into_inner(self.0);
        // Empty metadata frees before either provisional or payload admission.
        drop(resources.node.take());
        drop(resources);
    }

    /// The existing private detachment fixture stages real heap queues using
    /// this same constructor; physical states cannot escape uninitialized.
    pub(in crate::completion) fn heap_for_namespace(
        asid: AddressSpaceId,
        entries: &Namespace,
        cq_entries: u32,
    ) -> (PreparedEntry<(CqId, CqState)>, CqState) {
        let platform = crate::memory::budget::platform_identity(asid);
        let target = Target::from_namespace(asid, entries, platform).unwrap();
        let mut prepared = Self::new(target, cq_entries, None).unwrap();
        let result = (prepared.0.node.take().unwrap(), prepared.0.state.take().unwrap());
        prepared.finish();
        result
    }
}

pub(in crate::completion) fn attach(
    asid: AddressSpaceId,
    cq: CqId,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<(), CqOpenError> {
    let mut prepared = Prepared::new(Target::capture(asid)?, cq_entries, frame)?;
    let result = prepared.publish(cq);
    prepared.finish();
    if let Some(previous) = result? {
        previous.finish();
    }
    Ok(())
}

/// Complete detached queue metadata/payload. Shared RetiredEntry fallback
/// retains source, ring/backlog, original charge and node without cleanup.
#[must_use = "finish the complete retired queue after local registry unlock"]
pub(in crate::completion) struct RetiredCq(RetiredEntry<(CqId, CqState)>);

impl RetiredCq {
    pub(in crate::completion) fn from_node(node: RetiredEntry<(CqId, CqState)>) -> Self {
        Self(node)
    }

    pub(in crate::completion) fn finish(self) {
        let mut state = None;
        self.0.release_with(|(_, payload)| state = Some(payload));
        // The node has freed; payload/source/backing still own their charge.
        drop(state);
    }
}

/// One owner retains the whole unpublished namespace and original queue
/// preparation. Its private CQ map node is prepared before COMPLETIONS too.
struct PreparedNamespace(ManuallyDrop<NamespacePreparation>);

struct NamespacePreparation {
    namespace: namespace::Preparing,
    queue: Prepared,
}

impl PreparedNamespace {
    fn new(
        asid: AddressSpaceId,
        capacity: usize,
        cq_entries: u32,
        frame: Option<crate::memory::physical::PAddr>,
    ) -> Result<Self, CqOpenError> {
        let mut namespace = namespace::Preparing::new(asid, capacity)?;
        let platform = crate::memory::budget::platform_identity(asid);
        let mut queue = match Target::from_namespace(asid, namespace.entries(), platform)
            .and_then(|target| Prepared::new(target, cq_entries, frame))
        {
            Ok(queue) => queue,
            Err(error) => {
                namespace.finish();
                return Err(error);
            }
        };
        namespace.entries_mut().cqs.insert(
            queue.0.node.take().unwrap(),
            DEFAULT_CQ,
            queue.0.state.take().unwrap(),
        );
        Ok(Self(ManuallyDrop::new(NamespacePreparation {
            namespace,
            queue,
        })))
    }

    fn publish(&mut self) -> Result<Option<RetiredNamespace>, CqOpenError> {
        let mut registry = COMPLETIONS.write();
        let resources = &mut *self.0;
        resources.queue.0.target.validate(resources.namespace.entries())?;
        check_ring_alias(
            &registry,
            resources.queue.0.target.asid,
            DEFAULT_CQ,
            resources.queue.0.frame,
        )?;
        Prepared::initialize(
            resources.queue.0.frame,
            resources.queue.0.cq_entries,
            resources.namespace.entries_mut().cqs.get_mut(&DEFAULT_CQ).unwrap(),
        )?;
        Ok(resources.namespace.publish_into(&mut registry))
    }

    fn finish(self) {
        let NamespacePreparation {
            namespace,
            queue,
        } = ManuallyDrop::into_inner(self.0);
        namespace.finish();
        queue.finish();
    }
}

pub(in crate::completion) fn open_namespace(
    asid: AddressSpaceId,
    capacity: usize,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<(), CqOpenError> {
    let mut prepared = PreparedNamespace::new(asid, capacity, cq_entries, frame)?;
    let result = prepared.publish();
    prepared.finish();
    let previous = result?;
    if let Some(previous) = previous {
        previous.finish();
    }
    Ok(())
}

mod metadata_tests;
mod tests;

pub(in crate::completion) fn test_preparation() {
    tests::run();
    metadata_tests::run();
    namespace::test_preparation();
}
