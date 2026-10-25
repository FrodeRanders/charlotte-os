//! Serialized native controls for admitted detached metadata and disposal.

use super::*;
use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;

const CLIENT: usize = 0x5e39;

fn prepare_and_retire() {
    open_address_space_with_cq(CLIENT, 64, 64).unwrap();
    let account = record_admission(CLIENT).unwrap();
    let baseline = budget::node_used();
    let failed = Preparing::plain_with(
        Target::capture(CLIENT, DEFAULT_CQ).unwrap(),
        0,
        &OPERATION_IDS,
        || {
            let registry = COMPLETIONS.write();
            assert_eq!(registry[&CLIENT].live, 0);
            assert_eq!(account.used(), 1);
            Err(core::alloc::AllocError)
        },
    );
    assert!(matches!(failed, Err(SubmitError::WouldBlock)));
    assert_eq!(account.used(), 0);
    assert_eq!(budget::node_used(), baseline);
    let mut preparations = Vec::with_capacity(64);
    let mut operations = Vec::with_capacity(64);
    let mut retired = Vec::with_capacity(64);
    for cookie in 0..64 {
        preparations.push(
            if cookie % 2 == 0 {
                Preparing::plain_with(
                    Target::capture(CLIENT, DEFAULT_CQ).unwrap(),
                    cookie,
                    &OPERATION_IDS,
                    || {
                        {
                            let _registry = COMPLETIONS.write();
                        }
                        PreparedEntry::try_new()
                    },
                )
                .unwrap()
            } else {
                Preparing::timer(
                    Target::capture(CLIENT, DEFAULT_CQ).unwrap(),
                    u64::MAX,
                    cookie,
                    &OPERATION_IDS,
                    Arc::try_new,
                )
                .unwrap()
            },
        );
    }
    assert_eq!(account.used(), 64);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for preparing in &mut preparations {
            operations.push(preparing.publish().unwrap());
            assert!(preparing.0.entry.is_none() && preparing.0.record.is_none());
        }
        let registry = COMPLETIONS.read();
        assert_eq!(registry[&CLIENT].detached.iter().count(), 64);
        assert_eq!(registry[&CLIENT].live, 64);
    }
    for mut preparing in preparations {
        preparing.enqueue();
        preparing.finish();
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for operation in &operations {
            retired.push(retire_detached(CLIENT, *operation, OpResult::Ok(7)).unwrap());
        }
        let registry = COMPLETIONS.read();
        assert!(registry[&CLIENT].detached.first_key_value().is_none());
        assert_eq!(registry[&CLIENT].live, 1);
        assert_eq!(account.used(), 64);
        assert_eq!(timer_events_used(CLIENT), 32);
    }
    for owner in retired {
        owner.finish();
    }
    assert_eq!(account.used(), 1);
    assert_eq!(timer_events_used(CLIENT), 0);
    let ring = unsafe { cq_ring_of(CLIENT, DEFAULT_CQ).unwrap() };
    for cookie in 0..63 {
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, cookie);
    }
    // The final backlog node and its charge leave in the same refund batch.
    assert_eq!(cq_pending(CLIENT, DEFAULT_CQ), 1);
    assert_eq!(account.used(), 0);
    assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 63);
    assert_eq!(budget::node_used(), baseline);
    for operation in operations {
        assert_eq!(
            complete_detached(CLIENT, operation, OpResult::Ok(8)),
            Err(CapError::UnknownCap)
        );
    }
    close_address_space(CLIENT);
}

fn namespace_drain() {
    for replacement in [false, true] {
        open_address_space_with_cq(CLIENT, 8, 2).unwrap();
        let records = record_admission(CLIENT).unwrap();
        let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
        let queues = cq_admission(CLIENT).unwrap();
        for cookie in 0..4 {
            submit_detached_timer(CLIENT, DEFAULT_CQ, u64::MAX, cookie).unwrap();
            let op = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, cookie).unwrap();
            complete_detached(CLIENT, op, OpResult::Ok(1)).unwrap();
        }
        assert_eq!(records.used(), 7); // four live timers and three backlogged nodes
        assert_eq!(timers.used(), 4);
        if replacement {
            open_address_space_with_cq(CLIENT, 8, 2).unwrap();
        } else {
            close_address_space(CLIENT);
        }
        assert_eq!(records.used(), 0);
        assert_eq!(timers.used(), 0);
        assert_eq!(queues.used(), [0, 0]);
        if replacement {
            assert_eq!(record_admission(CLIENT).unwrap().used(), 0);
            close_address_space(CLIENT);
        }
    }
}

