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
            system_scheduler::REBALANCE_SUCCESSES,
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
    for _ in 0..128 {
        sleep_millis(1);
        assert_eq!(get_lp_id(), home);
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
