//! Scheduler timer, migration, and cross-LP retirement regression coverage.

use alloc::{
    sync::Weak,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    AtomicUsize,
    Ordering,
};

use spin::{
    LazyLock,
    Mutex,
};

use crate::{
    cpu::{
        isa::lp::ops::get_lp_id,
        multiprocessor::get_lp_count,
        scheduler::{
            sleep_millis,
            spawn_migratable_thread_on_lp,
            spawn_thread,
            spawn_thread_on_lp,
            system_scheduler::{
                REBALANCE_SUCCESSES,
                SYSTEM_SCHEDULER,
                get_thread_id,
            },
            threads::{
                DEAD_THREADS,
                MASTER_THREAD_TABLE,
                waker::WAKER_DIAGNOSTICS,
            },
        },
    },
    klib::observer::{
        Observable,
        Observer,
    },
    logln,
    mask_interrupts,
    memory::KERNEL_ASID,
};

#[unsafe(no_mangle)]
pub static SCHEDULER_LIFECYCLE_PROGRESS: AtomicU64 = AtomicU64::new(0);
static SCHEDULER_LIFECYCLE_WORKERS_DONE: AtomicU64 = AtomicU64::new(0);
static RUNTIME_REBALANCE_TARGET: AtomicU64 = AtomicU64::new(u64::MAX);
static RUNTIME_REBALANCE_WORKERS_DONE: AtomicU64 = AtomicU64::new(0);
static RUNTIME_REBALANCE_DONE: AtomicBool = AtomicBool::new(false);
static REMOTE_ABORT_DONE: AtomicBool = AtomicBool::new(false);
static SCHEDULER_LIFECYCLE_REPORTED: AtomicBool = AtomicBool::new(false);
static REMOTE_ABORT_TARGET_TID: AtomicUsize = AtomicUsize::new(usize::MAX);
static REMOTE_ABORT_TARGET_GENERATION: AtomicU64 = AtomicU64::new(0);
static REMOTE_TARGET_RUNNING: AtomicBool = AtomicBool::new(false);
static REMOTE_TARGET_RELEASE: AtomicBool = AtomicBool::new(false);
static REMOTE_TARGET_BLOCKED: AtomicBool = AtomicBool::new(false);
static REMOTE_WAKE_SENT: AtomicBool = AtomicBool::new(false);
static REMOTE_TARGET_RESUMED: AtomicBool = AtomicBool::new(false);

const WORKER_COUNT: u64 = 3;

#[derive(Default)]
struct AbortRaceEvent {
    observers: Mutex<Vec<Weak<dyn Observer>>>,
}

impl Observable for AbortRaceEvent {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        self.observers.lock().push(observer);
    }
}

impl AbortRaceEvent {
    fn signal(&self) {
        let observers = core::mem::take(&mut *self.observers.lock());
        for observer in observers {
            if let Some(observer) = observer.upgrade() {
                observer.notify();
            }
        }
    }
}

static REMOTE_ABORT_EVENT: LazyLock<AbortRaceEvent> = LazyLock::new(AbortRaceEvent::default);

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
             controlled runtime imbalance before the cross-LP abort regression.",
            WORKER_COUNT
        );
        spawn_thread(KERNEL_ASID, runtime_rebalance_coordinator);
    }
}

extern "C" fn runtime_rebalance_coordinator() {
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
            "[scheduler runtime rebalance] sustained-window sampling advanced certified \
             migrations to {}; starting cross-LP abort regression.",
            REBALANCE_SUCCESSES.load(Ordering::Relaxed)
        );
        RUNTIME_REBALANCE_DONE.store(true, Ordering::Release);
        start_remote_abort_regression();
    }
}

fn start_remote_abort_regression() {
    if get_lp_count() < 2 {
        logln!("[scheduler remote abort] skipped: requires at least two LPs");
        REMOTE_ABORT_DONE.store(true, Ordering::Release);
        maybe_report_success();
        return;
    }
    let target = spawn_thread_on_lp(KERNEL_ASID, remote_abort_target, 1);
    let generation = MASTER_THREAD_TABLE
        .read()
        .get(target)
        .expect("remote-abort target missing after spawn")
        .generation;
    REMOTE_ABORT_TARGET_GENERATION.store(generation, Ordering::Release);
    REMOTE_ABORT_TARGET_TID.store(target, Ordering::Release);
    spawn_thread_on_lp(KERNEL_ASID, remote_abort_coordinator, 0);
}

