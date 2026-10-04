//! Self-tests for the completion-capability subsystem (Option C prototype).
//!
//! These are whitebox integration tests of the kernel side of the async syscall
//! ABI (`docs/architecture/async-syscall-abi.md`). They validate the submission-side
//! semantics that exist today — the capability table, the buffer-ownership /
//! deferred-reclaim contract, the observer-signal path that [`wait`] relies on,
//! and submission backpressure — without requiring a running scheduler.
//! The separate scoped probe exercises the implemented EL0 syscall entry.
//!
//! [`wait`](crate::completion::wait) itself is not exercised here because it
//! blocks the calling thread, which requires the scheduler to be yielding;
//! self-tests run before the BSP yields. The signal path `wait` depends on is
//! validated through the owning callback fixtures and scheduled CQ tests.

use crate::{
    completion::{
        self,
        CancelState,
        OpCode,
        OpResult,
        OpStateKind,
        SubmitError,
    },
    logln,
};

pub fn test_completion_caps() {
    logln!("Testing completion-capability subsystem...");

    let asid = 0xc0ffee;
    completion::open_address_space(asid, 2);

    // --- submit transfers buffer ownership to the kernel ---------------------
    let cap = completion::submit(asid, OpCode::Read, Some(alloc::vec![0u8; 4])).unwrap();
    assert!(completion::holds_buffer(asid, cap).unwrap());
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::InFlight);

    // --- complete posts the result, hands the buffer back on poll -----------
    completion::complete(asid, cap, OpResult::Ok(4)).unwrap();
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Completed);
    // Buffer stays with the kernel until poll() drains it.
    assert!(completion::holds_buffer(asid, cap).unwrap());

    // A second completion is an idempotent no-op and must not change the result.
    completion::complete(asid, cap, OpResult::Err(9)).unwrap();

    let done = completion::poll(asid, cap).unwrap().expect("must be complete");
    assert!(!completion::holds_buffer(asid, cap).unwrap());
    assert!(matches!(done.result, OpResult::Ok(4)));
    assert_eq!(done.buffer.as_deref(), Some(&[0u8; 4][..]));
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Observed);

    // Draining twice yields nothing; the observed state is stable.
    assert!(completion::poll(asid, cap).unwrap().is_none());
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Observed);
    // Completing an observed operation is rejected as a no-op.
    completion::complete(asid, cap, OpResult::Ok(1)).unwrap();
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Observed);
    // Cancelling an observed operation reports AlreadyComplete.
    assert_eq!(completion::cancel(asid, cap).unwrap(), CancelState::AlreadyComplete);

    let first_operation = completion::operation_id(asid, cap).unwrap();

    // --- close revokes the handle --------------------------------------------
    completion::close(asid, cap).unwrap();
    assert!(completion::poll(asid, cap).is_err());

    // --- stale handles never alias a later operation -------------------------
    let next_cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    assert_ne!(next_cap, cap, "unified capability handles must not be reused");
    assert_ne!(
        completion::operation_id(asid, next_cap).unwrap(),
        first_operation,
        "a fresh capability must name a fresh operation id"
    );

    // --- close rejects in-flight caps (must complete or be drained first) ----
    assert!(completion::close(asid, next_cap).is_err()); // NotComplete
    completion::complete(asid, next_cap, OpResult::Ok(0)).unwrap();
    completion::close(asid, next_cap).unwrap(); // now it works

    // --- cancel: InFlight -> CancelPending -> Completed(Cancelled) -----------
    let cap2 = completion::submit(asid, OpCode::Write, Some(alloc::vec![1u8, 2, 3])).unwrap();
    assert_eq!(completion::cancel(asid, cap2).unwrap(), CancelState::CancelRequested);
    assert_eq!(completion::state_of(asid, cap2).unwrap(), OpStateKind::CancelPending);
    // Cancellation is idempotent while pending.
    assert_eq!(completion::cancel(asid, cap2).unwrap(), CancelState::CancelRequested);
    // Deferred reclaim: the kernel still owns the buffer while cancellation is
    // in flight — it may still touch it until the terminal completion.
    assert!(completion::holds_buffer(asid, cap2).unwrap());
    // A cancel-pending operation is not reclaimable yet.
    assert!(completion::close(asid, cap2).is_err());

    completion::complete(asid, cap2, OpResult::Ok(3)).unwrap();
    assert_eq!(completion::state_of(asid, cap2).unwrap(), OpStateKind::Completed);
    let done = completion::poll(asid, cap2).unwrap().expect("must be complete");
    assert!(matches!(done.result, OpResult::Cancelled));
    assert_eq!(done.buffer.as_deref(), Some(&[1u8, 2, 3][..]));
    completion::close(asid, cap2).unwrap();

    // --- submission backpressure (capacity 2) --------------------------------
    let _a = completion::submit(asid, OpCode::Nop, None).unwrap();
    let _b = completion::submit(asid, OpCode::Nop, None).unwrap();
    assert_eq!(completion::submit(asid, OpCode::Nop, None), Err(SubmitError::WouldBlock));

    completion::close_address_space(asid);

    // --- CQ overflow is retained in a kernel backlog, not lost --------------
    let cq_asid = 0xc0ff_ee01;
    completion::open_address_space_with_cq(cq_asid, 4, 2).expect("CQ setup failed");
    let cap_a = completion::submit(cq_asid, OpCode::Nop, None).unwrap();
    let cap_b = completion::submit(cq_asid, OpCode::Nop, None).unwrap();

    completion::complete(cq_asid, cap_a, OpResult::Ok(10)).unwrap();
    completion::complete(cq_asid, cap_b, OpResult::Ok(20)).unwrap();

    let ring_ptr = unsafe { completion::cq_ring_of(cq_asid, 0) }.expect("CQ ring must exist");
    assert_eq!(unsafe { &*ring_ptr }.pending(), 1, "small CQ should hold the first entry");
    assert_eq!(unsafe { &*ring_ptr }.overflow, 1, "second entry should hit a full ring");

    let first = unsafe { &mut *ring_ptr }.read().expect("first CQ entry must be present");
    assert_eq!(first.cookie, cap_a);

    assert_eq!(
        completion::cq_pending(cq_asid, 0),
        1,
        "cq_pending should flush the retained backlog entry"
    );
    let second = unsafe { &mut *ring_ptr }.read().expect("backlogged CQ entry must be posted");
    assert_eq!(second.cookie, cap_b);

    // --- a duplicate completion must not post a duplicate CQ entry -----------
    completion::complete(cq_asid, cap_a, OpResult::Ok(11)).unwrap();
    assert_eq!(
        completion::cq_pending(cq_asid, 0),
        0,
        "idempotent re-completion must not produce a CQ entry"
    );

    // --- a cancelled operation's CQ entry carries the effective result -------
    let cap_c = completion::submit(cq_asid, OpCode::Nop, None).unwrap();
    assert_eq!(completion::cancel(cq_asid, cap_c).unwrap(), CancelState::CancelRequested);
    completion::complete(cq_asid, cap_c, OpResult::Ok(30)).unwrap();
    let third = unsafe { &mut *ring_ptr }.read().expect("cancelled CQ entry must be posted");
    assert_eq!(third.cookie, cap_c);
    assert_eq!(
        crate::completion::cq::fields_to_op_result(third.status, third.result),
        OpResult::Cancelled,
        "the CQ ring and the capability must agree on the effective result"
    );

    assert!(completion::poll(cq_asid, cap_a).unwrap().is_some());
    assert!(completion::poll(cq_asid, cap_b).unwrap().is_some());
    assert!(matches!(
        completion::poll(cq_asid, cap_c).unwrap().expect("cancelled op must drain").result,
        OpResult::Cancelled
    ));
    completion::close(cq_asid, cap_a).unwrap();
    completion::close(cq_asid, cap_b).unwrap();
    completion::close(cq_asid, cap_c).unwrap();

    // --- cap-based consumers must not leave an unbounded stale backlog ------
    // Keep the one-slot ring full, then repeatedly consume completions through
    // poll(cap) instead. close(cap) must discard each redundant, undelivered
    // CQ record rather than retaining it forever in the kernel backlog.
    let blocker = completion::submit(cq_asid, OpCode::Nop, None).unwrap();
    completion::complete(cq_asid, blocker, OpResult::Ok(40)).unwrap();
    assert!(completion::poll(cq_asid, blocker).unwrap().is_some());
    completion::close(cq_asid, blocker).unwrap();
    for value in 0..64 {
        let cap = completion::submit(cq_asid, OpCode::Nop, None).unwrap();
        completion::complete(cq_asid, cap, OpResult::Ok(value)).unwrap();
        assert!(completion::poll(cq_asid, cap).unwrap().is_some());
        completion::close(cq_asid, cap).unwrap();
    }
    let blocker_entry = unsafe { &mut *ring_ptr }.read().expect("blocking CQ entry must remain");
    assert_eq!(blocker_entry.cookie, blocker);
    assert_eq!(
        completion::cq_pending(cq_asid, 0),
        0,
        "closed cap completions must not remain in the CQ backlog"
    );
    completion::close_address_space(cq_asid);

    test_completion_timer_admission();
    test_completion_record_admission();
    test_completion_queue_admission();

    logln!("Completion-capability subsystem tests passed.");
}

