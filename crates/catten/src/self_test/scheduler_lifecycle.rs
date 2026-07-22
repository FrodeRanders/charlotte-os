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
    let home = get_lp_id();
    let tid = get_thread_id().expect("lifecycle worker has no scheduler thread id");
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
    }
}
