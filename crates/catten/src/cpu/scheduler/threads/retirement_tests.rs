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
    klib::observer::CallOnNotify,
    memory::PHYSICAL_FRAME_ALLOCATOR,
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
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(NEXT_THREAD_GENERATION.load(Ordering::Relaxed), generation);

    // Warm physical kernel-stack table backing, which is cached independently
    // of thread/node ownership. These contexts are never scheduler-admitted.
    let warm: Vec<_> = (0..3).map(|_| Thread::new(KERNEL_ASID, unused_entry)).collect();
    drop(warm);
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
    assert!(!has_staged_generation(other_generation));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert!(!retirement_in_flight());
    for cap in [cap_a, cap_b, cap_c] {
        completion::close(asid, cap).unwrap();
    }
    drop((observer_a, observer_b, observer_c, token_a, token_b, token_c));
    completion::close_address_space(asid);
    crate::capability::close_address_space(asid);
    crate::logln!(
        "[thread retirement] prepared-node rejection, same-LP/current-stack retention, unlocked \
         callbacks and physical stack recovery passed"
    );
}