/// Kernel-boundary tests intentionally retain raw capabilities to verify the
/// admission and cancellation ABI, independently of userspace owner wrappers.
fn test_completion_timer_admission() {
    use crate::timers::budget::{
        self,
        DomainBudget,
    };
    let before = budget::node_used();
    let old = DomainBudget::new(2);
    let a = budget::reserve(&old, false).unwrap();
    let b = budget::reserve(&old, false).unwrap();
    assert!(budget::reserve(&old, false).is_err());
    let replacement = DomainBudget::new(2);
    drop(a);
    assert_eq!(old.used(), 1);
    assert_eq!(replacement.used(), 0, "late release must not credit a successor");
    drop(b);
    assert_eq!(budget::node_used(), before);

    // Reservation-only saturation does not enqueue thousands of timers or
    // consume the equivalent event storage. Every failed reservation rolls
    // back its local and node counters, leaving platform progress possible.
    let mut charges = alloc::vec::Vec::new();
    for _ in 0..budget::MAX_ORDINARY_TIMERS / budget::MAX_DOMAIN_TIMERS {
        let domain = DomainBudget::new(budget::MAX_DOMAIN_TIMERS);
        while let Ok(charge) = budget::reserve(&domain, false) {
            charges.push(charge);
        }
    }
    assert_eq!(budget::node_used().1, budget::MAX_ORDINARY_TIMERS);
    let extra = DomainBudget::new(1);
    assert!(budget::reserve(&extra, false).is_err());
    assert_eq!(extra.used(), 0);
    let progress = budget::reserve(&extra, true).expect("platform timer reserve");
    drop(progress);
    drop(charges);
    assert_eq!(budget::node_used(), before);

    let asid = 0xc0ae_b001;
    completion::open_address_space_with_cq(asid, 2, 2).expect("CQ setup failed");
    for _ in 0..64 {
        let cap = completion::submit_timer(asid, 3_600_000).unwrap();
        assert_eq!(completion::timer_events_used(asid), 1);
        assert_eq!(completion::cancel(asid, cap).unwrap(), CancelState::CancelRequested);
        assert_eq!(completion::timer_events_used(asid), 0, "local cancellation reclaims the event");
        assert_eq!(completion::poll(asid, cap).unwrap().unwrap().result, OpResult::Cancelled);
        assert_eq!(completion::cancel(asid, cap).unwrap(), CancelState::AlreadyComplete);
        completion::close(asid, cap).unwrap();
    }
    let cap = completion::submit_timer(asid, 3_600_000).unwrap();
    let detached = completion::submit_detached_timer(asid, 0, 3_600_000, 0x77).unwrap();
    let full = budget::node_used();
    assert_eq!(completion::submit_timer(asid, 1), Err(SubmitError::WouldBlock));
    assert_eq!(completion::submit_detached_timer(asid, 0, 1, 0), Err(SubmitError::WouldBlock));
    assert_eq!(budget::node_used(), full);
    completion::cancel(asid, cap).unwrap();
    completion::close(asid, cap).unwrap();
    assert_eq!(completion::cancel_detached(asid, detached).unwrap(), CancelState::CancelRequested);
    assert_eq!(completion::timer_events_used(asid), 0);
    // A full ring keeps the cancelled detached result/submission slot live.
    let ring = unsafe { completion::cq_ring_of(asid, 0) }.unwrap();
    while unsafe { &mut *ring }.read().is_some() {}
    let recovered = completion::submit_timer(asid, u64::MAX).unwrap();
    completion::abort_submission(asid, recovered).unwrap();
    assert_eq!(completion::timer_events_used(asid), 0, "submission rollback cancels its event");

    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);
    completion::open_address_space(asid, 1);
    let deferred = completion::submit_timer(asid, 3_600_000).unwrap();
    {
        // The per-LP guard masks IRQs. This reproduces cancellation from a
        // timer callback: flag the node without freeing its admission charge.
        let queue = crate::timers::TIMER_QUEUES.try_get_mut().unwrap();
        completion::cancel(asid, deferred).unwrap();
        completion::close(asid, deferred).unwrap();
        assert_eq!(completion::record_admission(asid).unwrap().used(), 0);
        assert_eq!(completion::timer_events_used(asid), 1);
        assert_eq!(completion::submit_timer(asid, 1), Err(SubmitError::WouldBlock));
        assert_eq!(
            completion::record_admission(asid).unwrap().used(),
            0,
            "timer-event rejection must roll back its staged record charge"
        );
        drop(queue);
    }
    crate::timers::process_local_events();
    assert_eq!(completion::timer_events_used(asid), 0);
    let recovered = completion::submit_timer(asid, 3_600_000).unwrap();
    completion::cancel(asid, recovered).unwrap();
    completion::close(asid, recovered).unwrap();

    // Simulate a callback already captured when the namespace is replaced.
    // Deliberately recycle both the ASID and numeric capability value.
    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);
    completion::open_address_space_with_cq(asid, 2, 2).expect("CQ setup failed");
    let old_cap = completion::submit_timer(asid, 3_600_000).unwrap();
    let captured = completion::completion_of(asid, old_cap).unwrap();
    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);
    completion::open_address_space_with_cq(asid, 2, 2).expect("CQ setup failed");
    let new_cap = completion::submit_timer(asid, 3_600_000).unwrap();
    assert_eq!(new_cap, old_cap, "test must exercise exact numeric namespace reuse");
    assert_eq!(
        completion::complete_registered(asid, new_cap, captured.clone(), OpResult::Ok(0)),
        Err(completion::CapError::UnknownCap)
    );
    assert_eq!(completion::state_of(asid, new_cap).unwrap(), OpStateKind::InFlight);
    drop(captured);
    assert_eq!(completion::timer_events_used(asid), 1);
    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);
    assert_eq!(budget::node_used(), before);
    logln!(
        "[completion timers] quota, reserved progress, cancellation churn, rollback and namespace \
         replacement passed"
    );
}

