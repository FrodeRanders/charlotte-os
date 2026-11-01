//! Serialized pre-scheduler fixtures use exact private namespaces and owners.
//! None substitutes a live root, resets counters or establishes physical proof.

use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    klib::observer::{
        CallOnNotify,
        WaitSponsor,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const CLIENT: usize = 0x5f00;

fn timer_account_pressure() {
    use crate::{
        memory::{
            self,
            AddressSpace,
            AddressSpaceRegistrationError,
            PHYSICAL_FRAME_ALLOCATOR,
        },
        timers::{
            account_budget,
            budget::DomainBudget,
        },
    };

    let before = account_budget::snapshot();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    open_address_space_with_cq(root.id(), 4, 2).unwrap();
    let original = COMPLETIONS.read()[&root.id()].timer_budget.clone();
    let mut ordinary = Vec::with_capacity(account_budget::NODE_LIMIT as usize);
    while let Ok(account) = DomainBudget::try_new(1, false) {
        ordinary.push(account);
        assert!(ordinary.len() <= account_budget::NODE_LIMIT as usize);
    }
    assert_eq!(account_budget::snapshot().1, account_budget::ORDINARY_LIMIT);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(DomainBudget::try_new(1, false).is_err());
        }
    }
    assert!(matches!(
        open_address_space_with_cq(root.id(), 4, 2),
        Err(CqOpenError::AllocationFailed)
    ));
    assert!(Arc::ptr_eq(&COMPLETIONS.read()[&root.id()].timer_budget, &original));
    drop(original);
    let full_ordinary = account_budget::snapshot();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let roots = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
    assert!(matches!(
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()),
        Err(AddressSpaceRegistrationError::MemorySponsorAllocationFailed)
    ));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    assert_eq!(
        memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(),
        roots
    );
    assert_eq!(account_budget::snapshot(), full_ordinary);
    memory::budget::mark_platform(root);
    assert_eq!(account_budget::snapshot(), full_ordinary);

    let trusted =
        memory::register_user_address_space(AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    open_address_space_with_cq(trusted.id(), 4, 2).unwrap();
    assert_eq!(account_budget::snapshot(), (full_ordinary.0 + 2, full_ordinary.1));
    // Trusted root backing alone does not grant either kind of timer event
    // platform privilege. Exercise the actual completion submission adapter.
    let events = crate::timers::budget::node_used();
    let scheduler_event = memory::budget::timer_sponsor(trusted.id()).reserve().unwrap();
    submit_timer(trusted.id(), u64::MAX).unwrap();
    assert_eq!(crate::timers::budget::node_used(), (events.0 + 2, events.1 + 2));
    drop(scheduler_event);
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(crate::timers::budget::node_used(), events);
    assert_eq!(account_budget::snapshot(), full_ordinary);

    let mut reserved = Vec::with_capacity(account_budget::NODE_LIMIT as usize);
    while let Ok(account) = DomainBudget::try_new(1, true) {
        reserved.push(account);
        assert!(reserved.len() <= account_budget::NODE_LIMIT as usize);
    }
    let full = account_budget::snapshot();
    assert_eq!(full.0, account_budget::NODE_LIMIT);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(DomainBudget::try_new(1, true).is_err());
    }
    let retained = Preparing::new(crate::memory::KERNEL_ASID, 4);
    assert!(matches!(retained, Err(CqOpenError::AllocationFailed)));
    // Both original accounts were ordinary even though the root was promoted.
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(account_budget::snapshot(), (full.0 - 2, full.1 - 2));
    let recovered =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    open_address_space_with_cq(recovered.id(), 4, 2).unwrap();
    memory::close_user_address_space_handle(recovered).unwrap();
    let counts = (ordinary.len(), reserved.len());
    drop(reserved);
    drop(ordinary);
    assert_eq!(account_budget::snapshot(), before);
    crate::logln!(
        "[timer account pressure] {} ordinary + {} reserved actual accounts; {}/{} ceilings, \
         heap-held denial, trusted root/namespace backing without event privilege, failed root \
         rollback, immutable promotion and close/recovery at total exhaustion passed",
        counts.0,
        counts.1,
        account_budget::ORDINARY_LIMIT,
        account_budget::NODE_LIMIT
    );
}

fn with_queue(asid: usize) -> Preparing {
    let mut preparing = Preparing::new(asid, 4).unwrap();
    let queue = delivery::Prepared::heap_for_namespace(asid, preparing.entries(), 2);
    preparing.entries_mut().cqs.insert(queue.0, DEFAULT_CQ, queue.1);
    preparing
}

