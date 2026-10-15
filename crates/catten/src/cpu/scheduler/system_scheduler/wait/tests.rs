//! Serialized never-dispatched owners exercise the real capture/publication,
//! source callback, queue and cancellation paths. No fabricated exit proof.

use alloc::sync::Weak;

use super::*;
use crate::{
    cpu::multiprocessor::interrupt_tracking::LocalInterruptMask,
    klib::observer::{
        WaitRegistration,
        WaitSponsor,
        registration::RegistrationError,
        waiter_source::WaiterSource,
    },
    memory::{
        KERNEL_ASID,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

struct Source<'a> {
    scheduler: &'a SystemScheduler,
    source: WaiterSource,
    reject: bool,
    ready: bool,
}

impl Observable for Source<'_> {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        {
            // Acquire real guards: remote contention is legitimate and must
            // not be mistaken for a recursive hold by this producer LP.
            let _lp = self.scheduler.get_lp_scheduler().lock();
            let _table = MASTER_THREAD_TABLE.write();
        }
        if self.reject {
            Err(RegistrationError::ResourceLimit)
        } else if self.ready {
            Ok(WaitRegistration::ready())
        } else {
            self.source.register(observer, sponsor)
        }
    }
}

extern "C" fn unused_entry() {
    panic!("serialized wait fixture must never dispatch");
}

fn fixture(sponsor: &WaitSponsor) -> Thread {
    let mut thread = Thread::new(KERNEL_ASID, unused_entry);
    thread.affinity_lp = Some(get_lp_id());
    // Isolate accounting before publication, retaining kernel classification
    // and normal node admission; never substitute a live thread's account.
    thread.wait_sponsor = sponsor.clone();
    thread
}

fn take_fixture(scheduler: &SystemScheduler, tid: ThreadId) -> Thread {
    let mut lp = scheduler.get_lp_scheduler().lock();
    let mut table = MASTER_THREAD_TABLE.write();
    let generation = table.get(tid).unwrap().generation;
    if matches!(table.get(tid).unwrap().state, ThreadState::Ready(_)) {
        lp.remove_thread(tid, Some(generation), &mut table).unwrap();
    }
    let thread = table.take_element(tid).unwrap();
    assert!(!thread.run_queue_link.is_linked());
    thread
}

fn release(mut thread: Thread) {
    assert!(!matches!(thread.state, ThreadState::Blocked(_)));
    // The real queue was explicitly unlinked and this fixture never dispatched.
    thread.state = ThreadState::NeedsLpAssignment;
    thread.release_unstarted().unwrap();
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let _setup = LocalInterruptMask::new();
    let scheduler = SYSTEM_SCHEDULER.read();
    let sponsor = WaitSponsor::new(true);
    let source = Source {
        scheduler: &scheduler,
        source: WaiterSource::new(),
        reject: false,
        ready: false,
    };
    let tid = publish_thread(fixture(&sponsor)).unwrap();
    let constraint = MigrationConstraint::GeneralWait;
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;

    // Rejection and immediate readiness do not change the original owner.
    for (reject, ready) in [(true, false), (false, true)] {
        let source = Source {
            scheduler: &scheduler,
            source: WaiterSource::new(),
            reject,
            ready,
        };
        let result = scheduler.block_thread_with_constraint_generation(tid, &source, constraint);
        if reject {
            assert!(matches!(result, Err(Error::WaitRegistrationFailed)));
        } else {
            assert_eq!(result.unwrap(), generation);
        }
        let table = MASTER_THREAD_TABLE.read();
        assert!(matches!(table.get(tid).unwrap().state, ThreadState::NeedsLpAssignment));
        assert_eq!(table.get(tid).unwrap().migration_constraints, 0);
        assert_eq!(sponsor.used(), 0);
    }

    // Source fires between registration and Blocked publication. The ordinary
    // callback cannot admit an uninstalled wait; its remembered hint prevents
    // parking. Heap hold proves publication/rejection destroys no metadata.
    let early = scheduler.prepare_wait(tid, &source).unwrap();
    early.waker.clone().notify();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(!scheduler.publish_wait(&early, constraint).unwrap());
    }
    early.waker.cancel_registration();
    assert_eq!(sponsor.used(), 0);

    // Source fires after publication: real wake admission queues this exact
    // thread and cancels its registration despite our retained strong Waker.
    let first = scheduler.prepare_wait(tid, &source).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(scheduler.publish_wait(&first, constraint).unwrap());
    }
    first.waker.clone().notify();
    assert_eq!(source.source.registered(), 0);
    assert_eq!(sponsor.used(), 0);
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert!(matches!(thread.state, ThreadState::Ready(lp) if lp == get_lp_id()));
        assert_eq!(thread.migration_constraints, 0);
    }

    // Ready -> Blocked removes its inline queue link allocation-free. A late
    // callback for the previous wait must not wake this later park, even with
    // the same thread generation. Its own callback still wakes normally.
    let second = scheduler.prepare_wait(tid, &source).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(scheduler.publish_wait(&second, constraint).unwrap());
    }
    first.waker.clone().notify();
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert!(matches!(&thread.state, ThreadState::Blocked(w) if Arc::ptr_eq(w, &second.waker)));
        assert!(!thread.run_queue_link.is_linked());
    }
    assert_eq!(source.source.registered(), 1);
    second.waker.clone().notify();
    assert_eq!(sponsor.used(), 0);

    // Extract a detached registration under the actual LP/table guards while
    // holding the heap. Its complete token survives until post-guard disposal.
    let third = scheduler.prepare_wait(tid, &source).unwrap();
    assert!(scheduler.publish_wait(&third, constraint).unwrap());
    let mut detached = None;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let mut lp = scheduler.get_lp_scheduler().lock();
        lp.add_thread(tid, Some(generation), Some(&third.waker), &mut detached).unwrap();
        assert!(detached.is_some());
        assert_eq!(source.source.registered(), 1);
    }
    detached.take().unwrap().cancel_registration();
    assert_eq!(sponsor.used(), 0);

    // Captured state changes reject before mutation; no new attempt or stale
    // queue owner is installed. This is a never-dispatched constructor state.
    let changed = scheduler.prepare_wait(tid, &source).unwrap();
    let mut original = take_fixture(&scheduler, tid);
    original.state = ThreadState::NeedsLpAssignment;
    let reused = MASTER_THREAD_TABLE.write().try_add_element(original).unwrap();
    assert_eq!(reused, tid);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(scheduler.publish_wait(&changed, constraint), Err(Error::InvalidThread)));
    }
    changed.waker.cancel_registration();

    // Reuse the exact slot for a new lifetime between capture and publication.
    let stale = scheduler.prepare_wait(tid, &source).unwrap();
    let replacement = fixture(&sponsor);
    let old = {
        let mut table = MASTER_THREAD_TABLE.write();
        let old = table.take_element(tid).unwrap();
        assert_eq!(table.try_add_element(replacement).unwrap(), tid);
        old
    };
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(scheduler.publish_wait(&stale, constraint), Err(Error::InvalidThread)));
    }
    stale.waker.clone().notify();
    assert!(matches!(
        MASTER_THREAD_TABLE.read().get(tid).unwrap().state,
        ThreadState::NeedsLpAssignment
    ));
    stale.waker.cancel_registration();
    assert_eq!(sponsor.used(), 0);
    release(old);
    release(take_fixture(&scheduler, tid));
    drop((early, first, second, third, changed, stale));
    drop(scheduler);
    drop(_setup);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[scheduler wait publication] guard-free registration, early/late wake, exact wait/slot \
         identity, heap-held publication/detachment and post-guard cancellation passed"
    );
}