/// Direct kernel ABI boundary; raw handles and retained Arcs exercise actual
/// record lifetimes independently of the userspace capability owner.
fn test_completion_record_admission() {
    use completion::budget::{
        self,
        DomainBudget,
    };
    let before = budget::node_used();
    let create = || {
        let user_as = {
            let _kernel = crate::memory::KERNEL_AS.lock();
            crate::cpu::isa::memory::paging::AddressSpace::new_user()
        };
        crate::memory::register_user_address_space(user_as).unwrap().id()
    };
    let asid = 0xc0ae_b002;
    completion::open_address_space(asid, 1);
    let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    let account = completion::record_admission(asid).unwrap();
    let retained = completion::completion_of(asid, cap).unwrap();
    completion::complete(asid, cap, OpResult::Ok(0)).unwrap();
    completion::close(asid, cap).unwrap();
    assert_eq!(account.used(), 1, "a retained object outlives its closed cap");
    assert_eq!(completion::submit(asid, OpCode::Nop, None), Err(SubmitError::WouldBlock));
    assert_eq!(completion::submit_timer(asid, 1), Err(SubmitError::WouldBlock));
    assert_eq!(completion::timer_events_used(asid), 0);
    drop(retained);
    assert_eq!(account.used(), 0);
    let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    completion::abort_submission(asid, cap).unwrap();
    assert_eq!(account.used(), 0);
    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);

    completion::open_address_space_with_cq(asid, 2, 2).expect("CQ setup failed");
    let account = completion::record_admission(asid).unwrap();
    let a = completion::submit_detached(asid, 0, OpCode::Nop, 11).unwrap();
    let b = completion::submit_detached(asid, 0, OpCode::Nop, 12).unwrap();
    assert_eq!(account.used(), 2);
    completion::complete_detached(asid, a, OpResult::Ok(0)).unwrap();
    assert_eq!(account.used(), 1);
    completion::cancel_detached(asid, b).unwrap();
    assert_eq!(account.used(), 1, "non-timer cancellation still awaits its producer");
    completion::complete_detached(asid, b, OpResult::Ok(0)).unwrap();
    assert_eq!(account.used(), 1, "an undelivered detached result retains its charge");
    let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    assert_eq!(account.used(), 2);
    assert_eq!(completion::submit_detached(asid, 0, OpCode::Nop, 13), Err(SubmitError::WouldBlock));
    assert_eq!(completion::submit_detached_timer(asid, 0, 1, 13), Err(SubmitError::WouldBlock));
    assert_eq!(
        completion::submit_detached(asid, 7, OpCode::Nop, 13),
        Err(SubmitError::NoCompletionQueue)
    );
    assert_eq!(account.used(), 2);
    completion::complete(asid, cap, OpResult::Ok(0)).unwrap();
    completion::close(asid, cap).unwrap();
    assert_eq!(account.used(), 1);
    let ring = unsafe { completion::cq_ring_of(asid, 0) }.unwrap();
    assert_eq!(unsafe { &mut *ring }.read().unwrap().cookie, 11);
    assert_eq!(completion::cq_pending(asid, 0), 1);
    assert_eq!(account.used(), 0, "ring delivery frees the retained record");
    assert_eq!(unsafe { &mut *ring }.read().unwrap().cookie, 12);
    let a = completion::submit_detached(asid, 0, OpCode::Nop, 14).unwrap();
    let b = completion::submit_detached(asid, 0, OpCode::Nop, 15).unwrap();
    completion::complete_detached(asid, a, OpResult::Ok(0)).unwrap();
    completion::complete_detached(asid, b, OpResult::Ok(0)).unwrap();
    completion::close_address_space(asid);
    assert_eq!(account.used(), 0, "namespace teardown frees an undelivered result");
    crate::capability::close_address_space(asid);
    assert_eq!(budget::node_used(), before);

    completion::open_address_space_with_cq(asid, 1, 2).expect("CQ setup failed");
    let account = completion::record_admission(asid).unwrap();
    for cookie in [16, 17] {
        let op = completion::submit_detached(asid, 0, OpCode::Nop, cookie).unwrap();
        completion::complete_detached(asid, op, OpResult::Ok(0)).unwrap();
    }
    assert_eq!(account.used(), 1);
    completion::open_cq(asid, 0, 2).expect("CQ setup failed");
    assert_eq!(account.used(), 0);
    let cap = completion::submit(asid, OpCode::Nop, None)
        .expect("replacing a CQ must return discarded detached submission slots");
    completion::abort_submission(asid, cap).unwrap();
    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);

    // Independently saturate ordinary and total node pools by reservation.
    let mut charges = alloc::vec::Vec::new();
    let mut remaining = budget::MAX_ORDINARY_RECORDS - before.1;
    while remaining != 0 {
        let count = remaining.min(budget::MAX_DOMAIN_RECORDS);
        let domain = DomainBudget::new(count);
        for _ in 0..count {
            charges.push(budget::reserve(&domain, false).unwrap());
        }
        remaining -= count;
    }
    let blocked = DomainBudget::new(1);
    let full = budget::node_used();
    assert!(budget::reserve(&blocked, false).is_err());
    assert_eq!(blocked.used(), 0);
    assert_eq!(budget::node_used(), full);
    drop(budget::reserve(&blocked, true).expect("platform record reserve"));
    let probe = create();
    completion::open_address_space_with_cq(probe, 2, 2).expect("CQ setup failed");
    let probe_account = completion::record_admission(probe).unwrap();
    assert_eq!(completion::submit(probe, OpCode::Nop, None), Err(SubmitError::WouldBlock));
    assert_eq!(completion::submit_timer(probe, 1), Err(SubmitError::WouldBlock));
    assert_eq!(completion::submit_detached(probe, 0, OpCode::Nop, 0), Err(SubmitError::WouldBlock));
    assert_eq!(completion::submit_detached_timer(probe, 0, 1, 0), Err(SubmitError::WouldBlock));
    assert_eq!(probe_account.used(), 0);
    assert_eq!(completion::timer_events_used(probe), 0);
    assert_eq!(budget::node_used(), full);
    crate::memory::budget::mark_platform(
        crate::memory::current_address_space_handle(probe).unwrap(),
    );
    let progress = completion::submit(probe, OpCode::Nop, None)
        .expect("kernel-designated platform domain must use reserved records");
    completion::abort_submission(probe, progress).unwrap();
    crate::self_test::close_test_address_space(probe).unwrap();
    assert_eq!(budget::node_used(), full);
    remaining = budget::MAX_NODE_RECORDS - full.0;
    while remaining != 0 {
        let count = remaining.min(budget::MAX_DOMAIN_RECORDS);
        let domain = DomainBudget::new(count);
        for _ in 0..count {
            charges.push(budget::reserve(&domain, true).unwrap());
        }
        remaining -= count;
    }
    let full = budget::node_used();
    assert!(budget::reserve(&blocked, true).is_err());
    assert_eq!(blocked.used(), 0);
    assert_eq!(budget::node_used(), full);
    drop(charges);
    assert_eq!(budget::node_used(), before);

    let owner = create();
    let identity = crate::memory::current_address_space_handle(owner).unwrap();
    completion::open_address_space_with_cq(owner, 2, 2).expect("CQ setup failed");
    let account = completion::record_admission(owner).unwrap();
    let cap = completion::submit(owner, OpCode::Nop, None).unwrap();
    let captured = completion::completion_of(owner, cap).unwrap();
    let old_cap = cap;
    completion::complete(owner, cap, OpResult::Ok(0)).unwrap();
    crate::memory::budget::retire(identity);
    assert_eq!(completion::submit(owner, OpCode::Nop, None), Err(SubmitError::UnknownAddressSpace));
    assert_eq!(completion::submit_timer(owner, 1), Err(SubmitError::UnknownAddressSpace));
    assert_eq!(
        completion::submit_detached(owner, 0, OpCode::Nop, 0),
        Err(SubmitError::UnknownAddressSpace)
    );
    assert_eq!(
        completion::submit_detached_timer(owner, 0, 1, 0),
        Err(SubmitError::UnknownAddressSpace)
    );
    assert_eq!(account.used(), 1);
    crate::self_test::close_test_address_space(owner).unwrap();
    assert_eq!(account.used(), 1);
    let replacement = create();
    assert_eq!(replacement, owner);
    assert_ne!(crate::memory::current_address_space_handle(replacement).unwrap(), identity);
    completion::open_address_space(replacement, 1);
    let cap = completion::submit(replacement, OpCode::Nop, None).unwrap();
    let fresh = completion::record_admission(replacement).unwrap();
    assert_eq!(cap, old_cap, "fixture must reuse the exact numeric capability");
    assert_eq!(
        completion::close_registered(replacement, cap, captured.clone()),
        Err(completion::CapError::UnknownCap)
    );
    assert_eq!(completion::state_of(replacement, cap).unwrap(), OpStateKind::InFlight);
    drop(captured);
    assert_eq!(account.used(), 0);
    assert_eq!(fresh.used(), 1, "old-generation release cannot credit replacement");
    completion::abort_submission(replacement, cap).unwrap();
    crate::self_test::close_test_address_space(replacement).unwrap();
    assert_eq!(budget::node_used(), before);
    logln!(
        "[completion records] quotas, retained objects/results, rollback, retirement and ASID \
         reuse passed"
    );
}