fn allocation_and_fresh_nodes() {
    let backing = account_budget::snapshot();
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let account = cq_admission(CLIENT).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![3])).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let original = account.used();
    let timer_accounts = crate::timers::account_budget::snapshot();
    for rejected_stage in 0..5 {
        let mut visits = 0;
        let failed = Preparing::new_with(CLIENT, 4, |stage| {
            let registry = COMPLETIONS.write();
            assert!(Arc::ptr_eq(&registry[&CLIENT].cq_budget, &account));
            assert_eq!(registry[&CLIENT].live, 1);
            assert_eq!(stage, visits);
            visits += 1;
            if stage == rejected_stage {
                Err(core::alloc::AllocError)
            } else {
                Ok(())
            }
        });
        assert!(matches!(failed, Err(CqOpenError::AllocationFailed)));
        assert_eq!(visits, rejected_stage + 1);
        assert_eq!(crate::timers::account_budget::snapshot(), timer_accounts);
        assert_eq!(account_budget::snapshot(), (backing.0 + 3, backing.1 + 3));
        assert_eq!(account.used(), original);
        assert_eq!(records.used(), 1);
        assert!(captured.holds_buffer());
        assert!(crate::capability::contains(
            CLIENT,
            cap,
            crate::capability::ObjectKind::Completion
        ));
    }
    let mut visits = 0;
    let preparing = Preparing::new_with(CLIENT, 4, |stage| {
        let _registry = COMPLETIONS.write();
        assert_eq!(stage, visits);
        visits += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(visits, 5);
    preparing.finish();
    close_address_space(CLIENT);
    drop(captured);
    assert_eq!(records.used(), 0);
    crate::capability::close_address_space(CLIENT);

    let baseline = cq_budget::node_used();
    let mut preparing = Vec::with_capacity(64);
    let mut accounts = Vec::with_capacity(64);
    let mut retired = Vec::with_capacity(64);
    for i in 0..64 {
        let next = with_queue(CLIENT + i);
        accounts.push(next.entries().cq_budget.clone());
        preparing.push(next);
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        crate::timers::account_budget::with_locked(|| {
            account_budget::with_locked(|| {
                for next in &mut preparing {
                    assert!(next.publish().unwrap().is_none());
                    assert!(next.is_published());
                }
                let mut registry = COMPLETIONS.write();
                for (i, account) in accounts.iter().enumerate() {
                    retired
                        .push(RetiredNamespace::from_node(registry.take(&(CLIENT + i)).unwrap()));
                    assert_eq!(account.used()[0], 1);
                }
            });
        });
    }
    for next in preparing {
        next.finish();
    }
    assert!(accounts.iter().all(|account| account.used()[0] == 1));
    for next in retired {
        next.finish();
    }
    assert!(accounts.iter().all(|account| account.used() == [0, 0]));
    assert_eq!(cq_budget::node_used(), baseline);
    drop((accounts, account, records));
    assert_eq!(account_budget::snapshot(), backing);
    assert_eq!(
        crate::timers::account_budget::snapshot(),
        (timer_accounts.0 - 1, timer_accounts.1 - 1)
    );
}

struct Reenter(Arc<AtomicUsize>);

impl Observer for Reenter {
    fn notify(self: Arc<Self>) {
        panic!("namespace disposal fabricated completion");
    }
}

impl Drop for Reenter {
    fn drop(&mut self) {
        let _registry = COMPLETIONS.write();
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn complete_replacement() {
    let backing = crate::timers::account_budget::snapshot();
    let account_backing = account_budget::snapshot();
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let queues = cq_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
    let timer_weak = Arc::downgrade(&timers);
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![7])).unwrap();
    let timer = submit_timer(CLIENT, u64::MAX).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let weak = Arc::downgrade(&captured);
    let hits = Arc::new(AtomicUsize::new(0));
    let observer: Arc<dyn Observer> = Arc::new(Reenter(hits.clone()));
    let source = crate::klib::observer::registration::ObserverList::try_new(1, false).unwrap();
    let token = source
        .register(Arc::downgrade(&observer), watch_budget::reserve(&watches, false).unwrap())
        .unwrap();
    assert!(!captured.set_event_observation(observer, token).finish());
    for cookie in 0..2 {
        let operation = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, cookie).unwrap();
        complete_detached(CLIENT, operation, OpResult::Ok(1)).unwrap();
    }
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("namespace disposal fabricated CQ readiness"));
    let old_source = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let old_token =
        sponsor.register(old_source.waiter_source(), Arc::downgrade(&observer)).unwrap();
    let ring = unsafe { cq_ring_of(CLIENT, DEFAULT_CQ).unwrap() };
    let mut next = with_queue(CLIENT);
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired = next.publish().unwrap().unwrap();
        assert!(!crate::capability::contains(
            CLIENT,
            cap,
            crate::capability::ObjectKind::Completion
        ));
        assert!(!crate::capability::contains(
            CLIENT,
            timer,
            crate::capability::ObjectKind::Completion
        ));
        assert_eq!(records.used(), 3);
        assert_eq!(queues.used()[0], 1);
        assert_eq!(watches.used(), 1);
        assert_eq!(timers.used(), 1);
        assert_eq!(sponsor.used(), 1);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // This private fixture owns the only old-ring consumer. No physical
        // frame is adopted and the retired owner still retains heap backing.
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 0);
    }
    next.finish();
    crate::capability::close_address_space(CLIENT);
    let successor = submit(CLIENT, OpCode::Nop, None).unwrap();
    assert_eq!(successor, cap);
    let next_records = record_admission(CLIENT).unwrap();
    let next_queues = cq_admission(CLIENT).unwrap();
    let next_source = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let next_token =
        sponsor.register(next_source.waiter_source(), Arc::downgrade(&observer)).unwrap();
    retired.finish();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(source.registered(), 0);
    assert_eq!(watches.used(), 0);
    assert_eq!(queues.used(), [0, 0]);
    assert_eq!(timers.used(), 0);
    assert_eq!(records.used(), 1);
    assert_eq!(next_records.used(), 1);
    assert_eq!(next_queues.used()[0], 1);
    assert_eq!(sponsor.used(), 1);
    assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 0);
    assert!(crate::capability::contains(
        CLIENT,
        successor,
        crate::capability::ObjectKind::Completion
    ));
    assert!(captured.holds_buffer());
    drop(captured);
    assert!(weak.upgrade().is_none());
    assert_eq!(records.used(), 1);
    drop(weak);
    assert_eq!(records.used(), 0);
    drop((old_token, next_token));
    assert_eq!(sponsor.used(), 0);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    assert_eq!(next_records.used(), 0);
    assert_eq!(next_queues.used(), [0, 0]);
    drop(timers);
    assert!(timer_weak.upgrade().is_none());
    assert_eq!(crate::timers::account_budget::snapshot(), (backing.0 + 1, backing.1 + 1));
    drop(timer_weak);
    assert_eq!(crate::timers::account_budget::snapshot(), backing);
    drop((records, queues, watches, next_records, next_queues));
    assert_eq!(account_budget::snapshot(), account_backing);
}

