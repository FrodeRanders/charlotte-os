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
            spawn_thread,
            spawn_migratable_thread_on_lp,
            system_scheduler::{
                get_thread_id,
                REBALANCE_SUCCESSES,
            },
            threads::MASTER_THREAD_TABLE,
        },
        multiprocessor::get_lp_count,
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
    MASTER_THREAD_TABLE
        .write()
        .get_mut(tid)
        .expect("lifecycle worker vanished")
        .migration_safe = false;
    let home = get_lp_id();
    for _ in 0..128 {
        sleep_millis(1);
        assert_eq!(get_lp_id(), home);
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).expect("lifecycle worker vanished");
        assert_eq!(thread.migration_constraints, 0);
        assert!(!thread.is_fully_migratable());
        drop(table);
        SCHEDULER_LIFECYCLE_PROGRESS.fetch_add(1, Ordering::Relaxed);
    }
    if SCHEDULER_LIFECYCLE_WORKERS_DONE.fetch_add(1, Ordering::AcqRel) + 1 == WORKER_COUNT {
        let migrations = REBALANCE_SUCCESSES.load(Ordering::Relaxed);
        if get_lp_count() > 1 {
            assert!(migrations > 0);
        }
        logln!(
            "[scheduler lifecycle] SUCCESS: {} timer wakes retained post-rebalance LP affinity across {} workers; {} certified Ready migration(s) completed.",
            128 * WORKER_COUNT,
            WORKER_COUNT,
            migrations
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
        logln!(
            "[scheduler runtime rebalance] SUCCESS: sustained-window sampling advanced certified migrations to {}.",
            REBALANCE_SUCCESSES.load(Ordering::Relaxed)
        );
    }
}
