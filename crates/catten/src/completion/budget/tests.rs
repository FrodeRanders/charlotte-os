//! Actual backlog publication with original record/domain/node/heap guards
//! held. The returned charge batch must neither allocate nor refund there.

use super::*;
use crate::{
    completion,
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const CLIENT: usize = 0x5e38;

fn mixed_classes() {
    let baseline = node_used();
    let account = DomainBudget::new(64);
    // This isolated kernel accounting fixture uses trusted classification and
    // leaves every live namespace's sponsor unchanged.
    let mut charges: [Option<Charge>; 64] =
        core::array::from_fn(|i| Some(reserve(&account, i % 2 != 0).unwrap()));
    let mut releases = Releases::default();
    {
        let domain = account.0.lock();
        let node = NODE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        for source in &mut charges {
            releases.push(source.take().unwrap());
        }
        assert_eq!(domain.used()[0] as usize, 64);
        assert_eq!(node.total.used()[0] as usize, baseline.0 + 64);
        assert_eq!(node.ordinary.used()[0] as usize, baseline.1 + 32);
        assert_eq!(releases.charges[0].as_ref().unwrap().count, 32);
        assert_eq!(releases.charges[1].as_ref().unwrap().count, 32);
    }
    assert_eq!(account.used(), 64);
    drop(releases);
    assert_eq!(account.used(), 0);
    assert_eq!(node_used(), baseline);
}

fn full_flush_and_replacement() {
    let baseline = node_used();
    completion::open_address_space_with_cq(CLIENT, 128, 64).unwrap();
    let account = completion::record_admission(CLIENT).unwrap();
    let ring = unsafe { completion::cq_ring_of(CLIENT, completion::DEFAULT_CQ).unwrap() };
    for cookie in 0..128 {
        let operation = completion::submit_detached(
            CLIENT,
            completion::DEFAULT_CQ,
            completion::OpCode::Nop,
            cookie,
        )
        .unwrap();
        completion::complete_detached(CLIENT, operation, completion::OpResult::Ok(cookie as i64))
            .unwrap();
    }
    let capacity =
        completion::COMPLETIONS.read()[&CLIENT].cqs[&completion::DEFAULT_CQ].ring_capacity as usize;
    let pending = 128 - (capacity - 1);
    assert!(pending > 1);
    assert_eq!(account.used(), pending);
    let mut releases = Releases::default();
    {
        let mut registry = completion::COMPLETIONS.write();
        let domain = account.0.lock();
        let node = NODE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let outcome = completion::flush_cq_backlog(
            registry.get_mut(&CLIENT).unwrap(),
            completion::DEFAULT_CQ,
            &mut releases,
        );
        assert_eq!((outcome.total, outcome.detached), (0, 0));
        assert_eq!(domain.used()[0] as usize, pending);
        assert_eq!(node.total.used()[0] as usize, baseline.0 + pending);
        assert!(releases.charges.iter().all(Option::is_none));
    }
    // This private heap ring has one consumer throughout the fixture.
    for cookie in 0..capacity - 1 {
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, cookie as u64);
    }
    let flushed = pending.min(capacity - 1);
    {
        let mut registry = completion::COMPLETIONS.write();
        let domain = account.0.lock();
        let node = NODE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let entries = registry.get_mut(&CLIENT).unwrap();
        let outcome = completion::flush_cq_backlog(entries, completion::DEFAULT_CQ, &mut releases);
        assert_eq!((outcome.total, outcome.detached), (flushed, flushed));
        assert_eq!(entries.live, pending - flushed);
        assert_eq!(domain.used()[0] as usize, pending, "visible entries refunded under registry");
        assert_eq!(node.total.used()[0] as usize, baseline.0 + pending);
        assert_eq!(releases.charges[0].as_ref().unwrap().count, flushed as u64);
    }
    for offset in 0..flushed {
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, (capacity - 1 + offset) as u64);
    }
    assert_eq!(account.used(), pending);
    completion::close_address_space(CLIENT);
    assert_eq!(account.used(), flushed);
    completion::open_address_space_with_cq(CLIENT, 128, 64).unwrap();
    let successor = completion::record_admission(CLIENT).unwrap();
    assert!(!Arc::ptr_eq(&account, &successor));
    drop(releases);
    assert_eq!((account.used(), successor.used()), (0, 0));
    assert_eq!(node_used(), baseline);
    completion::close_address_space(CLIENT);
}

fn producer_paths() {
    for capability in [false, true] {
        completion::open_address_space_with_cq(CLIENT, 4, 2).unwrap();
        let account = completion::record_admission(CLIENT).unwrap();
        let ring = unsafe { completion::cq_ring_of(CLIENT, completion::DEFAULT_CQ).unwrap() };
        let pending = if capability {
            completion::submit(CLIENT, completion::OpCode::Nop, None).unwrap()
        } else {
            completion::submit_detached(CLIENT, completion::DEFAULT_CQ, completion::OpCode::Nop, 3)
                .unwrap()
        };
        for cookie in 1..=2 {
            let operation = completion::submit_detached(
                CLIENT,
                completion::DEFAULT_CQ,
                completion::OpCode::Nop,
                cookie,
            )
            .unwrap();
            completion::complete_detached(
                CLIENT,
                operation,
                completion::OpResult::Ok(cookie as i64),
            )
            .unwrap();
        }
        assert_eq!(account.used(), 2);
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 1);
        if capability {
            completion::complete(CLIENT, pending, completion::OpResult::Ok(3)).unwrap();
        } else {
            completion::complete_detached(CLIENT, pending, completion::OpResult::Ok(3)).unwrap();
        }
        assert_eq!(account.used(), 1, "producer left flushed charges pending after return");
        assert_eq!(unsafe { (&mut *ring).read().unwrap() }.cookie, 2);
        assert_eq!(completion::cq_pending(CLIENT, completion::DEFAULT_CQ), 1);
        if capability {
            assert_eq!(account.used(), 1);
            completion::close(CLIENT, pending).unwrap();
        }
        assert_eq!(account.used(), 0);
        completion::close_address_space(CLIENT);
    }
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    mixed_classes();
    full_flush_and_replacement();
    producer_paths();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[backlog charge handback] full/FIFO flush under registry/domain/node/heap guards, \
         ordinary/platform aggregation, original-account replacement and both producers passed"
    );
}
