//! Serialized real-allocation pressure and exact-generation lifetime controls.
use super::*;
use crate::{
    completion::{
        self,
        COMPLETIONS,
        CqOpenError,
        DEFAULT_CQ,
        OpCode,
        RetiredCq,
        budget,
        cq_budget,
        namespace,
        watch_budget,
    },
    klib::observer::{
        CallOnNotify,
        Observer,
        registration::ObserverList,
    },
    memory::{
        self,
        AddressSpace,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

enum HeldAccount {
    Record(budget::AccountRef),
    Queue(cq_budget::AccountRef),
    Watch(watch_budget::AccountRef),
}

impl HeldAccount {
    fn try_new(kind: usize, platform: bool) -> Result<Self, AllocError> {
        match kind % 3 {
            0 => budget::DomainBudget::try_new(1, platform).map(Self::Record),
            1 => cq_budget::DomainBudget::try_new(platform).map(Self::Queue),
            _ => watch_budget::DomainBudget::try_new(1, platform).map(Self::Watch),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Record(account) => account.used() == 0,
            Self::Queue(account) => account.used() == [0, 0],
            Self::Watch(account) => account.used() == 0,
        }
    }
}

fn pressure() {
    let before = snapshot();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    completion::open_address_space_with_cq(root.id(), 4, 2).unwrap();
    let records = completion::record_admission(root.id()).unwrap();
    let queues = completion::cq_admission(root.id()).unwrap();
    let watches = completion::watch_admission(root.id()).unwrap();
    let cap = completion::submit(root.id(), OpCode::Read, Some(alloc::vec![7])).unwrap();
    let mut ordinary = alloc::vec::Vec::with_capacity(ORDINARY_LIMIT as usize);
    while let Ok(account) = HeldAccount::try_new(ordinary.len(), false) {
        assert!(account.is_empty());
        ordinary.push(account);
        assert!(ordinary.len() <= ORDINARY_LIMIT as usize);
    }
    assert_eq!(snapshot().1, ORDINARY_LIMIT);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            for kind in 0..3 {
                assert!(HeldAccount::try_new(kind, false).is_err());
            }
        }
    }
    // Zero, one or two available slots cannot publish a partial three-account
    // replacement. Independent timer-account preparation must also roll back.
    for available in 0..3 {
        for _ in 0..available {
            drop(ordinary.pop().unwrap());
        }
        let partial = snapshot();
        let timers = crate::timers::account_budget::snapshot();
        assert!(matches!(
            completion::open_address_space_with_cq(root.id(), 4, 2),
            Err(CqOpenError::AllocationFailed)
        ));
        assert_eq!(snapshot(), partial);
        assert_eq!(crate::timers::account_budget::snapshot(), timers);
        {
            let registry = COMPLETIONS.read();
            let entries = &registry[&root.id()];
            assert!(Arc::ptr_eq(&entries.record_budget, &records));
            assert!(Arc::ptr_eq(&entries.cq_budget, &queues));
            assert!(Arc::ptr_eq(&entries.watch_budget, &watches));
        }
        assert!(completion::poll(root.id(), cap).unwrap().is_none());
        assert!(completion::completion_of(root.id(), cap).unwrap().holds_buffer());
        for kind in 0..available {
            ordinary.push(HeldAccount::try_new(kind, false).unwrap());
        }
    }
    let full_ordinary = snapshot();
    memory::budget::mark_platform(root);
    assert_eq!(snapshot(), full_ordinary);
    let trusted =
        memory::register_user_address_space(AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    let queue_before = cq_budget::node_used();
    completion::open_address_space_with_cq(trusted.id(), 4, 2).unwrap();
    assert_eq!(snapshot(), (full_ordinary.0 + 3, full_ordinary.1));
    // Backing class grants no record/CQ/watch reserve privilege. Exercise the
    // production queue, submission and observation adapters before any grant.
    assert_eq!(cq_budget::node_used().0[0], queue_before.0[0] + 1);
    assert_eq!(cq_budget::node_used().1[0], queue_before.1[0] + 1);
    let record_before = budget::node_used();
    let watch_before = watch_budget::node_used();
    let pending = completion::submit(trusted.id(), OpCode::Read, None).unwrap();
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("account pressure fabricated completion"));
    let observation = completion::observe(trusted.id(), pending, observer).unwrap();
    assert_eq!(budget::node_used(), (record_before.0 + 1, record_before.1 + 1));
    assert_eq!(watch_budget::node_used(), (watch_before.0 + 1, watch_before.1 + 1));
    memory::close_user_address_space_handle(trusted).unwrap();
    drop(observation);
    assert_eq!(budget::node_used(), record_before);
    assert_eq!(watch_budget::node_used(), watch_before);
    assert_eq!(cq_budget::node_used(), queue_before);
    assert_eq!(snapshot(), full_ordinary);

    let mut reserved = alloc::vec::Vec::with_capacity((NODE_LIMIT - ORDINARY_LIMIT) as usize);
    while let Ok(account) = HeldAccount::try_new(reserved.len(), true) {
        assert!(account.is_empty());
        reserved.push(account);
        assert!(reserved.len() <= (NODE_LIMIT - ORDINARY_LIMIT) as usize);
    }
    let full = snapshot();
    assert_eq!(full.0, NODE_LIMIT);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for kind in 0..3 {
            assert!(HeldAccount::try_new(kind, true).is_err());
        }
    }
    assert!(matches!(
        namespace::Preparing::new(memory::KERNEL_ASID, 4),
        Err(CqOpenError::AllocationFailed)
    ));
    drop((records, queues, watches));
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(snapshot(), (full.0 - 3, full.1 - 3));
    let recovered =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    completion::open_address_space_with_cq(recovered.id(), 4, 2).unwrap();
    assert_eq!(snapshot(), full);
    memory::close_user_address_space_handle(recovered).unwrap();
    let counts = (ordinary.len(), reserved.len());
    drop((ordinary, reserved));
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[completion account pressure] {} ordinary + {} reserved actual mixed accounts; {}/{} \
         ceilings, 192 heap-held denials, zero/one/two-slot namespace rollback, trusted backing \
         without record/CQ/watch privilege, immutable promotion and close/recovery passed",
        counts.0,
        counts.1,
        ORDINARY_LIMIT,
        NODE_LIMIT
    );
}

