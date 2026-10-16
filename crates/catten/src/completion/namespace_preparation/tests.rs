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

fn with_queue(asid: usize) -> Preparing {
    let mut preparing = Preparing::new(asid, 4).unwrap();
    let queue = queue_preparation::Prepared::heap_for_namespace(asid, preparing.entries(), 2);
    preparing.entries_mut().cqs.insert(queue.0, DEFAULT_CQ, queue.1);
    preparing
}

fn allocation_and_fresh_nodes() {
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let account = cq_admission(CLIENT).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![3])).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let original = account.used();
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
        for next in &mut preparing {
            assert!(next.publish().unwrap().is_none());
            assert!(next.is_published());
        }
        let mut registry = COMPLETIONS.write();
        for (i, account) in accounts.iter().enumerate() {
            retired.push(RetiredNamespace::from_node(registry.take(&(CLIENT + i)).unwrap()));
            assert_eq!(account.used()[0], 1);
        }
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
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let queues = cq_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
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
    let old_token = sponsor.register(&old_source.identity, Arc::downgrade(&observer)).unwrap();
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
    let next_token = sponsor.register(&next_source.identity, Arc::downgrade(&observer)).unwrap();
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
}

fn retired_identity() {
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
    assert_eq!(current.used(), before);
    crate::memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(current.used(), [0, 0]);
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
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
