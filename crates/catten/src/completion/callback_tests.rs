//! Owning kernel callbacks and the final boot-status waiter migrations.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::{
    ObserveError,
    OpCode,
    OpResult,
    watch_budget as budget,
};
use crate::klib::observer::{
    CallOnNotify,
    Observer,
};

const CLIENT: usize = 0x5e00;

fn callback_count(asid: usize, cap: super::CompletionCap) -> usize {
    super::completion_of(asid, cap)
        .unwrap()
        .inner
        .lock()
        .callbacks
        .as_ref()
        .map_or(0, |list| list.registered())
}

pub(crate) fn test_admission() {
    crate::klib::observer::registration::test_entry_allocation_rollback();
    crate::service::launch::test_publication_waiter_admission();
    crate::self_test::results::test_waiter_admission();
    let baseline = budget::node_used();
    super::open_address_space(CLIENT, 129);
    let account = super::watch_admission(CLIENT).unwrap();
    let records = super::record_admission(CLIENT).unwrap();
    let cap = super::submit(CLIENT, OpCode::Read, Some(alloc::vec![1, 2, 3])).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let callback: Arc<dyn Observer> = CallOnNotify::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let mut observations = Vec::new();
    for _ in 0..budget::MAX_COMPLETION_CALLBACKS {
        observations.push(super::observe(CLIENT, cap, callback.clone()).unwrap());
    }
    assert_eq!(callback_count(CLIENT, cap), 128);
    assert_eq!(
        super::observe(CLIENT, cap, callback.clone()).unwrap_err(),
        ObserveError::ResourceLimit
    );
    assert_eq!(account.used(), 128);
    assert_eq!(records.used(), 1);
    observations.truncate(64);
    assert_eq!(callback_count(CLIENT, cap), 64);
    assert_eq!(account.used(), 64);
    for _ in 0..64 {
        observations.push(super::observe(CLIENT, cap, callback.clone()).unwrap());
    }
    drop(observations);
    assert_eq!(account.used(), 0);
    assert_eq!(callback_count(CLIENT, cap), 0);
    assert!(super::holds_buffer(CLIENT, cap).unwrap());
    assert_eq!(super::state_of(CLIENT, cap).unwrap(), super::OpStateKind::InFlight);
    for _ in 0..512 {
        drop(super::observe(CLIENT, cap, callback.clone()).unwrap());
        assert_eq!(callback_count(CLIENT, cap), 0);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let batch_cap = super::submit(CLIENT, OpCode::Nop, None).unwrap();
    let owners: Vec<_> =
        (0..128).map(|_| super::observe(CLIENT, batch_cap, callback.clone()).unwrap()).collect();
    super::complete(CLIENT, batch_cap, OpResult::Ok(0)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 128);
    assert_eq!(account.used(), 0, "retained callback tokens must not retain detached entries");
    drop(owners);
    super::close(CLIENT, batch_cap).unwrap();
    calls.store(0, Ordering::SeqCst);

    // Notification frees entries before reentrant callbacks; terminal/observed
    // late registrations notify immediately and cannot miss a drained source.
    let counter = calls.clone();
    let reentrant: Arc<dyn Observer> = CallOnNotify::new(move || {
        assert_eq!(callback_count(CLIENT, cap), 0);
        let other = super::submit(CLIENT, OpCode::Nop, None).unwrap();
        super::complete(CLIENT, other, OpResult::Ok(1)).unwrap();
        super::close(CLIENT, other).unwrap();
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let owner = super::observe(CLIENT, cap, reentrant).unwrap();
    super::complete(CLIENT, cap, OpResult::Ok(3)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(account.used(), 0);
    super::complete(CLIENT, cap, OpResult::Err(1)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(super::observe(CLIENT, cap, callback.clone()).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(super::poll(CLIENT, cap).unwrap().unwrap().buffer.unwrap(), [1, 2, 3]);
    drop(super::observe(CLIENT, cap, callback.clone()).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    super::close(CLIENT, cap).unwrap();
    assert_eq!(records.used(), 0, "a callback token must not retain its operation object");
    drop(owner);
    assert_eq!(
        super::observe(CLIENT, cap, callback.clone()).unwrap_err(),
        ObserveError::UnknownCap
    );
    super::close_address_space(CLIENT);
    assert_eq!(
        super::observe(CLIENT, cap, callback.clone()).unwrap_err(),
        ObserveError::UnknownAddressSpace
    );

    // One watch pool is shared with lifecycle watches. Ready callbacks need no
    // list entry and can complete even while an unrelated operation fills it.
    super::open_address_space(CLIENT, 2);
    let account = super::watch_admission(CLIENT).unwrap();
    let first = super::submit(CLIENT, OpCode::Nop, None).unwrap();
    let second = super::submit(CLIENT, OpCode::Nop, None).unwrap();
    let owner1 = super::observe(CLIENT, first, callback.clone()).unwrap();
    let charge = budget::reserve(&account, false).unwrap();
    assert_eq!(
        super::observe(CLIENT, second, callback.clone()).unwrap_err(),
        ObserveError::ResourceLimit
    );
    drop(charge);
    let owner2 = super::observe(CLIENT, first, callback.clone()).unwrap();
    super::complete(CLIENT, second, OpResult::Ok(0)).unwrap();
    let before = calls.load(Ordering::SeqCst);
    drop(super::observe(CLIENT, second, callback.clone()).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), before + 1);
    assert_eq!(account.used(), 2);
    drop((owner1, owner2));
    // Cancelling an operation is distinct from cancelling its subscription.
    let owner = super::observe(CLIENT, first, callback.clone()).unwrap();
    super::cancel(CLIENT, first).unwrap();
    assert_eq!(account.used(), 1);
    assert!(super::poll(CLIENT, first).unwrap().is_none());
    super::complete(CLIENT, first, OpResult::Ok(0)).unwrap();
    assert_eq!(super::poll(CLIENT, first).unwrap().unwrap().result, OpResult::Cancelled);
    assert_eq!(account.used(), 0);
    drop(owner);
    super::close(CLIENT, first).unwrap();
    super::close(CLIENT, second).unwrap();

    // Aborted unpublished records discard their callbacks even when an old
    // completion/token survives rollback. No spurious terminal callback runs.
    let cap = super::submit(CLIENT, OpCode::Nop, None).unwrap();
    let retained = super::completion_of(CLIENT, cap).unwrap();
    let owner = super::observe(CLIENT, cap, callback.clone()).unwrap();
    let before = calls.load(Ordering::SeqCst);
    super::abort_submission(CLIENT, cap).unwrap();
    assert_eq!(account.used(), 0);
    drop((retained, owner));
    assert_eq!(calls.load(Ordering::SeqCst), before);

    // Namespace replacement releases callback entries despite retained objects;
    // the old generation cannot lend its charge to a replacement with the same
    // ASID and capability number, and retirement forbids new subscriptions.
    super::close_address_space(CLIENT);
    let handle = crate::service::loader::create_user_address_space_handle();
    super::open_address_space(handle.id(), 2);
    let cap = super::submit(handle.id(), OpCode::Nop, None).unwrap();
    let old_account = super::watch_admission(handle.id()).unwrap();
    let old_records = super::record_admission(handle.id()).unwrap();
    let retained = super::completion_of(handle.id(), cap).unwrap();
    let owner = super::observe(handle.id(), cap, callback.clone()).unwrap();
    crate::memory::budget::retire(handle);
    assert_eq!(
        super::observe(handle.id(), cap, callback.clone()).unwrap_err(),
        ObserveError::UnknownAddressSpace
    );
    crate::memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(old_account.used(), 0);
    assert_eq!(old_records.used(), 1);
    let replacement = crate::service::loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), handle.id());
    assert_ne!(replacement.generation(), handle.generation());
    super::open_address_space(replacement.id(), 2);
    let fresh = super::submit(replacement.id(), OpCode::Nop, None).unwrap();
    assert_eq!(fresh, cap);
    let new_account = super::watch_admission(replacement.id()).unwrap();
    let fresh_owner = super::observe(replacement.id(), fresh, callback.clone()).unwrap();
    assert_eq!(
        super::observe_registered(handle.id(), cap, &retained, callback.clone()).unwrap_err(),
        ObserveError::UnknownCap
    );
    drop((owner, retained));
    assert_eq!(old_records.used(), 0);
    assert_eq!(new_account.used(), 1);
    assert!(super::poll(replacement.id(), fresh).unwrap().is_none());
    drop(fresh_owner);
    super::complete(replacement.id(), fresh, OpResult::Ok(0)).unwrap();
    super::close(replacement.id(), fresh).unwrap();
    crate::memory::close_user_address_space_handle(replacement).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), before);
    assert_eq!(budget::node_used(), baseline);
    crate::logln!(
        "[completion callbacks] SUCCESS: owning cancellation, source/shared admission, \
         late/reentrant notification, abort rollback, retirement and exact namespace reuse"
    );
}

pub(crate) fn test_scheduled_cleanup() {
    crate::self_test::waiters::with_isolated_sponsor(|sponsor| {
        crate::service::launch::test_publication_waiter_cleanup(sponsor);
        crate::self_test::results::test_waiter_cleanup(sponsor);
    });
    // A real immediate-return worker may become terminal either before or
    // after registration. Both paths must deliver one callback, never zero/two.
    extern "C" fn immediate_worker() {}
    let client = 0x5e01;
    super::open_address_space(client, 4);
    let account = super::watch_admission(client).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    for expected in 1..=32 {
        let cap = super::submit_worker(client, immediate_worker, OpResult::Ok(0)).unwrap();
        let counter = calls.clone();
        let callback: Arc<dyn Observer> = CallOnNotify::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let owner = super::observe(client, cap, callback).unwrap();
        assert!(super::wait_timeout(client, cap, 5_000).unwrap());
        let deadline = crate::cpu::scheduler::monotonic_millis() + 5_000;
        while calls.load(Ordering::SeqCst) < expected {
            assert!(crate::cpu::scheduler::monotonic_millis() < deadline);
            crate::cpu::scheduler::yield_lp();
        }
        assert_eq!(calls.load(Ordering::SeqCst), expected);
        drop(owner);
        super::close(client, cap).unwrap();
    }
    assert_eq!(account.used(), 0);
    super::close_address_space(client);
    crate::logln!(
        "[status waiters] SUCCESS: 64 publication plus 64 result timeout cycles and 32 real \
         worker callbacks reconcile owning admission without weak-entry pruning"
    );
}