// Deliberate terminal-retention controls use private raw fixture namespaces,
// never real user roots or a live sponsor substitution. Their bounded retained
// allocations/charges remain until guest shutdown; this is not retry custody.
fn guarded_abandonment() {
    use core::sync::atomic::{
        AtomicUsize,
        Ordering,
    };
    struct Probe(Arc<AtomicUsize>);
    impl Observer for Probe {
        fn notify(self: Arc<Self>) {
            panic!("abandonment fabricated readiness");
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    open_address_space_with_cq(CLIENT, 8, 2).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let queues = cq_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let original_authority = crate::capability::node_admission_used();
    let cap = crate::completion::submit(CLIENT, OpCode::Read, Some(alloc::vec![1, 2, 3])).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let weak = Arc::downgrade(&captured);
    let hits = Arc::new(AtomicUsize::new(0));
    let observer: Arc<dyn Observer> = Arc::new(Probe(hits.clone()));
    let source = crate::klib::observer::registration::ObserverList::try_new(1, false).unwrap();
    let registration = source
        .register(Arc::downgrade(&observer), watch_budget::reserve(&watches, false).unwrap())
        .unwrap();
    assert!(!captured.set_event_observation(observer, registration).finish());
    drop(captured);
    submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, 1).unwrap();
    for cookie in 2..4 {
        let op = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, cookie).unwrap();
        complete_detached(CLIENT, op, OpResult::Ok(1)).unwrap();
    }
    let namespace = {
        let mut registry = COMPLETIONS.write();
        let mut namespace = RetiredNamespace::from_node(registry.take(&CLIENT).unwrap());
        revoke_namespace_authority(CLIENT, &mut namespace);
        namespace
    };
    let queue_usage = queues.used();
    assert_eq!(records.used(), 3);
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(namespace);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
    assert_eq!(records.used(), 3);
    assert_eq!(
        crate::capability::node_admission_used(),
        (original_authority.0 + 1, original_authority.1 + 1)
    );
    assert_eq!(queues.used(), queue_usage);
    assert_eq!(watches.used(), 1);
    assert_eq!(source.registered(), 1);
    assert!(weak.upgrade().unwrap().holds_buffer());
    assert!(!crate::capability::contains(CLIENT, cap, crate::capability::ObjectKind::Completion));
    // Reuse does not reclaim or credit the retained predecessor's payloads.
    open_address_space_with_cq(CLIENT, 8, 2).unwrap();
    assert_eq!(record_admission(CLIENT).unwrap().used(), 0);
    close_address_space(CLIENT);
    assert_eq!(records.used(), 3);
    assert_eq!(
        crate::capability::node_admission_used(),
        (original_authority.0 + 1, original_authority.1 + 1)
    );
    assert_eq!(queues.used(), queue_usage);
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    open_address_space_with_cq(CLIENT, 8, 2).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
    let timer = Preparing::timer(
        Target::capture(CLIENT, DEFAULT_CQ).unwrap(),
        u64::MAX,
        1,
        &OPERATION_IDS,
        Arc::try_new,
    )
    .unwrap();
    let plain =
        Preparing::plain(Target::capture(CLIENT, DEFAULT_CQ).unwrap(), 2, &OPERATION_IDS).unwrap();
    let observer =
        Arc::downgrade(timer.0.record.as_ref().unwrap()._timer_observer.as_ref().unwrap());
    close_address_space(CLIENT);
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop((timer, plain));
    }
    assert_eq!(records.used(), 2);
    assert_eq!(timers.used(), 1);
    assert!(observer.upgrade().is_some());

    for flush in [false, true] {
        open_address_space_with_cq(CLIENT, 8, 2).unwrap();
        let records = record_admission(CLIENT).unwrap();
        let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
        let operation = submit_detached_timer(CLIENT, DEFAULT_CQ, u64::MAX, 1).unwrap();
        let observer = Arc::downgrade(
            COMPLETIONS.read()[&CLIENT].detached[&operation]._timer_observer.as_ref().unwrap(),
        );
        if flush {
            for cookie in 2..4 {
                let op = submit_detached(CLIENT, DEFAULT_CQ, OpCode::Nop, cookie).unwrap();
                complete_detached(CLIENT, op, OpResult::Ok(1)).unwrap();
            }
            let ring = unsafe { cq_ring_of(CLIENT, DEFAULT_CQ).unwrap() };
            assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 2);
        }
        let retired = retire_detached(CLIENT, operation, OpResult::Ok(1)).unwrap();
        if flush {
            assert_eq!(records.used(), 2);
        }
        close_address_space(CLIENT);
        assert_eq!(records.used(), 1);
        {
            let _registry = COMPLETIONS.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            drop(retired);
        }
        assert_eq!(records.used(), 1);
        assert_eq!(timers.used(), 1);
        assert!(observer.upgrade().is_some());
    }
    crate::logln!(
        "[detached metadata retention] guarded namespace/preparation/terminal abandonment \
         retained original nodes/charges, timers, CQ backing, watch and buffer; successors \
         unaffected"
    );
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    prepare_and_retire();
    namespace_drain();
    guarded_abandonment();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[detached metadata] unlocked allocation/rejection, 64 heap-held fresh \
         publications/removals, charged backlog nodes, exact duplicate rejection and complete \
         namespace close/replacement passed"
    );
}
