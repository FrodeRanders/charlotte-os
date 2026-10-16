//! Serialized admitted payload/authority controls; all heap-held probes run
//! before AP schedulers start. Raw fixture names provide no physical proof.

use super::*;
use crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR;

const CLIENT: usize = 0x5e3a;

fn allocation_and_publication() {
    open_address_space_with_cq(CLIENT, 64, 64).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let authority_before = crate::capability::node_admission_used();
    let failed = Preparing::new_with(
        Target::capture(CLIENT).unwrap(),
        Some(alloc::vec![1]),
        true,
        &OPERATION_IDS,
        Completion::new,
        |_, _| panic!("metadata rejection reached authority preparation"),
        || {
            let registry = COMPLETIONS.write();
            assert!(registry[&CLIENT].table.first_key_value().is_none());
            assert_eq!((records.used(), watches.used()), (1, 1));
            Err(core::alloc::AllocError)
        },
    );
    assert!(matches!(failed, Err(SubmitError::WouldBlock)));
    assert_eq!((records.used(), watches.used()), (0, 0));
    assert_eq!(crate::capability::node_admission_used(), authority_before);
    let mut preparations = Vec::with_capacity(64);
    let mut captured = Vec::with_capacity(64);
    let mut weak = Vec::with_capacity(64);
    let mut caps = Vec::with_capacity(64);
    let mut retired = Vec::with_capacity(64);
    for i in 0..64 {
        let mut preparing = Preparing::new_with(
            Target::capture(CLIENT).unwrap(),
            Some(alloc::vec![i]),
            false,
            &OPERATION_IDS,
            Completion::new,
            |asid, root| PreparedReservation::try_new(asid, ObjectKind::Completion, root),
            || {
                {
                    let _registry = COMPLETIONS.write();
                }
                PreparedEntry::try_new()
            },
        )
        .unwrap();
        if i % 2 != 0 {
            preparing.timer(u64::MAX, crate::timers::PreparedEvent::new, Arc::try_new).unwrap();
        }
        let completion = preparing.0.completion.as_ref().unwrap().clone();
        weak.push(Arc::downgrade(&completion));
        captured.push(completion);
        preparations.push(preparing);
    }
    assert_eq!(records.used(), 64);
    assert_eq!(
        crate::capability::node_admission_used(),
        (authority_before.0 + 64, authority_before.1 + 64)
    );
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for preparing in &mut preparations {
            caps.push(preparing.publish().unwrap());
            assert!(preparing.0.entry.is_none());
        }
        assert_eq!(COMPLETIONS.read()[&CLIENT].table.iter().count(), 64);
    }
    for mut preparing in preparations {
        preparing.enqueue();
        preparing.finish();
    }
    assert!(matches!(retire_completed(CLIENT, caps[0], &captured[0]), Err(CapError::NotComplete)));
    for cap in &caps {
        complete(CLIENT, *cap, OpResult::Ok(1)).unwrap();
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for (cap, completion) in caps.iter().zip(&captured) {
            retired.push(retire_completed(CLIENT, *cap, completion).unwrap());
        }
        let registry = COMPLETIONS.read();
        assert!(registry[&CLIENT].table.first_key_value().is_none());
        assert!(registry[&CLIENT].cqs[&DEFAULT_CQ].backlog.is_empty());
        assert_eq!(registry[&CLIENT].live, 0);
        assert_eq!(records.used(), 64);
        assert_eq!(
            crate::capability::node_admission_used(),
            (authority_before.0 + 64, authority_before.1 + 64)
        );
    }
    for owner in retired {
        owner.finish();
    }
    assert_eq!(crate::capability::node_admission_used(), authority_before);
    assert!(captured.iter().all(|completion| completion.holds_buffer()));
    drop(captured);
    assert_eq!(timer_events_used(CLIENT), 0);
    assert_eq!(records.used(), 64, "weak backing lost its original record charge");
    assert!(weak.iter().all(|completion| completion.upgrade().is_none()));
    drop(weak);
    assert_eq!(records.used(), 0);
    for cap in caps {
        assert_eq!(close(CLIENT, cap), Err(CapError::UnknownCap));
    }
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

