//! Private serialized queue owners exercise real admitted storage. Heap-held
//! probes run before AP schedulers; no live sponsor or physical root is replaced.

use super::*;
use crate::{
    klib::observer::{
        CallOnNotify,
        WaitSponsor,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const CLIENT: usize = 0x5e3c;

fn fresh_nodes() {
    open_address_space(CLIENT, 4);
    let account = cq_admission(CLIENT).unwrap();
    let baseline = cq_budget::node_used();
    let failed = Prepared::new_with(
        Target::capture(CLIENT).unwrap(),
        2,
        None,
        |_| panic!("node rejection reached list allocation"),
        |_| panic!("node rejection reached ring allocation"),
        || {
            let registry = COMPLETIONS.write();
            assert!(registry[&CLIENT].cqs.first_key_value().is_none());
            assert_eq!(account.used()[0], 1);
            Err(core::alloc::AllocError)
        },
    );
    assert!(matches!(failed, Err(CqOpenError::AllocationFailed)));
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(cq_budget::node_used(), baseline);
    let mut preparations = Vec::with_capacity(32);
    let mut retired = Vec::with_capacity(32);
    for _ in 0..32 {
        preparations.push(
            Prepared::new_with(
                Target::capture(CLIENT).unwrap(),
                2,
                None,
                |platform| ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform),
                cq::CompletionQueueRing::new_page,
                || {
                    {
                        let _registry = COMPLETIONS.write();
                    }
                    PreparedEntry::try_new()
                },
            )
            .unwrap(),
        );
    }
    assert_eq!(account.used()[0], 32);
    let rejected = Prepared::new_with(
        Target::capture(CLIENT).unwrap(),
        2,
        None,
        |_| panic!("full admission reached source allocation"),
        |_| panic!("full admission reached ring allocation"),
        || panic!("full admission reached metadata allocation"),
    );
    assert!(matches!(rejected, Err(CqOpenError::ResourceLimit)));
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for (id, preparing) in preparations.iter_mut().enumerate() {
            assert!(preparing.publish(id as CqId).unwrap().is_none());
            assert!(preparing.0.node.is_none() && preparing.0.state.is_none());
        }
        assert_eq!(COMPLETIONS.read()[&CLIENT].cqs.iter().count(), 32);
        assert_eq!(account.used()[0], 32);
    }
    for preparing in preparations {
        preparing.finish();
    }
    // Global namespace-map removal is still a separate allocating boundary.
    // Detach it before proving the real queue-node removal with the heap held.
    let mut namespace = COMPLETIONS.write().remove(&CLIENT).unwrap();
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        while let Some((&id, _)) = namespace.cqs.first_key_value() {
            retired.push(RetiredCq::from_node(namespace.cqs.take(&id).unwrap()));
        }
        assert_eq!(account.used()[0], 32);
    }
    namespace.finish();
    assert_eq!(account.used()[0], 32);
    for queue in retired {
        queue.finish();
    }
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(cq_budget::node_used(), baseline);
}

fn retired_backlog() {
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let account = cq_admission(CLIENT).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let ring = unsafe { cq_ring_of(CLIENT, DEFAULT_CQ).unwrap() };
    for cookie in 0..2 {
        let op = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, cookie).unwrap();
        complete_detached(CLIENT, op, OpResult::Ok(1)).unwrap();
    }
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("queue disposal fabricated readiness"));
    let source = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let token = sponsor.register(&source.identity, Arc::downgrade(&observer)).unwrap();
    let mut preparing = Prepared::new(Target::capture(CLIENT).unwrap(), 2, None).unwrap();
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired = preparing.publish(DEFAULT_CQ).unwrap().unwrap();
        assert_eq!(COMPLETIONS.read()[&CLIENT].live, 0);
        assert_eq!(account.used()[0], 2);
        assert_eq!(records.used(), 1);
        assert_eq!(sponsor.used(), 1);
        // Old heap ring is retained in its detached owner; this private
        // fixture is its only consumer and never resumes that queue.
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 0);
    }
    preparing.finish();
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let successor = cq_admission(CLIENT).unwrap();
    let next = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let next_token = sponsor.register(&next.identity, Arc::downgrade(&observer)).unwrap();
    assert_eq!(account.used()[0], 1);
    assert_eq!(sponsor.used(), 2);
    retired.finish();
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(records.used(), 0);
    assert_eq!(successor.used()[0], 1);
    assert_eq!(sponsor.used(), 1);
    assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 0);
    drop((token, next_token));
    assert_eq!(sponsor.used(), 0);
    close_address_space(CLIENT);
    assert_eq!(successor.used(), [0, 0]);
}

// This deliberately drops an inert ManuallyDrop preparation to test fallback.
#[allow(clippy::drop_non_drop)]
fn guarded_abandonment() {
    open_address_space(CLIENT, 4);
    let account = cq_admission(CLIENT).unwrap();
    let preparing = Prepared::new(Target::capture(CLIENT).unwrap(), 2, None).unwrap();
    close_address_space(CLIENT);
    let retained = account.used();
    assert_eq!(retained[0], 1);
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(preparing);
    }
    assert_eq!(account.used(), retained);
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let account = cq_admission(CLIENT).unwrap();
    let records = record_admission(CLIENT).unwrap();
    for cookie in 0..2 {
        let op = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, cookie).unwrap();
        complete_detached(CLIENT, op, OpResult::Ok(1)).unwrap();
    }
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("retained queue fabricated readiness"));
    let source = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let token = sponsor.register(&source.identity, Arc::downgrade(&observer)).unwrap();
    let mut preparing = Prepared::new(Target::capture(CLIENT).unwrap(), 2, None).unwrap();
    let retired = preparing.publish(DEFAULT_CQ).unwrap().unwrap();
    preparing.finish();
    open_address_space_with_cq(CLIENT, 4, 2).unwrap();
    let successor = cq_admission(CLIENT).unwrap();
    let retained = account.used();
    assert_eq!(retained[0], 1);
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(retired);
    }
    assert_eq!(account.used(), retained);
    assert_eq!(records.used(), 1);
    assert_eq!(sponsor.used(), 1);
    assert_eq!(successor.used()[0], 1);
    // The independent token still owns cancellation. Cancelling it later does
    // not free the abandoned queue/ring/backlog or its original record charge.
    drop(token);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(account.used(), retained);
    assert_eq!(records.used(), 1);
    close_address_space(CLIENT);
    assert_eq!(successor.used(), [0, 0]);
    crate::logln!(
        "[CQ metadata retention] guarded prepared/retired fallback retained two original queue \
         charges and one backlog record; successor and independent waiter cancellation preserved"
    );
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    fresh_nodes();
    retired_backlog();
    guarded_abandonment();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[CQ metadata] unlocked node rejection, 32 heap-held fresh publications/removals, \
         admission before allocation, complete backlog/source handback and original-account reuse \
         passed"
    );
}
