//! Exact-generation whole-domain sweep, fence and namespace reuse fixtures.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    cpu::scheduler::{
        system_scheduler::{
            domain_has_live_threads,
            publish_thread,
        },
        threads::{
            Thread,
            has_staged_generation,
            reap_dead_threads_with,
        },
    },
    klib::observer::{
        CallOnNotify,
        Observer,
    },
    memory::{
        self,
        AddressSpaceCloseError,
        DomainLimits,
        PHYSICAL_FRAME_ALLOCATOR,
    },
};

extern "C" fn unused_entry() {}

fn domain() -> AddressSpaceHandle {
    let handle =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    memory::set_domain_limits(
        handle,
        DomainLimits {
            user_stack_pages: 1,
            max_threads: 4,
        },
    )
    .unwrap();
    handle
}

fn discard(tid: ThreadId) {
    // All contexts here are never scheduler-admitted. The fixture owns their
    // exact numeric slots without any concurrent scheduler activity.
    let thread = MASTER_THREAD_TABLE.write().take_element(tid).unwrap();
    thread.release_unstarted().unwrap();
}

pub(crate) fn run() {
    // Warm shared kernel-stack branches independently from user-root private
    // tables, which must still refund at each exact root's final teardown.
    let warm = domain();
    let threads: Vec<_> = (0..4).map(|_| Thread::new(warm.id(), unused_entry)).collect();
    for thread in threads {
        thread.release_unstarted().unwrap();
    }
    memory::close_user_address_space_handle(warm).unwrap();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let owner = domain();
    let foreign = domain();
    let captured: Vec<_> = (0..3)
        .map(|_| {
            let thread = Thread::new(owner.id(), unused_entry);
            let generation = thread.generation;
            (publish_thread(thread).unwrap(), generation)
        })
        .collect();
    let prepared = Thread::new(owner.id(), unused_entry);
    let foreign_tid = publish_thread(Thread::new(foreign.id(), unused_entry)).unwrap();
    let callbacks = Arc::new(AtomicUsize::new(0));
    let notified = callbacks.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        assert!(THREAD_PUBLICATION_GATE.try_lock().is_some());
        assert!(MASTER_THREAD_TABLE.try_write().is_some());
        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
        assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
        notified.fetch_add(1, Ordering::Relaxed);
    });
    let account = crate::completion::watch_budget::DomainBudget::new(1);
    let subscription = prepared
        .try_observe_exit(
            Arc::downgrade(&observer),
            crate::completion::watch_budget::reserve(&account, true).unwrap(),
        )
        .unwrap();
    let sweep = DomainAbortSweep::begin(owner).unwrap();
    assert!(ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().thread_admission_closed);
    assert!(!domain_has_live_threads(owner));
    assert!(domain_has_live_threads(foreign));
    assert!(matches!(publish_thread(prepared), Err(Error::ThreadTerminated)));
    assert_eq!(callbacks.load(Ordering::Relaxed), 1);
    assert_eq!(account.used(), 0);
    drop((observer, subscription));
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        assert!(matches!(Thread::try_new(owner.id(), unused_entry), Err(Error::ThreadTerminated)));
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    assert_eq!(
        ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().thread_stack_slots.count_ones(),
        3
    );
    assert_eq!(
        memory::close_user_address_space_handle(owner),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    let mut replacement = None;
    sweep
        .run(|tid, generation| {
            assert!(THREAD_PUBLICATION_GATE.try_lock().is_some());
            assert!(MASTER_THREAD_TABLE.try_write().is_some());
            assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
            assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
            if replacement.is_none() {
                assert_eq!((tid, generation), captured[0]);
                discard(tid);
                let thread = Thread::new(foreign.id(), unused_entry);
                let later_generation = thread.generation;
                let later_tid = publish_thread(thread).unwrap();
                assert_eq!(later_tid, tid);
                assert_ne!(later_generation, generation);
                replacement = Some((later_tid, later_generation));
            }
        })
        .unwrap();
    let (later_tid, later_generation) = replacement.unwrap();
    assert_eq!(MASTER_THREAD_TABLE.read().get(later_tid).unwrap().generation, later_generation);
    assert!(MASTER_THREAD_TABLE.read().get(foreign_tid).is_ok());
    for &(tid, generation) in &captured[1..] {
        assert!(MASTER_THREAD_TABLE.read().get(tid).is_err());
        assert!(has_staged_generation(generation));
    }
    // Boot-only adapter: every context in this fixture was never admitted.
    reap_dead_threads_with(crate::cpu::isa::lp::ops::get_lp_id(), 0);
    for &(_, generation) in &captured[1..] {
        assert!(!has_staged_generation(generation));
    }
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get(owner.id()).unwrap().thread_stack_slots, 0);
    // Repeated sweeps use the same inline fence and release every temporary
    // root lease. They do not require per-abort registry storage.
    let mut request_published = false;
    abort_domain_threads_with_request(owner, |root| {
        assert!(THREAD_PUBLICATION_GATE.try_lock().is_some());
        assert!(MASTER_THREAD_TABLE.try_write().is_some());
        assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
        assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
        assert_eq!(
            memory::close_user_address_space_handle(owner),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
        assert_eq!(root.handle(), owner);
        request_published = true;
        Ok(())
    })
    .unwrap();
    assert!(request_published);
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        abort_domain_threads(owner).unwrap();
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    memory::close_user_address_space_handle(owner).unwrap();
    let successor = domain();
    assert_eq!(successor.id(), owner.id());
    assert_ne!(successor.generation(), owner.generation());
    assert!(!ADDRESS_SPACE_TABLE.lock().get(successor.id()).unwrap().thread_admission_closed);
    let successor_tid = publish_thread(Thread::new(successor.id(), unused_entry)).unwrap();
    for _ in 0..64 {
        assert!(matches!(
            abort_domain_threads_with_request(owner, |_| panic!(
                "stale root published a force request"
            )),
            Err(Error::ThreadTerminated)
        ));
    }
    assert!(domain_has_live_threads(successor));
    assert!(MASTER_THREAD_TABLE.read().get(successor_tid).is_ok());
    assert!(!ADDRESS_SPACE_TABLE.lock().get(successor.id()).unwrap().thread_admission_closed);
    // Dummy pages must never be read or written: the obsolete root must reject
    // force publication before those addresses are used by shutdown policy.
    crate::service::shutdown::tests::test_rejected_thread_abort(
        crate::service::supervisor::ServiceDomain {
            asid: owner.id(),
            address_space: owner,
            tid: captured[0].0,
            generation: captured[0].1,
            config_frame: memory::PAddr::from(0u64),
            status_frame: memory::PAddr::from(0u64),
        },
    );
    crate::service::shutdown::tests::test_rejected_service_pages(
        crate::service::supervisor::ServiceDomain {
            asid: owner.id(),
            address_space: owner,
            tid: captured[0].0,
            generation: captured[0].1,
            config_frame: 0u64.into(),
            status_frame: 0u64.into(),
        },
    );
    assert!(MASTER_THREAD_TABLE.read().get(successor_tid).is_ok());
    discard(successor_tid);
    discard(later_tid);
    discard(foreign_tid);
    let prepared_for_close = Thread::new(successor.id(), unused_entry);
    let closing = memory::retirement::ClosingAddressSpace::begin(successor).unwrap();
    assert!(matches!(publish_thread(prepared_for_close), Err(Error::ThreadTerminated)));
    assert!(matches!(Thread::try_new(successor.id(), unused_entry), Err(Error::ThreadTerminated)));
    for _ in 0..8 {
        assert!(matches!(
            abort_domain_threads_with_request(successor, |_| panic!(
                "closing root published a force request"
            )),
            Err(Error::ThreadTerminated)
        ));
        assert!(!ADDRESS_SPACE_TABLE.lock().get(successor.id()).unwrap().thread_admission_closed);
    }
    assert!(matches!(closing.poll().unwrap(), memory::retirement::CloseProgress::Complete));
    let kernel = memory::current_address_space_handle(memory::KERNEL_ASID).unwrap();
    assert!(matches!(abort_domain_threads(kernel), Err(Error::ThreadTerminated)));
    assert!(!ADDRESS_SPACE_TABLE.lock().get(kernel.id()).unwrap().thread_admission_closed);
    memory::close_user_address_space_handle(foreign).unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    crate::logln!(
        "[domain abort] inline fence, rejected prepared/late threads, unlocked generation claim, \
         TID/ASID reuse and root/frame recovery passed"
    );
}