fn abort_backlog() {
    open_address_space_with_cq(CLIENT, 2, 2).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let filler = crate::completion::submit(CLIENT, OpCode::Nop, None).unwrap();
    complete(CLIENT, filler, OpResult::Ok(0)).unwrap();
    let original_authority = crate::capability::node_admission_used();
    for event in [false, true] {
        for _ in 0..64 {
            let submission = event.then(|| EventSubmission::new(CLIENT).unwrap());
            let cap = submission.as_ref().map_or_else(
                || crate::completion::submit(CLIENT, OpCode::Nop, None).unwrap(),
                EventSubmission::cap,
            );
            complete(CLIENT, cap, OpResult::Ok(1)).unwrap();
            assert_eq!(COMPLETIONS.read()[&CLIENT].cqs[&DEFAULT_CQ].backlog.len(), 1);
            if let Some(submission) = submission {
                drop(submission);
            } else {
                abort_submission(CLIENT, cap).unwrap();
            }
            let registry = COMPLETIONS.read();
            assert!(registry[&CLIENT].cqs[&DEFAULT_CQ].backlog.is_empty());
            assert_eq!(registry[&CLIENT].live, 1);
            assert_eq!(records.used(), 1);
            assert_eq!(crate::capability::node_admission_used(), original_authority);
        }
    }
    close(CLIENT, filler).unwrap();
    assert_eq!(records.used(), 0);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

fn namespace_identity() {
    open_address_space_with_cq(CLIENT, 4, 4).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let cap = crate::completion::submit(CLIENT, OpCode::Read, Some(alloc::vec![7])).unwrap();
    let timer = crate::completion::submit_timer(CLIENT, u64::MAX).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let weak = Arc::downgrade(&captured);
    drop(captured);
    let mut previous = COMPLETIONS.write().remove(&CLIENT).unwrap();
    let authority = crate::capability::node_admission_used();
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        revoke_namespace_authority(CLIENT, &mut previous);
        assert!(!crate::capability::contains(CLIENT, cap, ObjectKind::Completion));
        assert!(!crate::capability::contains(CLIENT, timer, ObjectKind::Completion));
        assert_eq!(crate::capability::node_admission_used(), authority);
        assert_eq!(records.used(), 2);
    }
    crate::capability::close_address_space(CLIENT);
    open_address_space_with_cq(CLIENT, 4, 4).unwrap();
    let successor = crate::completion::submit(CLIENT, OpCode::Nop, None).unwrap();
    assert_eq!(successor, cap);
    let successor_records = record_admission(CLIENT).unwrap();
    previous.finish();
    assert!(crate::capability::contains(CLIENT, successor, ObjectKind::Completion));
    assert_eq!(successor_records.used(), 1);
    assert_eq!(records.used(), 1);
    assert!(weak.upgrade().is_none());
    drop(weak);
    assert_eq!(records.used(), 0);
    close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
}

fn guarded_abandonment() {
    // Deliberate bounded quarantine uses private raw namespaces, no live roots
    // or sponsor replacements. Both new record/authority charges remain live.
    open_address_space_with_cq(CLIENT, 4, 4).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let watches = watch_admission(CLIENT).unwrap();
    let timers = COMPLETIONS.read()[&CLIENT].timer_budget.clone();
    let mut preparing = Preparing::new(
        Target::capture(CLIENT).unwrap(),
        Some(alloc::vec![9]),
        true,
        &OPERATION_IDS,
    )
    .unwrap();
    preparing.timer(u64::MAX, crate::timers::PreparedEvent::new, Arc::try_new).unwrap();
    let weak = Arc::downgrade(preparing.0.completion.as_ref().unwrap());
    let cap = preparing.0.reservation.as_ref().unwrap().identity();
    let authority = crate::capability::node_admission_used();
    close_address_space(CLIENT);
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(preparing);
    }
    assert_eq!((records.used(), watches.used(), timers.used()), (1, 1, 1));
    assert_eq!(crate::capability::node_admission_used(), authority);
    assert!(!crate::capability::contains(CLIENT, cap, ObjectKind::Completion));
    assert!(weak.upgrade().unwrap().holds_buffer());
    // Leave that original hidden authority namespace retained. A distinct raw
    // namespace exercises complete retired-node abandonment and numeric reuse.
    let other = CLIENT + 1;
    open_address_space_with_cq(other, 4, 4).unwrap();
    let records = record_admission(other).unwrap();
    let cap = crate::completion::submit(other, OpCode::Read, Some(alloc::vec![8])).unwrap();
    let completion = completion_of(other, cap).unwrap();
    let weak = Arc::downgrade(&completion);
    complete(other, cap, OpResult::Ok(0)).unwrap();
    let retired = retire_completed(other, cap, &completion).unwrap();
    drop(completion);
    close_address_space(other);
    crate::capability::close_address_space(other);
    open_address_space_with_cq(other, 4, 4).unwrap();
    let successor = crate::completion::submit(other, OpCode::Nop, None).unwrap();
    assert_eq!(successor, cap);
    let authority = crate::capability::node_admission_used();
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(retired);
    }
    assert_eq!(records.used(), 1);
    assert!(weak.upgrade().unwrap().holds_buffer());
    assert_eq!(crate::capability::node_admission_used(), authority);
    assert!(crate::capability::contains(other, successor, ObjectKind::Completion));
    close_address_space(other);
    crate::capability::close_address_space(other);
    crate::logln!(
        "[capability metadata retention] guarded prepared/retired abandonment retained payload, \
         hidden/detached authority, original charges, timer, watch and buffer; numeric successor \
         unchanged"
    );
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    allocation_and_publication();
    abort_backlog();
    namespace_identity();
    guarded_abandonment();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[capability metadata] unlocked allocation/rejection, 64 heap-held publications and \
         paired payload/authority removals, 128 full-ring aborts, weak-only original charges and \
         namespace identity passed"
    );
}