/// Runs before other LPs begin scheduling, so synthetic namespaces and raw
/// ring inspection below have no concurrent producer or consumer.
fn test_completion_queue_admission() {
    use completion::{
        CqOpenError,
        cq::CqRingError,
        cq_budget as budget,
    };

    use crate::cpu::isa::interface::memory::address::PhysicalAddress;

    let before = budget::node_used();
    let asid = 0xc0ff_ee30;
    assert_eq!(completion::open_cq(asid, 0, 2), Err(CqOpenError::UnknownAddressSpace));
    assert_eq!(budget::node_used(), before);
    completion::open_address_space_with_cq(asid, 2, 2).unwrap();
    let account = completion::cq_admission(asid).unwrap();
    let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    completion::complete(asid, cap, OpResult::Ok(73)).unwrap();
    let ring = unsafe { completion::cq_ring_of(asid, 0) }.unwrap();
    assert_eq!((ring as usize) % core::mem::align_of::<completion::cq::CompletionQueueRing>(), 0);
    let charged = account.used();
    let admitted = budget::node_used();
    assert_eq!(
        completion::open_address_space_with_cq(asid, usize::MAX, 1),
        Err(CqOpenError::Ring(CqRingError::CapacityTooSmall))
    );
    assert_eq!(completion::state_of(asid, cap).unwrap(), OpStateKind::Completed);
    assert_eq!(unsafe { completion::cq_ring_of(asid, 0) }.unwrap(), ring);
    assert_eq!(unsafe { &*ring }.pending(), 1);
    assert_eq!(account.used(), charged);
    assert_eq!(budget::node_used(), admitted);
    for id in 1..budget::DOMAIN_LIMIT[0] as u32 {
        completion::open_cq(asid, id, 2).unwrap();
    }
    assert_eq!(account.used()[0], budget::DOMAIN_LIMIT[0]);
    let full = budget::node_used();
    assert_eq!(completion::open_cq(asid, 32, 2), Err(CqOpenError::ResourceLimit));
    assert_eq!(completion::open_cq(asid, 0, 2), Err(CqOpenError::ResourceLimit));
    assert_eq!(budget::node_used(), full);
    assert_eq!(unsafe { &mut *ring }.read().unwrap().result, 73);
    completion::close_address_space(asid);
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(budget::node_used(), before);

    // A huge requested submission capacity is clamped before backlog sizing.
    completion::open_address_space_with_cq(asid, usize::MAX, u32::MAX).unwrap();
    let account = completion::cq_admission(asid).unwrap();
    let first = account.used();
    assert!(first[1] <= budget::DOMAIN_LIMIT[1]);
    let mut id = 1;
    while completion::open_cq(asid, id, 2).is_ok() {
        id += 1;
    }
    assert!(id > 1 && u64::from(id) < budget::DOMAIN_LIMIT[0]);
    assert_eq!(account.used(), [u64::from(id), u64::from(id) * first[1]]);
    let full = budget::node_used();
    assert_eq!(completion::open_cq(asid, id, 2), Err(CqOpenError::ResourceLimit));
    assert_eq!(budget::node_used(), full);
    completion::close_address_space(asid);
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(budget::node_used(), before);

    // Physical ring setup must not write before admission. The test itself
    // owns this frame until all referencing queues have been removed.
    let frame = crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().unwrap();
    let page = unsafe { frame.into_hhdm_mut::<u8>() };
    unsafe { core::ptr::write_bytes(page, 0xa5, 4096) };
    completion::open_address_space(asid, 2);
    let account = completion::cq_admission(asid).unwrap();
    assert_eq!(
        completion::open_cq_phys(asid, 0, frame, 1),
        Err(CqOpenError::Ring(CqRingError::CapacityTooSmall))
    );
    assert_eq!(
        completion::open_cq_phys(asid, 0, frame + 1usize, 2),
        Err(CqOpenError::Ring(CqRingError::FrameMisaligned))
    );
    assert_eq!(account.used(), [0, 0]);
    assert!(unsafe { core::slice::from_raw_parts(page, 4096) }.iter().all(|byte| *byte == 0xa5));
    let exhausted = budget::reserve(&account, false, budget::DOMAIN_LIMIT).unwrap();
    assert_eq!(completion::open_cq_phys(asid, 0, frame, 2), Err(CqOpenError::ResourceLimit));
    assert!(unsafe { core::slice::from_raw_parts(page, 4096) }.iter().all(|byte| *byte == 0xa5));
    drop(exhausted);
    completion::open_cq_phys(asid, 0, frame, 2).unwrap();
    assert_eq!(account.used(), [1, charged[1] - 4096]);
    let cap = completion::submit(asid, OpCode::Nop, None).unwrap();
    completion::complete(asid, cap, OpResult::Ok(91)).unwrap();
    let ring = unsafe { completion::cq_ring_of(asid, 0) }.unwrap();
    let alias = asid + 1;
    completion::open_address_space(alias, 2);
    let admitted = budget::node_used();
    assert_eq!(completion::open_cq_phys(asid, 1, frame, 2), Err(CqOpenError::RingInUse));
    assert_eq!(completion::open_cq_phys(alias, 0, frame, 2), Err(CqOpenError::RingInUse));
    assert_eq!(budget::node_used(), admitted);
    assert_eq!(unsafe { &mut *ring }.read().unwrap().result, 91);
    completion::open_cq_phys(asid, 0, frame, 2).unwrap();
    assert_eq!(account.used(), [1, charged[1] - 4096]);
    completion::close_address_space(asid);
    completion::close_address_space(alias);
    crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame).unwrap();
    assert_eq!(budget::node_used(), before);

    // Exhaust each vector dimension independently, checking ordinary reserve,
    // platform headroom, total exhaustion, and transactional local rollback.
    for dimension in 0..2 {
        let mut charges = alloc::vec::Vec::new();
        let mut remaining = budget::ORDINARY_LIMIT[dimension] - before.1[dimension];
        // Leave two count slots to force failure after partially installing
        // the five queues of a real signed domain below.
        if dimension == 0 {
            remaining -= 2;
        }
        while remaining != 0 {
            let amount = remaining.min(budget::DOMAIN_LIMIT[dimension]);
            let mut vector = [0, 0];
            vector[dimension] = amount;
            charges.push(budget::reserve(&budget::DomainBudget::new(), false, vector).unwrap());
            remaining -= amount;
        }
        if dimension == 0 {
            let handle = crate::service::loader::create_user_address_space_handle();
            crate::memory::close_user_address_space_handle(handle).unwrap();
            let full = budget::node_used();
            let image = crate::service::store::service_elf(b"ns").unwrap();
            assert!(matches!(
                crate::service::loader::try_load_domain(image),
                Err(crate::service::loader::DomainLoadError::CompletionQueue(
                    CqOpenError::ResourceLimit
                ))
            ));
            assert_eq!(budget::node_used(), full, "partial loader preparation must roll back CQs");
            let replacement = crate::service::loader::create_user_address_space_handle();
            assert_eq!(replacement.id(), handle.id(), "failed loader must free its ASID");
            crate::memory::close_user_address_space_handle(replacement).unwrap();
            let loaded = crate::service::loader::try_load_platform_domain(image).unwrap();
            assert_eq!(budget::node_used().0[0], full.0[0] + 5);
            assert_eq!(budget::node_used().1, full.1);
            crate::memory::close_user_address_space_handle(loaded.address_space).unwrap();
            assert_eq!(budget::node_used(), full);
            charges.push(budget::reserve(&budget::DomainBudget::new(), false, [2, 0]).unwrap());
        }
        let rejected = budget::DomainBudget::new();
        let mut one = [0, 0];
        one[dimension] = 1;
        let full = budget::node_used();
        assert!(budget::reserve(&rejected, false, one).is_err());
        assert_eq!(rejected.used(), [0, 0]);
        assert_eq!(budget::node_used(), full);
        let mut remaining = budget::NODE_LIMIT[dimension] - full.0[dimension];
        while remaining != 0 {
            let amount = remaining.min(budget::DOMAIN_LIMIT[dimension]);
            let mut vector = [0, 0];
            vector[dimension] = amount;
            charges.push(budget::reserve(&budget::DomainBudget::new(), true, vector).unwrap());
            remaining -= amount;
        }
        let full = budget::node_used();
        assert!(budget::reserve(&rejected, true, one).is_err());
        assert_eq!(rejected.used(), [0, 0]);
        assert_eq!(budget::node_used(), full);
        drop(charges);
        assert_eq!(budget::node_used(), before);
    }

    let handle = crate::service::loader::create_user_address_space_handle();
    completion::open_address_space_with_cq(handle.id(), 2, 2).unwrap();
    let account = completion::cq_admission(handle.id()).unwrap();
    let charged = account.used();
    crate::memory::budget::retire(handle);
    assert_eq!(completion::open_cq(handle.id(), 1, 2), Err(CqOpenError::RetiringAddressSpace));
    assert_eq!(
        completion::open_address_space_with_cq(handle.id(), 2, 2),
        Err(CqOpenError::RetiringAddressSpace)
    );
    assert_eq!(account.used(), charged);
    crate::memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(account.used(), [0, 0]);
    let replacement = crate::service::loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), handle.id());
    completion::open_address_space_with_cq(replacement.id(), 2, 2).unwrap();
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(completion::cq_admission(replacement.id()).unwrap().used(), charged);
    crate::memory::close_user_address_space_handle(replacement).unwrap();
    assert_eq!(budget::node_used(), before);
    logln!(
        "[completion queues] count/bytes, aliasing, replacement, loader rollback and retirement \
         passed"
    );
}

