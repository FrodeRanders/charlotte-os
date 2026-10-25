//! Real captured records and owning callback/source metadata; heap-held
//! publication and registry-reentrant disposal qualify only these local guards.

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

const CLIENT: usize = 0x5e31;

fn prepare(asid: usize, cap: CompletionCap, observer: &Arc<dyn Observer>) -> Preparing {
    let captured = completion_of(asid, cap).unwrap();
    Preparing::new(Captured::new(asid, cap, &captured).unwrap(), Arc::downgrade(observer)).unwrap()
}

fn publication() {
    open_address_space(CLIENT, 256);
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![1, 2, 3])).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let account = watch_admission(CLIENT).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let weak = Arc::downgrade(&captured);
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        let _registry = COMPLETIONS.write();
        if let Some(completion) = weak.upgrade() {
            let _inner = completion.inner.lock();
        }
        counter.fetch_add(1, Ordering::SeqCst);
    });
    // Inject rejection at actual list preparation and prove that allocation
    // can reenter both guards. Charge rollback happens after this closure.
    let rejected = Preparing::new_with(
        Captured::new(CLIENT, cap, &captured).unwrap(),
        Arc::downgrade(&observer),
        |_| {
            let _registry = COMPLETIONS.write();
            let _inner = captured.inner.lock();
            Err(RegistrationError::AllocationFailed)
        },
        Preparing::entry,
    );
    assert!(matches!(rejected, Err(ObserveError::ResourceLimit)));
    assert_eq!(account.used(), 0);
    assert!(captured.inner.lock().callbacks.is_none());
    let mut first = Preparing::new_with(
        Captured::new(CLIENT, cap, &captured).unwrap(),
        Arc::downgrade(&observer),
        |platform| {
            {
                let _registry = COMPLETIONS.write();
                let _inner = captured.inner.lock();
            }
            ObserverList::try_new(watch_budget::MAX_COMPLETION_CALLBACKS, platform)
        },
        Preparing::entry,
    )
    .unwrap();
    let mut second = prepare(CLIENT, cap, &observer);
    let token1;
    let token2;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        token1 = first.publish().unwrap().unwrap();
        token2 = second.publish().unwrap().unwrap();
        let inner = captured.inner.lock();
        assert_eq!(inner.callbacks.as_ref().unwrap().registered(), 2);
        assert!(!core::ptr::eq(
            &**inner.callbacks.as_ref().unwrap(),
            &**second.0.candidate.as_ref().unwrap(),
        ));
    }
    assert_eq!(account.used(), 2);
    first.finish();
    second.finish();
    drop((token1, token2));
    assert_eq!(account.used(), 0);
    assert!(captured.holds_buffer());
    assert_eq!(captured.state_kind(), OpStateKind::InFlight);

    let mut owners = Vec::new();
    for _ in 0..watch_budget::MAX_COMPLETION_CALLBACKS {
        owners.push(observe(CLIENT, cap, observer.clone()).unwrap());
    }
    let mut full = prepare(CLIENT, cap, &observer);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(full.publish(), Err(ObserveError::ResourceLimit)));
    }
    assert_eq!(account.used(), watch_budget::MAX_COMPLETION_CALLBACKS + 1);
    full.finish();
    assert_eq!(account.used(), watch_budget::MAX_COMPLETION_CALLBACKS);
    drop(owners);
    assert_eq!(account.used(), 0);

    let mut ready = prepare(CLIENT, cap, &observer);
    complete(CLIENT, cap, OpResult::Ok(7)).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(ready.publish().unwrap().is_none());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(account.used(), 1);
    ready.finish();
    assert_eq!(account.used(), 0);
    // Public terminal delivery reenters both guards exactly once, with no
    // prepared entry remaining charged. Observing still does not take data.
    let owner = observe(CLIENT, cap, observer.clone()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(captured.holds_buffer());
    assert_eq!(captured.state_kind(), OpStateKind::Completed);
    drop(owner);
    close(CLIENT, cap).unwrap();

    let pending = submit(CLIENT, OpCode::Nop, None).unwrap();
    let mut stale = prepare(CLIENT, pending, &observer);
    close_address_space(CLIENT);
    open_address_space(CLIENT, 4);
    let fresh = submit(CLIENT, OpCode::Nop, None).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(stale.publish(), Err(ObserveError::UnknownAddressSpace)));
    }
    assert_eq!(account.used(), 1);
    assert_eq!(watch_admission(CLIENT).unwrap().used(), 0);
    stale.finish();
    assert_eq!(account.used(), 0);
    assert_eq!(state_of(CLIENT, fresh).unwrap(), OpStateKind::InFlight);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    close_address_space(CLIENT);

    // Retirement after capture/preparation fences publication too. No live
    // sponsor is substituted and the original admission refunds after unlock.
    let root = crate::service::loader::create_user_address_space_handle();
    open_address_space(root.id(), 4);
    let cap = submit(root.id(), OpCode::Nop, None).unwrap();
    let account = watch_admission(root.id()).unwrap();
    let mut retired = prepare(root.id(), cap, &observer);
    crate::memory::budget::retire(root);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(retired.publish(), Err(ObserveError::UnknownAddressSpace)));
    }
    assert_eq!(account.used(), 1);
    retired.finish();
    assert_eq!(account.used(), 0);
    crate::memory::close_user_address_space_handle(root).unwrap();
}

