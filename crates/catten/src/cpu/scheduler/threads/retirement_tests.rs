//! Prepared thread retirement and actual stack/callback ownership fixtures.

use alloc::sync::Arc;
use core::{
    alloc::AllocError,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use super::*;
use crate::{
    completion,
    cpu::scheduler::deferred::drain as drain_reaper_wakes,
    klib::observer::CallOnNotify,
    memory::{
        AddressSpaceInterface,
        PHYSICAL_FRAME_ALLOCATOR,
    },
};

extern "C" fn unused_entry() {}

pub(crate) fn run() {
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let generation = NEXT_THREAD_GENERATION.load(Ordering::Relaxed);
    for _ in 0..64 {
        assert!(matches!(
            Thread::try_new_with_retirement(KERNEL_ASID, unused_entry, || Err(AllocError)),
            Err(crate::cpu::scheduler::system_scheduler::Error::ThreadPreparationFailed)
        ));
    }
    for _ in 0..64 {
        assert!(matches!(
            Thread::try_new_with_storage(
                KERNEL_ASID,
                unused_entry,
                PreparedEntry::try_new,
                || Err(AllocError)
            ),
            Err(crate::cpu::scheduler::system_scheduler::Error::ThreadPreparationFailed)
        ));
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(NEXT_THREAD_GENERATION.load(Ordering::Relaxed), generation);

    // Warm physical kernel-stack table backing, which is cached independently
    // of thread/node ownership. These contexts are never scheduler-admitted.
    let warm: Vec<_> = (0..3).map(|_| Thread::new(KERNEL_ASID, unused_entry)).collect();
    for thread in warm {
        thread.release_unstarted().unwrap();
    }
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let asid = 0xc0ae_b007;
    completion::open_address_space(asid, 3);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut owners = Vec::new();
    for _ in 0..3 {
        let thread = Thread::new(KERNEL_ASID, unused_entry);
        let cap = completion::submit(asid, completion::OpCode::Nop, None).unwrap();
        let calls = calls.clone();
        let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
            assert!(DEAD_THREADS.try_write().is_some());
            assert!(MASTER_THREAD_TABLE.try_write().is_some());
            assert!(retirement_in_flight());
            completion::complete(asid, cap, completion::OpResult::Ok(0)).unwrap();
            calls.fetch_add(1, Ordering::Relaxed);
        });
        let token = thread
            .try_observe_exit(
                Arc::downgrade(&observer),
                completion::watch_budget::reserve(
                    &completion::watch_admission(asid).unwrap(),
                    true,
                )
                .unwrap(),
            )
            .unwrap();
        owners.push((thread, cap, observer, token));
    }
    let [
        (first, cap_a, observer_a, token_a),
        (second, cap_b, observer_b, token_b),
        (third, cap_c, observer_c, token_c),
    ] = owners.try_into().unwrap_or_else(|_| panic!("fixture owner count"));
    let retained_sp = first.context.kernel_stack_bounds().0;
    let retained_generation = first.generation;
    METADATA_PROBE_GENERATION.store(retained_generation, Ordering::Release);
    let disposal_before = METADATA_PROBE_HITS.load(Ordering::Relaxed);
    let other_generation = third.generation;
    let lp = crate::cpu::isa::lp::ops::get_lp_id();
    // An independent inline head also exists on single-LP boots. No context
    // here has run; inspecting that head never claims a remote active stack.
    let other_lp = (lp + 1) % crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS as LpId;
    let epoch = retirement_epoch();
    {
        let _transition = begin_retirement();
        stage_dead_thread(lp, usize::MAX, first);
        stage_dead_thread(lp, usize::MAX - 1, second);
        stage_dead_thread(other_lp, usize::MAX - 2, third);
    }
    assert!(retirement_epoch() > epoch);
    assert!(!crate::cpu::isa::lp::ops::get_int_state());
    reap_dead_threads();
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(has_staged_generation(retained_generation));
    reap_dead_threads_with(lp, retained_sp);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(has_staged_generation(retained_generation));
    assert!(has_staged_generation(other_generation));
    assert!(completion::poll(asid, cap_a).unwrap().is_none());
    assert!(completion::poll(asid, cap_c).unwrap().is_none());
    #[cfg(target_arch = "aarch64")]
    {
        let set_ownership = |value| {
            DEAD_THREADS.read()[lp as usize]
                .iter()
                .find(|thread| thread.generation == retained_generation)
                .unwrap()
                .context
                .on_cpu
                .store(value, Ordering::Release);
        };
        // A synthetic assembly ownership flag must defer even when the test
        // supplies an SP outside this never-scheduled context's stack.
        set_ownership(1);
        reap_dead_threads_with(lp, current_stack_pointer());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(has_staged_generation(retained_generation));
        set_ownership(0);
    }
    reap_dead_threads_with(lp, current_stack_pointer());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(!has_staged_generation(retained_generation));
    assert!(has_staged_generation(other_generation));
    // Test-only inspection: these contexts never ran. Only the selected head
    // changes; this does not authorize production cross-LP physical reaping.
    reap_dead_threads_with(other_lp, current_stack_pointer());
    assert_eq!(calls.load(Ordering::Relaxed), 3);
    assert_eq!(METADATA_PROBE_HITS.load(Ordering::Relaxed), disposal_before + 1);
    METADATA_PROBE_GENERATION.store(0, Ordering::Release);
    assert!(!has_staged_generation(other_generation));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert!(!retirement_in_flight());
    for cap in [cap_a, cap_b, cap_c] {
        completion::close(asid, cap).unwrap();
    }
    drop((observer_a, observer_b, observer_c, token_a, token_b, token_c));
    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);
    readiness(lp, other_lp);
    inline_queue_retirement_fence();
    terminal_pair_retention(lp);
    assert!(!reaper_work_ready(lp, current_stack_pointer()));
    abandoned_payloads();
    crate::logln!(
        "[thread retirement] node/context-storage rejection, masked entry, same-LP/current-stack \
         retention, unlocked callbacks and physical stack recovery passed"
    );
}