fn retained_generation() {
    let before = snapshot();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    completion::open_address_space_with_cq(root.id(), 4, 2).unwrap();
    let records = completion::record_admission(root.id()).unwrap();
    let queues = completion::cq_admission(root.id()).unwrap();
    let watches = completion::watch_admission(root.id()).unwrap();
    let record_weak = Arc::downgrade(&records);
    let queue_weak = Arc::downgrade(&queues);
    let watch_weak = Arc::downgrade(&watches);
    let cap = completion::submit(root.id(), OpCode::Read, Some(alloc::vec![9])).unwrap();
    let captured = completion::completion_of(root.id(), cap).unwrap();
    let completion_weak = Arc::downgrade(&captured);
    let source = ObserverList::try_new(1, false).unwrap();
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("retirement fabricated watch notification"));
    let token = source
        .register(Arc::downgrade(&observer), watch_budget::reserve(&watches, false).unwrap())
        .unwrap();
    // This private heap queue has no consumer or physical mapping. Detach its
    // complete typed owners under the existing registry, before real root close.
    let mut retired = {
        let mut registry = COMPLETIONS.write();
        let mut owner = namespace::RetiredNamespace::from_node(registry.take(&root.id()).unwrap());
        owner.revoke_authority(root.id());
        owner
    };
    let queue = RetiredCq::from_node(retired.cqs.take(&DEFAULT_CQ).unwrap());
    memory::close_user_address_space_handle(root).unwrap();
    let successor =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(root.id(), successor.id());
    assert_ne!(root.generation(), successor.generation());
    completion::open_address_space_with_cq(successor.id(), 4, 2).unwrap();
    retired.finish();
    assert_eq!(records.used(), 1);
    assert_eq!(queues.used()[0], 1);
    assert_eq!(watches.used(), 1);
    assert_eq!(snapshot(), (before.0 + 6, before.1 + 6));
    drop((records, queues, watches));
    drop(captured);
    assert!(completion_weak.upgrade().is_none());
    assert_eq!(record_weak.upgrade().unwrap().used(), 1);
    drop(completion_weak);
    queue.finish();
    drop(token);
    assert!(record_weak.upgrade().is_none());
    assert!(queue_weak.upgrade().is_none());
    assert!(watch_weak.upgrade().is_none());
    assert_eq!(snapshot(), (before.0 + 6, before.1 + 6));
    drop(record_weak);
    assert_eq!(snapshot(), (before.0 + 5, before.1 + 5));
    drop(queue_weak);
    assert_eq!(snapshot(), (before.0 + 4, before.1 + 4));
    drop(watch_weak);
    assert_eq!(snapshot(), (before.0 + 3, before.1 + 3));
    {
        let registry = COMPLETIONS.read();
        let entries = &registry[&successor.id()];
        assert_eq!(entries.record_budget.used(), 0);
        assert_eq!(entries.cq_budget.used()[0], 1);
        assert_eq!(entries.watch_budget.used(), 0);
    }
    memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[completion account lifetime] real completion/weak backing, retired heap CQ and owned \
         watch entry retained original accounts across root close and exact ASID reuse; three \
         final-weak refunds left successor unchanged"
    );
}

pub(super) fn run() {
    pressure();
    retained_generation();
}