extern "C" fn remote_abort_target() {
    // Hold LP1 in a known physically-running state so LP0 cannot accidentally
    // exercise the non-running abort path. The pending scheduler IPI is handled
    // only after this thread installs the deliberate block/wake race.
    mask_interrupts!();
    REMOTE_TARGET_RUNNING.store(true, Ordering::Release);
    while !REMOTE_TARGET_RELEASE.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }

    let tid = get_thread_id().expect("remote-abort target has no TID");
    SYSTEM_SCHEDULER
        .read()
        .block_thread(tid, &*REMOTE_ABORT_EVENT)
        .expect("remote-abort target failed to install blocking waker");
    REMOTE_TARGET_BLOCKED.store(true, Ordering::Release);
    while !REMOTE_WAKE_SENT.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    crate::cpu::scheduler::yield_lp();

    REMOTE_TARGET_RESUMED.store(true, Ordering::Release);
    panic!("remotely aborted target resumed after its owner-LP switch");
}

extern "C" fn remote_abort_coordinator() {
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    while !REMOTE_TARGET_RUNNING.load(Ordering::Acquire) {
        deadline.assert_pending("remote-abort target to run on LP1");
        crate::cpu::scheduler::yield_lp();
    }
    let target = REMOTE_ABORT_TARGET_TID.load(Ordering::Acquire);
    assert_ne!(target, usize::MAX);
    SYSTEM_SCHEDULER.read().abort_thread(target).expect("remote cross-LP abort request failed");
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(target).expect("remote target retired before interrupts released");
        assert!(thread.abort_requested.load(Ordering::Acquire));
        assert_eq!(thread.abort_owner_lp.load(Ordering::Acquire), 1);
    }

    REMOTE_TARGET_RELEASE.store(true, Ordering::Release);
    while !REMOTE_TARGET_BLOCKED.load(Ordering::Acquire) {
        deadline.assert_pending("remote-abort target to install its racing block");
        crate::cpu::scheduler::yield_lp();
    }
    let rejected_before = WAKER_DIAGNOSTICS[2].load(Ordering::Acquire);
    REMOTE_ABORT_EVENT.signal();
    assert_eq!(
        WAKER_DIAGNOSTICS[2].load(Ordering::Acquire),
        rejected_before + 1,
        "abort-requested generation was not rejected by wake admission"
    );
    REMOTE_WAKE_SENT.store(true, Ordering::Release);

    let target_generation = REMOTE_ABORT_TARGET_GENERATION.load(Ordering::Acquire);
    loop {
        let in_master = MASTER_THREAD_TABLE.read().get(target).is_ok();
        let in_dead = DEAD_THREADS
            .read()
            .values()
            .flatten()
            .any(|thread| thread.generation == target_generation);
        if !in_master && !in_dead {
            break;
        }
        deadline.assert_pending("owner-LP retirement and deferred reaping");
        crate::cpu::scheduler::yield_lp();
    }
    assert!(!REMOTE_TARGET_RESUMED.load(Ordering::Acquire));
    REMOTE_ABORT_DONE.store(true, Ordering::Release);
    logln!(
        "[scheduler remote abort] target stayed on LP1 through request, racing wake was rejected, \
         and owner-side retirement completed off-CPU."
    );
    maybe_report_success();
}

fn maybe_report_success() {
    if RUNTIME_REBALANCE_DONE.load(Ordering::Acquire)
        && REMOTE_ABORT_DONE.load(Ordering::Acquire)
        && SCHEDULER_LIFECYCLE_REPORTED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        crate::self_test::results::pass(crate::self_test::results::TestId::SchedulerLifecycle);
        logln!(
            "[scheduler lifecycle] SUCCESS: timer affinity, certified migration, and safe \
             cross-LP retirement verified."
        );
    }
}