struct DisposalProbe {
    completion: CompletionWeak,
    hits: Arc<AtomicUsize>,
    expect_namespace: bool,
}

impl Observer for DisposalProbe {
    fn notify(self: Arc<Self>) {
        panic!("namespace discard fabricated a completion");
    }
}

impl Drop for DisposalProbe {
    fn drop(&mut self) {
        let registry = COMPLETIONS.write();
        assert_eq!(registry.contains_key(&CLIENT), self.expect_namespace);
        let completion =
            self.completion.upgrade().expect("detached namespace lost original record");
        let _inner = completion.inner.lock();
        self.hits.fetch_add(1, Ordering::SeqCst);
    }
}

fn namespace_disposal() {
    for mode in 0..3 {
        open_address_space(CLIENT, 4);
        let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![9])).unwrap();
        let captured = completion_of(CLIENT, cap).unwrap();
        let account = watch_admission(CLIENT).unwrap();
        let records = record_admission(CLIENT).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let observer: Arc<dyn Observer> = Arc::new(DisposalProbe {
            completion: Arc::downgrade(&captured),
            hits: hits.clone(),
            expect_namespace: mode != 0,
        });
        let source = ObserverList::try_new(1, false).unwrap();
        let registration = source
            .register(Arc::downgrade(&observer), watch_budget::reserve(&account, false).unwrap())
            .unwrap();
        assert!(!captured.set_event_observation(observer, registration).finish());
        match mode {
            0 => close_address_space(CLIENT),
            1 => open_address_space(CLIENT, 4),
            _ => open_address_space_with_cq(CLIENT, 4, 8).unwrap(),
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(source.registered(), 0);
        assert_eq!(account.used(), 0);
        assert_eq!(records.used(), 1, "retained record must retain its original charge");
        assert_eq!(captured.state_kind(), OpStateKind::InFlight);
        assert!(captured.holds_buffer());
        drop(captured);
        assert_eq!(records.used(), 0);
        close_address_space(CLIENT);
    }
}

fn cq_detachment() {
    // Private complete namespace; its real admitted rings share its original
    // account. Exercise the production splice under the actual registry/heap
    // without substituting any live root, account or ring consumer.
    let mut entries = empty_as(CLIENT, 8);
    let first = delivery::Prepared::heap_for_namespace(CLIENT, &entries, 8);
    let second = delivery::Prepared::heap_for_namespace(CLIENT, &entries, 8);
    entries.cqs.insert(first.0, 0, first.1);
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("replaced CQ fabricated readiness"));
    let list = entries.cqs[&0].waiters.clone();
    let token = sponsor.register(&list, Arc::downgrade(&observer)).unwrap();
    let previous;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _registry = COMPLETIONS.write();
        previous = replace_cq(&mut entries, 0, second.0, second.1);
        assert_eq!(sponsor.used(), 1);
        assert_eq!(entries.cq_budget.used()[0], 2);
    }
    if let Some(previous) = previous {
        previous.finish();
    }
    assert_eq!(sponsor.used(), 0);
    assert_eq!(entries.cq_budget.used()[0], 1);
    assert_eq!(list.registered(), 0);
    drop(token);
    entries.finish();
    // A normal public CQ replacement cancels an old source too.
    open_address_space_with_cq(CLIENT, 4, 8).unwrap();
    let interest = capture_cq_interest(CLIENT, 0).unwrap();
    let token = sponsor.register(&interest.identity, Arc::downgrade(&observer)).unwrap();
    open_cq(CLIENT, 0, 8).unwrap();
    assert_eq!(sponsor.used(), 0);
    assert_eq!(cq_waiter_count(CLIENT, 0), 0);
    drop(token);
    close_address_space(CLIENT);
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    publication();
    namespace_disposal();
    cq_detachment();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[completion callback preparation] unlocked allocation/rejection, heap-held \
         competing/full/ready/retired/reused publication, reentrant namespace disposal and \
         complete charged CQ detachment passed"
    );
}
