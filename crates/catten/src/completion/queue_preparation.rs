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
        entries: &AsCompletions,
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

    fn validate(&self, entries: &AsCompletions) -> Result<(), CqOpenError> {
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
pub(super) struct Prepared {
    target: Target,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
    state: Option<CqState>,
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
        )
    }

    fn new_with(
        target: Target,
        cq_entries: u32,
        frame: Option<crate::memory::physical::PAddr>,
        allocate_list: impl FnOnce(bool) -> Result<Waiters, RegistrationError>,
        allocate_ring: impl FnOnce(u32) -> Result<RingStorage, cq::CqRingError>,
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
        let charge = cq_budget::reserve(&target.account, target.platform, [1, bytes as u64])
            .map_err(|_| CqOpenError::ResourceLimit)?;
        let mut state = CqState {
            ring: core::ptr::null_mut(),
            ring_head: 0,
            ring_capacity: cq::CompletionQueueRing::capacity_for(cq_entries),
            backlog: VecDeque::new(),
            retained_limit: target.capacity,
            work_generation: 0,
            last_seen_generation: 0,
            waiters: allocate_list(target.platform).map_err(|_| CqOpenError::AllocationFailed)?,
            _buf: None,
            _storage_charge: charge,
        };
        state.backlog.try_reserve_exact(slots).map_err(|_| CqOpenError::AllocationFailed)?;
        if state.backlog.capacity() > slots {
            return Err(CqOpenError::AllocationFailed);
        }
        if frame.is_none() {
            let (buffer, ring) = allocate_ring(cq_entries).map_err(CqOpenError::Ring)?;
            state._buf = Some(buffer);
            state.ring = ring;
        }
        Ok(Self {
            target,
            cq_entries,
            frame,
            state: Some(state),
        })
    }

    fn initialize(&self, state: &mut CqState) -> Result<(), CqOpenError> {
        if let Some(frame) = self.frame {
            // Kernel mapping boundary: caller retains the writable frame and
            // quiescent consumer. Identity/retirement/alias checks precede all
            // writes, under the same registry hold as publication.
            state.ring = unsafe { cq::CompletionQueueRing::init_at_phys(frame, self.cq_entries) }
                .map_err(CqOpenError::Ring)?;
        }
        Ok(())
    }

    fn publish(&mut self, cq: CqId) -> Result<Option<CqState>, CqOpenError> {
        let mut registry = COMPLETIONS.write();
        let entries = registry.get(&self.target.asid).ok_or(CqOpenError::UnknownAddressSpace)?;
        self.target.validate(entries)?;
        check_ring_alias(&registry, self.target.asid, cq, self.frame)?;
        let mut state = self.state.take().expect("CQ preparation published twice");
        // Validation above plus construction preflight make physical init
        // infallible here. Retain the owner if that contract ever changes.
        if let Err(error) = self.initialize(&mut state) {
            self.state = Some(state);
            return Err(error);
        }
        Ok(replace_cq(registry.get_mut(&self.target.asid).unwrap(), cq, state))
    }

    /// The existing private detachment fixture stages real heap queues using
    /// this same constructor; physical states cannot escape uninitialized.
    pub(super) fn heap_for_namespace(
        asid: AddressSpaceId,
        entries: &AsCompletions,
        cq_entries: u32,
    ) -> CqState {
        let platform = crate::memory::budget::platform_identity(asid);
        let target = Target::from_namespace(asid, entries, platform).unwrap();
        Self::new(target, cq_entries, None).unwrap().state.take().unwrap()
    }
}

pub(super) fn attach(
    asid: AddressSpaceId,
    cq: CqId,
    cq_entries: u32,
    frame: Option<crate::memory::physical::PAddr>,
) -> Result<(), CqOpenError> {
    let mut prepared = Prepared::new(Target::capture(asid)?, cq_entries, frame)?;
    let previous = prepared.publish(cq)?;
    drop(previous);
    Ok(())
}

/// One owner retains the whole unpublished namespace and original queue
/// preparation. Its private CQ map node is prepared before COMPLETIONS too.
struct PreparedNamespace(ManuallyDrop<NamespacePreparation>);

struct NamespacePreparation {
    entries: Option<AsCompletions>,
    queue: Prepared,
}

impl PreparedNamespace {
    fn new(
        asid: AddressSpaceId,
        capacity: usize,
        cq_entries: u32,
        frame: Option<crate::memory::physical::PAddr>,
    ) -> Result<Self, CqOpenError> {
        let mut entries = empty_as(asid, capacity);
        let platform = crate::memory::budget::platform_identity(asid);
        let mut queue = match Target::from_namespace(asid, &entries, platform)
            .and_then(|target| Prepared::new(target, cq_entries, frame))
        {
            Ok(queue) => queue,
            Err(error) => {
                entries.finish();
                return Err(error);
            }
        };
        entries.cqs.insert(DEFAULT_CQ, queue.state.take().unwrap());
        Ok(Self(ManuallyDrop::new(NamespacePreparation {
            entries: Some(entries),
            queue,
        })))
    }

    fn publish(&mut self) -> Result<Option<AsCompletions>, CqOpenError> {
        let mut registry = COMPLETIONS.write();
        let resources = &mut *self.0;
        let entries = resources.entries.as_mut().expect("CQ namespace published twice");
        resources.queue.target.validate(entries)?;
        check_ring_alias(
            &registry,
            resources.queue.target.asid,
            DEFAULT_CQ,
            resources.queue.frame,
        )?;
        resources.queue.initialize(entries.cqs.get_mut(&DEFAULT_CQ).unwrap())?;
        let mut previous =
            registry.insert(resources.queue.target.asid, resources.entries.take().unwrap());
        if let Some(previous) = &mut previous {
            revoke_namespace_authority(resources.queue.target.asid, previous);
        }
        Ok(previous)
    }

    fn finish(self) {
        let mut resources = ManuallyDrop::into_inner(self.0);
        if let Some(entries) = resources.entries.take() {
            entries.finish();
        }
        drop(resources);
    }
}

pub(super) fn open_namespace(
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

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
