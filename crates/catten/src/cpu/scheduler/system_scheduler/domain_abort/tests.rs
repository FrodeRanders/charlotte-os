//! Exact-generation whole-domain sweep, fence and namespace reuse fixtures.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicU64,
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

// One serialized EL0 fixture; these scalars select observations, never own a
// root/thread or authorize cleanup. Arm before publishing its faulting caller.
static HANDOFF_ROOT: AtomicUsize = AtomicUsize::new(0);
static HANDOFF_GENERATION: AtomicUsize = AtomicUsize::new(0);
static HANDOFF_PEER: AtomicUsize = AtomicUsize::new(0);
static HANDOFF_PEER_GENERATION: AtomicU64 = AtomicU64::new(0);
static HANDOFF_PHASE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn arm_self_handoff(handle: AddressSpaceHandle, peer: (ThreadId, ThreadGeneration)) {
    assert_eq!(HANDOFF_ROOT.load(Ordering::Acquire), 0);
    HANDOFF_GENERATION.store(handle.generation(), Ordering::Relaxed);
    HANDOFF_PEER.store(peer.0, Ordering::Relaxed);
    HANDOFF_PEER_GENERATION.store(peer.1, Ordering::Relaxed);
    HANDOFF_PHASE.store(1, Ordering::Relaxed);
    HANDOFF_ROOT.store(handle.id() + 1, Ordering::Release);
}

fn observes(handle: AddressSpaceHandle) -> bool {
    HANDOFF_ROOT.load(Ordering::Acquire) == handle.id() + 1
        && HANDOFF_GENERATION.load(Ordering::Relaxed) == handle.generation()
}

pub(super) fn before_self_handoff(
    handle: AddressSpaceHandle,
    caller: Option<(ThreadId, ThreadGeneration)>,
) {
    if !observes(handle) {
        return;
    }
    let (tid, generation) = caller.expect("faulting sweep did not defer its caller");
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    assert!(THREAD_PUBLICATION_GATE.try_lock().is_some());
    assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert_eq!(thread.generation, generation);
        assert_eq!(thread.address_space, Some(handle));
        assert!(!thread.abort_requested.load(Ordering::Acquire));
        let peer = HANDOFF_PEER.load(Ordering::Relaxed);
        let peer_generation = HANDOFF_PEER_GENERATION.load(Ordering::Relaxed);
        if let Ok(thread) = table.get(peer)
            && thread.generation == peer_generation
        {
            assert_eq!(thread.address_space, Some(handle));
            assert!(thread.abort_requested.load(Ordering::Acquire));
        }
    }
    let kernel = memory::current_address_space_handle(memory::KERNEL_ASID).unwrap();
    {
        let mask = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        let scheduler = SYSTEM_SCHEDULER.read();
        assert!(matches!(
            scheduler.request_executing_abort(tid, generation.wrapping_add(1), handle, &mask),
            Err(Error::InvalidThread)
        ));
        assert!(matches!(
            scheduler.request_executing_abort(tid, generation, kernel, &mask),
            Err(Error::InvalidThread)
        ));
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    // No guard/local mask survives into the forced scheduler boundary. The
    // fault entry's IRQ state is preserved, never force-enabled by this probe.
    for _ in 0..8 {
        crate::cpu::scheduler::yield_lp();
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert_eq!(thread.generation, generation);
        assert!(!thread.abort_requested.load(Ordering::Acquire));
    }
    assert_eq!(HANDOFF_PHASE.swap(2, Ordering::AcqRel), 1);
}

pub(super) fn after_self_handoff(
    handle: AddressSpaceHandle,
    caller: Option<(ThreadId, ThreadGeneration)>,
    success: bool,
) {
    if !observes(handle) {
        return;
    }
    assert!(success);
    assert!(!crate::cpu::isa::lp::ops::get_int_state());
    let (tid, generation) = caller.unwrap();
    let table = MASTER_THREAD_TABLE.read();
    let thread = table.get(tid).unwrap();
    assert_eq!(thread.generation, generation);
    assert!(thread.abort_requested.load(Ordering::Acquire));
    #[cfg(target_arch = "aarch64")]
    assert!(thread.context.is_on_cpu());
    assert_eq!(
        thread.abort_owner_lp.load(Ordering::Acquire),
        crate::cpu::isa::lp::ops::get_lp_id() as usize
    );
    assert_eq!(HANDOFF_PHASE.swap(3, Ordering::AcqRel), 2);
}

pub(crate) fn finish_self_handoff(handle: AddressSpaceHandle) {
    assert!(observes(handle));
    assert_eq!(HANDOFF_PHASE.load(Ordering::Acquire), 3);
    HANDOFF_ROOT.store(0, Ordering::Release);
    crate::logln!(concat!(
        "[domain abort] real fault: stale identity rejection, peer request, ",
        "eight pre-handoff yields, masked self-request/root completion ",
        "and exact root teardown passed"
    ));
}

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