fn inline_queue_retirement_fence() {
    use crate::{
        cpu::scheduler::lp_schedulers::run_queue::{
            RunQueue,
            ThreadHandle,
        },
        memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
    };
    // This table and queue never publish to a live scheduler. The complete
    // thread owns its stack and inline link across both rejected releases.
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut table = ThreadTable::new();
    let thread = Thread::new(KERNEL_ASID, unused_entry);
    let generation = thread.generation;
    let tid = table.add_element(thread);
    let key = ThreadHandle {
        tid,
        generation,
    };
    let mut queue = RunQueue::new();
    queue.push_back(&mut table, key).unwrap();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _master = MASTER_THREAD_TABLE.write();
        let _retirement = DEAD_THREADS.write();
        let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        assert!(!can_reap(table.get(tid).unwrap(), current_stack_pointer()));
        let mut thread = table.take_element(tid).unwrap();
        assert!(matches!(
            thread.retire_stacks(),
            Err(crate::memory::thread_stack::RetirementError::RunQueueLinked)
        ));
        assert!(!thread.retirement_metadata_completed);
        let thread = thread.release_unstarted().expect_err("linked owner must be returned");
        assert_eq!(thread.generation, generation);
        assert!(thread.run_queue_link.is_linked());
        assert_eq!(table.add_element(thread), tid);
        assert_eq!(allocator.free_frames(), free);
        queue.remove(&mut table, key).unwrap();
    }
    table.take_element(tid).unwrap().release_unstarted().unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    crate::logln!(
        "[inline run queue retirement] linked stack/unstarted release rejected under heap, \
         master/retirement/physical guards; complete exact owner returned, explicit unlink and \
         final frame recovery passed"
    );
}

