//! Exact-episode exit-watch preparation/publication fixtures.

use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::{
    system_scheduler::Error,
    threads::exit_source::PreparingExitWatch,
    *,
};
use crate::{
    completion::watch_budget,
    klib::observer::{
        CallOnNotify,
        list_budget,
        registration::{
            ObserverList,
            RegistrationError,
        },
    },
    memory::{
        KERNEL_ASID,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

extern "C" fn unused_entry() {}

/// Never scheduler-admitted; removal retains the complete exact owner until
/// explicit cleanup after the table guard leaves.
struct Target {
    tid: ThreadId,
    generation: ThreadGeneration,
}

impl Target {
    fn new() -> Self {
        let thread = Thread::new(KERNEL_ASID, unused_entry);
        let generation = thread.generation;
        Self {
            tid: publish_thread(thread).unwrap(),
            generation,
        }
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        let thread = MASTER_THREAD_TABLE.write().take_element(self.tid).unwrap();
        assert_eq!(thread.generation, self.generation);
        thread.release_unstarted().unwrap();
    }
}

fn reenter_table(irq: bool) {
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    // Blocking reentry is intentional: remote contention is legal, whereas a
    // caller-owned write guard would deadlock. No global try-lock premise.
    drop(MASTER_THREAD_TABLE.write());
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
}

/// Serialized boot probes check counters and reuse; concurrent pressure needs separate tests.
pub(crate) fn run() {
    let lists = list_budget::node_used();
    let account = watch_budget::DomainBudget::new(4);
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let irq = crate::cpu::isa::lp::ops::get_int_state();

    for prepared in [false, true] {
        let target = Target::new();
        let rejected = super::observe_thread_exit_matching_with(
            target.tid,
            Some(target.generation),
            None,
            Arc::downgrade(&observer),
            watch_budget::reserve(&account, false).unwrap(),
            |preparation| {
                reenter_table(irq);
                if prepared {
                    preparation.prepare()?;
                }
                let _heap = PRIMARY_ALLOCATOR.lock();
                let _table = MASTER_THREAD_TABLE.write();
                assert_eq!(account.used(), 1);
                assert_eq!(Arc::weak_count(&observer), 1);
                Err(RegistrationError::AllocationFailed)
            },
        );
        assert!(matches!(rejected, Err(Error::WaitRegistrationFailed)));
        assert_eq!(MASTER_THREAD_TABLE.read().get(target.tid).unwrap().exit_watch_count(), 0);
        assert_eq!(account.used(), 0);
        drop(target);
    }

    // Both legacy TID-only and generation-bound callers retain the initially
    // captured episode across storage preparation, never a replacement.
    for qualified in [false, true] {
        let target = Target::new();
        let tid = target.tid;
        let generation = target.generation;
        let mut replacement = None;
        let stale = super::observe_thread_exit_matching_with(
            tid,
            qualified.then_some(generation),
            None,
            Arc::downgrade(&observer),
            watch_budget::reserve(&account, false).unwrap(),
            |preparation| {
                reenter_table(irq);
                drop(target);
                let next = Target::new();
                assert_eq!(next.tid, tid);
                assert_ne!(next.generation, generation);
                replacement = Some(next);
                preparation.prepare()
            },
        );
        assert!(matches!(stale, Err(Error::InvalidThread)));
        assert_eq!(account.used(), 0);
        assert_eq!(MASTER_THREAD_TABLE.read().get(tid).unwrap().exit_watch_count(), 0);
        drop(replacement);
    }

    // Another registration may win first-source publication during preparation.
    // Both entries must share that winner, releasing the unused charged list.
    let target = Target::new();
    let before = list_budget::node_used();
    let mut winner = None;
    let token = super::observe_thread_exit_matching_with(
        target.tid,
        Some(target.generation),
        None,
        Arc::downgrade(&observer),
        watch_budget::reserve(&account, false).unwrap(),
        |preparation| {
            reenter_table(irq);
            preparation.prepare()?;
            winner = Some(
                super::observe_thread_exit_with_generation(
                    target.tid,
                    target.generation,
                    Arc::downgrade(&observer),
                    watch_budget::reserve(&account, false).unwrap(),
                )
                .unwrap(),
            );
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(list_budget::node_used(), (before.0 + 1, before.1 + 1));
    assert_eq!(MASTER_THREAD_TABLE.read().get(target.tid).unwrap().exit_watch_count(), 2);
    drop((token, winner));
    assert_eq!(account.used(), 0);
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    drop(target);

    // A foreign-domain request rejects before any storage preparation.
    let root = crate::service::loader::create_user_address_space_handle();
    let target = Target::new();
    let denied = super::observe_thread_exit_matching_with(
        target.tid,
        Some(target.generation),
        Some(root),
        Arc::downgrade(&observer),
        watch_budget::reserve(&account, false).unwrap(),
        |_| panic!("unauthorized exit watch reached preparation"),
    );
    assert!(matches!(denied, Err(Error::PermissionDenied)));
    assert_eq!(account.used(), 0);
    drop(target);
    crate::memory::close_user_address_space_handle(root).unwrap();

    // Real publication and source rejection with the heap held: neither may
    // allocate/free an entry/list, drop its weak reference or refund its charge.
    for closed in [false, true] {
        let target = Target::new();
        let list = ObserverList::try_new(watch_budget::MAX_THREAD_WATCHES, false).unwrap();
        if closed {
            drop(list.close());
        }
        let mut preparation = PreparingExitWatch::new(
            Some(list),
            Arc::downgrade(&observer),
            watch_budget::reserve(&account, false).unwrap(),
        )
        .unwrap();
        let result = {
            let _heap = PRIMARY_ALLOCATOR.lock();
            let table = MASTER_THREAD_TABLE.write();
            table.get(target.tid).unwrap().observe_exit_prepared(&mut preparation)
        };
        assert_eq!(account.used(), 1);
        preparation.finish();
        if closed {
            assert!(matches!(result, Err(RegistrationError::Closed)));
            assert_eq!(account.used(), 0);
        } else {
            drop(result.unwrap());
            assert_eq!(account.used(), 0);
        }
        drop(target);
    }
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    assert_eq!(Arc::weak_count(&observer), 0);
    assert_eq!(account.used(), 0);
    assert_eq!(list_budget::node_used(), lists);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[exit watch preparation] allocation rejection, exact-generation reuse, competing source, \
         authorization and heap-held publication/rejection passed"
    );
    threads::exit_source::test_preparation();
}

/// Live scheduled entry checks reentry and both caller IRQ policies without
/// comparing global counters or requiring global lock availability.
pub(crate) fn scheduled() {
    use crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask;
    let account = watch_budget::DomainBudget::new(1);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("cancelled watch notified"));
    let target = Target::new();
    let original_irq = crate::cpu::isa::lp::ops::get_int_state();
    assert!(original_irq, "scheduled exit-watch fixture requires normal IRQ-enabled entry");
    for masked in [false, true] {
        let outer = masked.then(LocalInterruptMask::new);
        let irq = crate::cpu::isa::lp::ops::get_int_state();
        let token = super::observe_thread_exit_matching_with(
            target.tid,
            Some(target.generation),
            None,
            Arc::downgrade(&observer),
            watch_budget::reserve(&account, false).unwrap(),
            |preparation| {
                reenter_table(irq);
                preparation.prepare()
            },
        )
        .unwrap();
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
        drop(token);
        assert_eq!(account.used(), 0);
        drop(outer);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), original_irq);
    }
    drop(target);
    crate::logln!(
        "[exit watch preparation] scheduled table reentry and caller IRQ preservation passed"
    );
}
