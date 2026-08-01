//! Scheduler timer lifecycle regression coverage.

use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use crate::{
    cpu::{
        isa::lp::ops::get_lp_id,
        scheduler::{
            sleep_millis,
            spawn_migratable_thread_on_lp,
            spawn_thread,
            system_scheduler::{
                REBALANCE_SUCCESSES,
                SYSTEM_SCHEDULER,
                get_thread_id,
            },
            threads::MASTER_THREAD_TABLE,
        },
    },
    logln,
    memory::KERNEL_ASID,
};

#[unsafe(no_mangle)]
pub static SCHEDULER_LIFECYCLE_PROGRESS: AtomicU64 = AtomicU64::new(0);
static SCHEDULER_LIFECYCLE_WORKERS_DONE: AtomicU64 = AtomicU64::new(0);
static RUNTIME_REBALANCE_TARGET: AtomicU64 = AtomicU64::new(u64::MAX);
static RUNTIME_REBALANCE_WORKERS_DONE: AtomicU64 = AtomicU64::new(0);

const WORKER_COUNT: u64 = 3;

pub fn test_scheduler_lifecycle() {
    for _ in 0..WORKER_COUNT {
        spawn_migratable_thread_on_lp(KERNEL_ASID, worker, 0);
    }
    logln!(
        "[scheduler lifecycle] {} initially co-located timer-affinity workers deferred",
        WORKER_COUNT
    );
}

extern "C" fn worker() {
    let tid = get_thread_id().expect("lifecycle worker has no scheduler thread id");
    // These workers are migratable only while queued at boot. Once they begin
    // their timer-affinity regression, freeze their established home; the
    // delayed compute-only workload below separately covers runtime migration.
    MASTER_THREAD_TABLE.write().get_mut(tid).expect("lifecycle worker vanished").migration_safe =
        false;
    let home = get_lp_id();
    logln!("[scheduler lifecycle] worker tid={} running on LP{}", tid, home);
    let generation =
        MASTER_THREAD_TABLE.read().get(tid).expect("lifecycle worker vanished").generation;
    assert!(
        SYSTEM_SCHEDULER.read().submit_new_thread(tid).is_err(),
        "generation-free admission accepted a non-new thread"
    );
    assert!(
        SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation.wrapping_add(1)).is_err(),
        "stale generation re-admitted a live thread"
    );
    // One substantial wait is enough to verify the property under test:
    // rebalance established this worker's home LP before it acquired a timer
    // affinity, and the wake must return it to that LP. Hundreds of 1 ms waits
    // merely made this boot gate depend on emulator scheduling granularity.
    sleep_millis(128);
    assert_eq!(get_lp_id(), home);
    let table = MASTER_THREAD_TABLE.read();
    let thread = table.get(tid).expect("lifecycle worker vanished");
    assert_eq!(thread.migration_constraints, 0);
    assert!(!thread.is_fully_migratable());
    drop(table);
    SCHEDULER_LIFECYCLE_PROGRESS.fetch_add(1, Ordering::Relaxed);
    logln!("[scheduler lifecycle] worker tid={} completed on LP{}", tid, home);
    if SCHEDULER_LIFECYCLE_WORKERS_DONE.fetch_add(1, Ordering::AcqRel) + 1 == WORKER_COUNT {
        logln!(
            "[scheduler lifecycle] {} timer wakes retained established LP affinity; inducing a \
             controlled runtime imbalance before recording success.",
            WORKER_COUNT
        );
        spawn_thread(KERNEL_ASID, runtime_rebalance_coordinator);
    }
}

extern "C" fn runtime_rebalance_coordinator() {
    // Keep the deliberate runnable imbalance out of the early lifecycle gates.
    sleep_millis(3_000);
    let target = REBALANCE_SUCCESSES.load(Ordering::Acquire) + 1;
    RUNTIME_REBALANCE_TARGET.store(target, Ordering::Release);
    for _ in 0..WORKER_COUNT {
        spawn_migratable_thread_on_lp(KERNEL_ASID, runtime_rebalance_worker, 0);
    }
}

extern "C" fn runtime_rebalance_worker() {
    let target = RUNTIME_REBALANCE_TARGET.load(Ordering::Acquire);
    while REBALANCE_SUCCESSES.load(Ordering::Acquire) < target {
        crate::cpu::scheduler::yield_lp();
    }
    if RUNTIME_REBALANCE_WORKERS_DONE.fetch_add(1, Ordering::AcqRel) + 1 == WORKER_COUNT {
        crate::self_test::results::pass(crate::self_test::results::TestId::SchedulerLifecycle);
        logln!(
            "[scheduler runtime rebalance] SUCCESS: sustained-window sampling advanced certified \
             migrations to {}.",
            REBALANCE_SUCCESSES.load(Ordering::Relaxed)
        );
    }
}