fn readiness(lp: LpId, other_lp: LpId) {
    use crate::klib::observer::{
        WaitSponsor,
        registration::RegistrationError,
    };
    drain_reaper_wakes();
    assert!(!reaper_work_ready(lp, current_stack_pointer()));
    let sponsor = WaitSponsor::new(true);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        assert!(MASTER_THREAD_TABLE.try_write().is_some());
        assert!(DEAD_THREADS.try_write().is_some());
        assert!(crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.try_write().is_some());
        count.fetch_add(1, Ordering::Relaxed);
    });
    let source = &REAPER_WORK[lp as usize].waiters;
    let other = &REAPER_WORK[other_lp as usize].waiters;
    let token = source.register(Arc::downgrade(&observer), &sponsor).unwrap();
    let unrelated = other.register(Arc::downgrade(&observer), &sponsor).unwrap();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let first = Thread::new(KERNEL_ASID, unused_entry);
    let second = Thread::new(KERNEL_ASID, unused_entry);
    {
        let _system = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.write();
        let _table = MASTER_THREAD_TABLE.write();
        stage_dead_thread(lp, usize::MAX - 4, first);
        stage_dead_thread(lp, usize::MAX - 5, second);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }
    assert!(reaper_work_ready(lp, current_stack_pointer()));
    drain_reaper_wakes();
    assert_eq!(calls.load(Ordering::Relaxed), 1, "staging coalesces without a wrong-LP wake");
    assert_eq!(source.registered(), 0);
    assert_eq!(other.registered(), 1);
    drop((token, unrelated));
    assert_eq!(sponsor.used(), 0);
    // Publication and its wake can precede registration: the durable exact
    // head remains ready for the scheduler's post-registration recheck.
    let late = source.register(Arc::downgrade(&observer), &sponsor).unwrap();
    assert!(reaper_work_ready(lp, current_stack_pointer()));
    drop(late);
    assert_eq!(source.registered(), 0);
    reap_dead_threads_with(lp, current_stack_pointer());
    assert!(!reaper_work_ready(lp, current_stack_pointer()));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);

    // Full source rejection leaves the never-admitted fixture runnable and
    // refunds its attempted registration, with no timer/watchdog or cleanup.
    let mut full = Vec::new();
    for _ in 0..crate::klib::observer::waiter_budget::SOURCE_LIMIT {
        full.push(source.register(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert_eq!(
        source.register(Arc::downgrade(&observer), &sponsor).unwrap_err(),
        RegistrationError::ResourceLimit
    );
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let thread = Thread::new(KERNEL_ASID, unused_entry);
    let used = thread.wait_sponsor.used();
    let tid = crate::cpu::scheduler::system_scheduler::publish_thread(thread).unwrap();
    assert!(matches!(
        crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .block_thread_with_constraint_generation(tid, source, MigrationConstraint::GeneralWait),
        Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed)
    ));
    let thread = MASTER_THREAD_TABLE.write().take_element(tid).unwrap();
    assert!(matches!(thread.state, ThreadState::NeedsLpAssignment));
    assert_eq!(thread.wait_sponsor.used(), used);
    thread.release_unstarted().unwrap();
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    drop(full);
    assert_eq!(source.registered(), 0);
    assert_eq!(sponsor.used(), 0);
    crate::logln!(
        "[reaper readiness] guarded staging, coalesced exact-LP wake, late registration recheck, \
         full-source rejection and original frame recovery passed"
    );
}

fn terminal_pair_retention(lp: LpId) {
    let thread = Thread::new(KERNEL_ASID, unused_entry);
    let generation = thread.generation;
    let base = thread.context.kernel_stack_bounds().0;
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let context_identity = &*thread.context as *const ThreadContext;
    stage_dead_thread(lp, usize::MAX - 3, thread);
    let mut attempts = 0;
    // Inject one per-call physical rejection while the reaper still owns the
    // complete node; its ordinary error branch must reinsert that same owner.
    reap_dead_threads_releasing_with(lp, current_stack_pointer(), |thread| {
        attempts += 1;
        assert!(DEAD_THREADS.try_write().is_some());
        assert!(MASTER_THREAD_TABLE.try_write().is_some());
        thread.finish_retirement_metadata();
        thread.context.reject_stack_release_for_test()
    });
    assert_eq!(attempts, 1);
    let snapshot = staged_retirement_snapshot(generation).unwrap();
    assert_eq!(snapshot.root, None);
    assert_eq!(snapshot.lp, Some(lp));
    assert!(snapshot.started);
    assert!(matches!(
        snapshot.error,
        Some(crate::memory::thread_stack::RetirementError::Kernel(_))
    ));
    for _ in 0..2 {
        reap_dead_threads_with(lp, current_stack_pointer());
        assert!(has_staged_generation(generation));
        {
            let guard = DEAD_THREADS.read();
            let retained =
                guard[lp as usize].iter().find(|thread| thread.generation == generation).unwrap();
            assert_eq!(&*retained.context as *const ThreadContext, context_identity);
            assert!(retained.retirement_metadata_completed);
            assert!(retained.context.stack_retirement_started());
        }
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
        assert!(
            crate::memory::KERNEL_AS.lock().is_mapped(crate::memory::VAddr::from(base)).unwrap()
        );
    }
    // The existing registry node keeps the owner, and no fresh custody entry
    // or scalar retry exists. This retained kernel pair costs sixteen pages.
    assert!(!retirement_in_flight());
    crate::logln!(
        "[published stack retirement] failed pair/node retained through two scans; no physical \
         retry"
    );
}

static METADATA_PROBE_GENERATION: AtomicU64 = AtomicU64::new(0);
static METADATA_PROBE_HITS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn before_metadata_disposal(thread: &Thread) {
    // Only the serialized boot fixture selects a lifetime. Concurrent holds
    // by other LPs must never make normal production disposal fail this probe.
    if METADATA_PROBE_GENERATION.load(Ordering::Acquire) != thread.generation {
        return;
    }
    assert!(thread.retirement_metadata_completed && thread.stacks_retired);
    assert!(DEAD_THREADS.try_write().is_some());
    assert!(MASTER_THREAD_TABLE.try_write().is_some());
    assert!(crate::memory::ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
    METADATA_PROBE_HITS.fetch_add(1, Ordering::Relaxed);
}

fn abandoned_payloads() {
    use crate::klib::observer::{
        WaitSponsor,
        waiter_source::WaiterSource,
    };
    let asid = 0xc0ae_d007;
    completion::open_address_space(asid, 4);
    let watches = completion::watch_admission(asid).unwrap();
    let root = crate::service::loader::create_user_address_space_handle();
    let abandoned_before = ABANDONED_THREAD_PAYLOADS.load(Ordering::Relaxed);
    for owner in [KERNEL_ASID, root.id()] {
        let mut thread = Thread::new(owner, unused_entry);
        let source = WaiterSource::new();
        let sponsor = WaitSponsor::new(false);
        let waker = Arc::new(Waker::new(usize::MAX, thread.generation));
        let registration =
            source.register(Arc::downgrade(&waker) as Weak<dyn Observer>, &sponsor).unwrap();
        waker.set_registration(registration);
        thread.state = ThreadState::Blocked(waker);
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
            panic!("abandoned thread fabricated an exit callback");
        });
        let token = thread
            .try_observe_exit(
                Arc::downgrade(&observer),
                completion::watch_budget::reserve(&watches, true).unwrap(),
            )
            .unwrap();
        let usage_before = crate::memory::usage::domain_usage(owner);
        {
            // Hold the exact local boundaries that fallback must not enter.
            // This thread was never admitted/executing and owns all its fields.
            let _table = MASTER_THREAD_TABLE.write();
            let _dead = DEAD_THREADS.write();
            let _spaces = crate::memory::ADDRESS_SPACE_TABLE.lock();
            let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
            let free_before = allocator.free_frames();
            drop(thread);
            assert_eq!(allocator.free_frames(), free_before);
        }
        assert_eq!(hits.load(Ordering::Relaxed), 0);
        assert_eq!(source.registered(), 1);
        assert_eq!(sponsor.used(), 1);
        assert_eq!(watches.used(), 1);
        assert_eq!(crate::memory::usage::domain_usage(owner), usage_before);
        // External watch/source cancellation is separate from the abandoned
        // payload. It cannot recover/adopt its context, stack or root lease.
        drop(token);
        assert_eq!(watches.used(), 0);
        drop(source);
        assert_eq!(sponsor.used(), 0);
    }
    assert_eq!(ABANDONED_THREAD_PAYLOADS.load(Ordering::Relaxed), abandoned_before + 2);
    assert_eq!(crate::memory::usage::domain_usage(root.id()).unwrap().threads, 1);
    assert_eq!(
        crate::memory::close_user_address_space_handle(root),
        Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
    );
    completion::close_address_space(asid);
    crate::logln!(
        "[thread metadata retention] kernel/user payload Drop under \
         table/retirement/root/allocator guards: no exit, waiter cancellation, usage refund or \
         frame return; exact user root remains busy"
    );
}