fn retired_identity() {
    let backing = account_budget::snapshot();
    let root = crate::service::loader::create_user_address_space_handle();
    let mut preparing = Preparing::new(root.id(), 4).unwrap();
    let old_account = Arc::downgrade(&preparing.entries().cq_budget);
    crate::memory::close_user_address_space_handle(root).unwrap();
    let successor = crate::service::loader::create_user_address_space_handle();
    assert_eq!(successor.id(), root.id());
    assert_ne!(successor.generation(), root.generation());
    open_address_space_with_cq(successor.id(), 4, 2).unwrap();
    let current = cq_admission(successor.id()).unwrap();
    let before = current.used();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(preparing.publish(), Err(CqOpenError::RetiringAddressSpace)));
        assert!(!preparing.is_published());
        assert!(Arc::ptr_eq(&COMPLETIONS.read()[&successor.id()].cq_budget, &current));
        assert_eq!(current.used(), before);
    }
    preparing.finish();
    assert!(old_account.upgrade().is_none());
    assert_eq!(account_budget::snapshot(), (backing.0 + 4, backing.1 + 4));
    assert_eq!(current.used(), before);
    crate::memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(current.used(), [0, 0]);
    drop(old_account);
    assert_eq!(account_budget::snapshot(), (backing.0 + 1, backing.1 + 1));
    drop(current);
    assert_eq!(account_budget::snapshot(), backing);
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    account_budget::test_admission();
    timer_account_pressure();
    allocation_and_fresh_nodes();
    complete_replacement();
    retired_identity();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[completion namespace metadata] five unlocked allocation-stage rejections, 64 heap-held \
         fresh publications/removals, complete source/timer/backlog/authority handback, reentrant \
         disposal, retired-generation rejection and original-account reuse passed"
    );
}