/// Exercises the capability-free submission path: operations identified by
/// OperationId, correlated by user data and delivered through their CQ ring.
pub fn test_detached_operations() {
    logln!("Testing capability-free (detached) completion path...");

    // Detached submission requires a CQ delivery channel.
    let no_cq_asid = 0xde7a_c400;
    completion::open_address_space(no_cq_asid, 2);
    assert_eq!(
        completion::submit_detached(no_cq_asid, 0, OpCode::Nop, 0x1111),
        Err(SubmitError::NoCompletionQueue),
        "detached submit without a CQ ring must be refused"
    );
    completion::close_address_space(no_cq_asid);

    let asid = 0xde7a_c401;
    completion::open_address_space_with_cq(asid, 3, 8).expect("CQ setup failed");
    let ring_ptr = unsafe { completion::cq_ring_of(asid, 0) }.expect("CQ ring must exist");

    // Happy path: user_data comes back as the CQ cookie, no capability slot
    // is consumed.
    let op_a = completion::submit_detached(asid, 0, OpCode::Nop, 0xaaaa_0001).unwrap();
    let op_b = completion::submit_detached(asid, 0, OpCode::Nop, 0xaaaa_0002).unwrap();
    assert_ne!(op_a, op_b, "operation ids must be distinct");

    completion::complete_detached(asid, op_a, OpResult::Ok(7)).unwrap();
    let entry = unsafe { &mut *ring_ptr }.read().expect("detached completion must post");
    assert_eq!(entry.cookie, 0xaaaa_0001, "CQ cookie must be the submitter's user_data");
    assert_eq!(entry.result, 7);

    // A completed detached operation no longer exists.
    assert_eq!(
        completion::complete_detached(asid, op_a, OpResult::Ok(8)),
        Err(completion::CapError::UnknownCap),
        "double completion of a detached operation must be rejected"
    );
    assert_eq!(
        completion::cancel_detached(asid, op_a),
        Err(completion::CapError::UnknownCap),
        "cancelling a reclaimed detached operation must be rejected"
    );

    // Cancellation forces the effective result.
    assert_eq!(completion::cancel_detached(asid, op_b).unwrap(), CancelState::CancelRequested);
    completion::complete_detached(asid, op_b, OpResult::Ok(9)).unwrap();
    let entry = unsafe { &mut *ring_ptr }.read().expect("cancelled detached must post");
    assert_eq!(entry.cookie, 0xaaaa_0002);
    assert_eq!(
        crate::completion::cq::fields_to_op_result(entry.status, entry.result),
        OpResult::Cancelled,
        "a cancel-pending detached operation must complete as Cancelled"
    );

    // Detached operations share the submission-backpressure budget with
    // capability-backed ones (capacity 3).
    let _c1 = completion::submit(asid, OpCode::Nop, None).unwrap();
    let _d1 = completion::submit_detached(asid, 0, OpCode::Nop, 1).unwrap();
    let _d2 = completion::submit_detached(asid, 0, OpCode::Nop, 2).unwrap();
    assert_eq!(
        completion::submit_detached(asid, 0, OpCode::Nop, 3),
        Err(SubmitError::WouldBlock),
        "detached submissions must respect the shared capacity"
    );
    assert_eq!(
        completion::submit(asid, OpCode::Nop, None),
        Err(SubmitError::WouldBlock),
        "capability-backed submissions must see detached load"
    );

    // Completing a detached operation frees budget again.
    completion::complete_detached(asid, _d1, OpResult::Ok(0)).unwrap();
    let _d3 = completion::submit_detached(asid, 0, OpCode::Nop, 4).unwrap();

    // --- per-shard routing: a second queue receives its own traffic ----------
    completion::complete_detached(asid, _d2, OpResult::Ok(0)).unwrap();
    completion::complete_detached(asid, _d3, OpResult::Ok(0)).unwrap();
    while unsafe { &mut *ring_ptr }.read().is_some() {}

    completion::open_cq(asid, 1, 8).expect("CQ setup failed");
    let ring1 = unsafe { completion::cq_ring_of(asid, 1) }.expect("CQ 1 ring must exist");
    let routed = completion::submit_detached(asid, 1, OpCode::Nop, 0xbbbb_0001).unwrap();
    completion::complete_detached(asid, routed, OpResult::Ok(41)).unwrap();
    assert_eq!(
        completion::cq_pending(asid, 1),
        1,
        "a detached completion must route to its selected queue"
    );
    assert_eq!(
        completion::cq_pending(asid, 0),
        0,
        "the default queue must not observe another queue's traffic"
    );
    let entry = unsafe { &mut *ring1 }.read().expect("CQ 1 entry must be present");
    assert_eq!(entry.cookie, 0xbbbb_0001);
    assert_eq!(entry.result, 41);

    // Submitting to a queue that does not exist is refused.
    assert_eq!(
        completion::submit_detached(asid, 7, OpCode::Nop, 5),
        Err(SubmitError::NoCompletionQueue),
        "detached submit to a nonexistent queue must be refused"
    );

    completion::close_address_space(asid);

    // A detached completion that cannot enter a full ring continues to count
    // against submission capacity until userspace drains space and a kernel
    // entry point flushes it. This bounds retained CQ records by that capacity.
    let bounded_asid = 0xde7a_c402;
    completion::open_address_space_with_cq(bounded_asid, 2, 2).expect("CQ setup failed");
    let bounded_ring =
        unsafe { completion::cq_ring_of(bounded_asid, 0) }.expect("bounded CQ ring must exist");
    let first = completion::submit_detached(bounded_asid, 0, OpCode::Nop, 0xb001).unwrap();
    let retained = completion::submit_detached(bounded_asid, 0, OpCode::Nop, 0xb002).unwrap();
    completion::complete_detached(bounded_asid, first, OpResult::Ok(1)).unwrap();
    completion::complete_detached(bounded_asid, retained, OpResult::Ok(2)).unwrap();

    let admitted = completion::submit_detached(bounded_asid, 0, OpCode::Nop, 0xb003).unwrap();
    assert_eq!(
        completion::submit_detached(bounded_asid, 0, OpCode::Nop, 0xb004),
        Err(SubmitError::WouldBlock),
        "an undelivered detached completion must retain its submission slot"
    );
    let first_entry = unsafe { &mut *bounded_ring }.read().expect("first CQ entry must exist");
    assert_eq!(first_entry.cookie, 0xb001);

    let retried = completion::submit_detached(bounded_asid, 0, OpCode::Nop, 0xb004)
        .expect("submission should flush retained work after userspace drains CQ space");
    let retained_entry =
        unsafe { &mut *bounded_ring }.read().expect("retained CQ entry must be flushed");
    assert_eq!(retained_entry.cookie, 0xb002);
    completion::complete_detached(bounded_asid, admitted, OpResult::Ok(3)).unwrap();
    completion::complete_detached(bounded_asid, retried, OpResult::Ok(4)).unwrap();
    completion::close_address_space(bounded_asid);
    logln!("Capability-free (detached) completion tests passed.");
}
