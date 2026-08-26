//! Authoritative completion state for deferred kernel self-tests.
//!
//! The selftest suite's tests that need a running scheduler or a real EL0
//! domain run as *deferred verifiers*: each is a kernel thread registered
//! with one bit in a per-test bitmap. This module tracks which tests are
//! registered (`EXPECTED`), which have passed (`PASSED`) or failed
//! (`FAILED`), and — once [`finalize_and_start_coordinator`] is called after
//! boot — runs a coordinator thread that prints periodic `SELFTEST WAITING`
//! summaries and, when every registered test has resolved, a single
//! authoritative `SELFTEST COMPLETE` line with pass/fail/pending counts and
//! bitmaps.
//!
//! Why: boot-path tests cannot block on asynchronous results (the scheduler
//! is not running), so the verdict must come from a thread that yields. The
//! coordinator deliberately avoids timer-based sleeps so the authoritative
//! result is never itself dependent on the timer-wake path under test.
//!
//! Expected outcome: `register_boot_suite` registers the 17 boot tests (plus
//! any feature-gated network tests), every verifier reports exactly once via
//! [`pass`]/[`fail`] (asserting it was registered and has not already
//! resolved), and the coordinator terminates with `failed=0 pending=0` — on
//! virt/TCG and sbsa-ref this is `passed=17 failed=0 pending=0`. The HVF
//! compatibility suite omits the NVMe and persistent-Raft results because
//! protected DMA is unavailable there, and expects 15 passes.
//!
//! A panic in a verifier is routed through [`fail_verifier_thread`] (installed
//! by the panic handler) so a crashing verifier atomically fails its own bit
//! instead of hanging the boot.

use alloc::{
    sync::Weak,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};

use concurrent_queue::ConcurrentQueue;
use spin::LazyLock;

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        spawn_thread,
        threads::ThreadId,
    },
    klib::observer::{
        Observable,
        Observer,
    },
    logln,
    memory::KERNEL_ASID,
};

#[derive(Clone, Copy)]
pub struct Deadline {
    expires_at: u64,
}

impl Deadline {
    pub fn after_millis(timeout: u64) -> Self {
        Self {
            expires_at: monotonic_millis().saturating_add(timeout),
        }
    }

    #[track_caller]
    pub fn assert_pending(self, what: &str) {
        assert!(
            monotonic_millis() < self.expires_at,
            "self-test deadline expired while waiting for {}",
            what
        );
    }

    pub fn is_expired(&self) -> bool {
        monotonic_millis() >= self.expires_at
    }
}

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum TestId {
    El0 = 0,
    RaftStorage = 1,
    El0Ipc = 2,
    El0IpcBlocking = 3,
    El0IpcCrossAs = 4,
    El0IpcMemory = 5,
    El0IpcMemoryCancel = 6,
    El0IpcMemoryCopy = 7,
    El0CrossLp = 8,
    PingPong = 9,
    Sitas = 10,
    Service = 11,
    CqWait = 12,
    Device = 13,
    Uart = 14,
    SchedulerLifecycle = 15,
    Nvme = 16,
    Net = 17,
    Disco = 18,
    Dns = 19,
    Tcpip = 20,
    Http = 21,
    Clusterctl = 22,
    Join = 23,
    Dhcp = 24,
    S3 = 25,
    Kafka = 26,
}

impl TestId {
    const ALL: [Self; 27] = [
        Self::El0,
        Self::RaftStorage,
        Self::El0Ipc,
        Self::El0IpcBlocking,
        Self::El0IpcCrossAs,
        Self::El0IpcMemory,
        Self::El0IpcMemoryCancel,
        Self::El0IpcMemoryCopy,
        Self::El0CrossLp,
        Self::PingPong,
        Self::Sitas,
        Self::Service,
        Self::CqWait,
        Self::Device,
        Self::Uart,
        Self::SchedulerLifecycle,
        Self::Nvme,
        Self::Net,
        Self::Disco,
        Self::Dns,
        Self::Tcpip,
        Self::Http,
        Self::Clusterctl,
        Self::Join,
        Self::Dhcp,
        Self::S3,
        Self::Kafka,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::El0 => "el0",
            Self::RaftStorage => "raft-storage",
            Self::El0Ipc => "el0-ipc",
            Self::El0IpcBlocking => "el0-ipc-blocking",
            Self::El0IpcCrossAs => "el0-ipc-cross-as",
            Self::El0IpcMemory => "el0-ipc-memory",
            Self::El0IpcMemoryCancel => "el0-ipc-memory-cancel",
            Self::El0IpcMemoryCopy => "el0-ipc-memory-copy",
            Self::El0CrossLp => "el0-cross-lp",
            Self::PingPong => "ping-pong",
            Self::Sitas => "sitas",
            Self::Service => "service-lifecycle",
            Self::CqWait => "cq-wait",
            Self::Device => "device",
            Self::Uart => "uart",
            Self::SchedulerLifecycle => "scheduler-lifecycle",
            Self::Nvme => "nvme",
            Self::Net => "net",
            Self::Disco => "disco",
            Self::Dns => "dns",
            Self::Tcpip => "tcpip",
            Self::Http => "http",
            Self::Clusterctl => "clusterctl",
            Self::Join => "join",
            Self::Dhcp => "dhcp",
            Self::S3 => "s3-tls",
            Self::Kafka => "kafka",
        }
    }
}

static EXPECTED: AtomicU64 = AtomicU64::new(0);
static PASSED: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
static FINALIZED: AtomicBool = AtomicBool::new(false);
const NO_VERIFIER: u64 = u64::MAX;
static VERIFIER_TIDS: [AtomicU64; TestId::ALL.len()] =
    [const { AtomicU64::new(NO_VERIFIER) }; TestId::ALL.len()];
static VERIFIER_GENERATIONS: [AtomicU64; TestId::ALL.len()] =
    [const { AtomicU64::new(0) }; TestId::ALL.len()];

/// Waiters (verifier threads) parked on the results bitmap. `pass`/`fail`
/// notify them so a verifier waiting for a specific test resolves as soon as
/// the outcome is published — the event-driven counterpart to busy-polling
/// [`has_passed`] with `yield_lp`.
static RESULTS_OBSERVERS: LazyLock<ConcurrentQueue<Weak<dyn Observer>>> =
    LazyLock::new(ConcurrentQueue::unbounded);

struct ResultsObservable;

impl Observable for ResultsObservable {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        let _ = RESULTS_OBSERVERS.push(observer);
    }
}

/// Wake every parked verifier. Runs after a bitmap transition so waiters see
/// the new state when they resume.
fn notify_results_observers() {
    while let Ok(observer) = RESULTS_OBSERVERS.pop() {
        if let Some(observer) = observer.upgrade() {
            observer.notify();
        }
    }
}

/// Drop dead registrations left behind by timed-out waits, keeping the
/// long-lived wait queue bounded.
fn prune_results_observers() {
    let mut live = Vec::new();
    while let Ok(observer) = RESULTS_OBSERVERS.pop() {
        if observer.strong_count() != 0 {
            live.push(observer);
        }
    }
    for observer in live {
        let _ = RESULTS_OBSERVERS.push(observer);
    }
}

/// Block the calling thread until `id` has resolved (passed or failed), or
/// `timeout_ms` elapses. Returns `true` on success, `false` on timeout.
///
/// Parks the thread on [`RESULTS_OBSERVERS`] with a timer watchdog, so the
/// LP idles (and the timer/device wake paths stay live) instead of the
/// verifier busy-spinning with `yield_lp`.
pub fn wait_until_resolved(id: TestId, timeout_ms: u64) -> bool {
    let resolved = crate::cpu::scheduler::block_until(&ResultsObservable, timeout_ms, || {
        has_passed(id) || has_failed(id)
    });
    prune_results_observers();
    resolved
}

const fn bit(id: TestId) -> u64 {
    1 << id as u8
}

pub fn register(id: TestId) {
    assert!(
        !FINALIZED.load(Ordering::Acquire),
        "deferred self-test registered after result set was finalized"
    );
    EXPECTED.fetch_or(bit(id), Ordering::AcqRel);
}

pub fn register_boot_suite() {
    #[cfg(target_arch = "aarch64")]
    {
        let tests = [
            TestId::El0,
            TestId::El0Ipc,
            TestId::El0IpcBlocking,
            TestId::El0IpcCrossAs,
            TestId::El0IpcMemory,
            TestId::El0IpcMemoryCancel,
            TestId::El0IpcMemoryCopy,
            TestId::El0CrossLp,
            TestId::PingPong,
            TestId::Sitas,
            TestId::Service,
            TestId::CqWait,
            TestId::Device,
            TestId::Uart,
            TestId::SchedulerLifecycle,
        ];
        for test in tests {
            register(test);
        }
        #[cfg(not(feature = "hvf_compat"))]
        {
            register(TestId::RaftStorage);
            register(TestId::Nvme);
        }
    }
    // x86_64 runs the shared raw-syscall, userspace, device, service, storage,
    // scheduler, and feature-selected network/cluster suites. PL011 UART and
    // sitas integration remain AArch64-specific. Registering a test that can
    // never resolve would leave the coordinator permanently in
    // `SELFTEST WAITING`.
    #[cfg(target_arch = "x86_64")]
    {
        register(TestId::El0);
        register(TestId::El0Ipc);
        register(TestId::El0IpcBlocking);
        register(TestId::El0IpcCrossAs);
        register(TestId::El0IpcMemory);
        register(TestId::El0IpcMemoryCancel);
        register(TestId::El0IpcMemoryCopy);
        register(TestId::El0CrossLp);
        register(TestId::PingPong);
        register(TestId::CqWait);
        register(TestId::Device);
        register(TestId::Service);
        register(TestId::SchedulerLifecycle);
        #[cfg(not(feature = "hvf_compat"))]
        {
            register(TestId::RaftStorage);
            register(TestId::Nvme);
        }
    }
    #[cfg(all(feature = "virtio_net_test", not(feature = "hvf_compat")))]
    register(TestId::Net);
    #[cfg(feature = "disco_net_test")]
    register(TestId::Disco);
    #[cfg(feature = "dns_net_test")]
    register(TestId::Dns);
    #[cfg(feature = "tcpip_net_test")]
    register(TestId::Tcpip);
    #[cfg(feature = "http_net_test")]
    register(TestId::Http);
    #[cfg(feature = "dhcp_test")]
    register(TestId::Dhcp);
    #[cfg(feature = "s3_test")]
    register(TestId::S3);
    #[cfg(feature = "kafka_test")]
    register(TestId::Kafka);
    #[cfg(feature = "clusterctl_test")]
    {
        register(TestId::Clusterctl);
        register(TestId::Join);
    }
}

pub fn pass(id: TestId) {
    let test_bit = bit(id);
    assert!(
        EXPECTED.load(Ordering::Acquire) & test_bit != 0,
        "unregistered deferred self-test reported success"
    );
    assert_eq!(
        FAILED.load(Ordering::Acquire) & test_bit,
        0,
        "failed deferred self-test later reported success"
    );
    PASSED.fetch_or(test_bit, Ordering::AcqRel);
    VERIFIER_TIDS[id as usize].store(NO_VERIFIER, Ordering::Release);
    VERIFIER_GENERATIONS[id as usize].store(0, Ordering::Release);
    notify_results_observers();
}

pub fn has_passed(id: TestId) -> bool {
    PASSED.load(Ordering::Acquire) & bit(id) != 0
}

pub fn has_failed(id: TestId) -> bool {
    FAILED.load(Ordering::Acquire) & bit(id) != 0
}

pub fn fail(id: TestId) {
    let test_bit = bit(id);
    assert!(
        EXPECTED.load(Ordering::Acquire) & test_bit != 0,
        "unregistered deferred self-test reported failure"
    );
    assert_eq!(
        PASSED.load(Ordering::Acquire) & test_bit,
        0,
        "passed deferred self-test later reported failure"
    );
    FAILED.fetch_or(test_bit, Ordering::AcqRel);
    VERIFIER_TIDS[id as usize].store(NO_VERIFIER, Ordering::Release);
    VERIFIER_GENERATIONS[id as usize].store(0, Ordering::Release);
    notify_results_observers();
}

/// Spawn a deferred verifier and associate its kernel TID with its result bit.
pub fn spawn_verifier(id: TestId, entry: extern "C" fn()) -> ThreadId {
    crate::cpu::scheduler::spawn_thread_after_publish(KERNEL_ASID, entry, |tid, generation| {
        VERIFIER_GENERATIONS[id as usize].store(generation, Ordering::Release);
        VERIFIER_TIDS[id as usize].store(tid as u64, Ordering::Release);
    })
}

/// Panic-handler hook: atomically fail the test owned by `tid`.
///
/// This deliberately performs no allocation, logging, or locking.
pub fn fail_verifier_thread(tid: u64, generation: u64) {
    for (index, verifier) in VERIFIER_TIDS.iter().enumerate() {
        if VERIFIER_GENERATIONS[index].load(Ordering::Acquire) == generation
            && verifier
                .compare_exchange(tid, NO_VERIFIER, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            VERIFIER_GENERATIONS[index].store(0, Ordering::Release);
            FAILED.fetch_or(1 << index, Ordering::AcqRel);
            return;
        }
    }
}

/// Freeze the expected test set and start the sole authoritative reporter.
///
/// Call this after all boot-time deferred tests have been admitted, immediately
/// before the bootstrap processor yields to the scheduler.
pub fn finalize_and_start_coordinator() {
    FINALIZED.store(true, Ordering::Release);
    spawn_thread(KERNEL_ASID, coordinator);
}

extern "C" fn coordinator() {
    let mut next_report = 0u64;
    // Force the first WAITING summary even though nothing has resolved yet.
    let mut last_pending = u64::MAX;
    loop {
        let expected = EXPECTED.load(Ordering::Acquire);
        let passed = PASSED.load(Ordering::Acquire);
        let failed = FAILED.load(Ordering::Acquire);
        if failed != 0 || passed == expected {
            let pending = expected & !(passed | failed);
            for test in TestId::ALL {
                if failed & bit(test) != 0 {
                    logln!("SELFTEST FAILED: {}", test.name());
                }
                if pending & bit(test) != 0 {
                    logln!("SELFTEST PENDING: {}", test.name());
                }
            }
            logln!(
                "SELFTEST COMPLETE: passed={} failed={} pending={} passed_bitmap={:#x} \
                 failed_bitmap={:#x} pending_bitmap={:#x}",
                passed.count_ones(),
                failed.count_ones(),
                pending.count_ones(),
                passed,
                failed,
                pending
            );
            return;
        }
        let pending = expected & !passed;
        let now = monotonic_millis();
        // Report only when a test resolves (the pending set shrinks) or on a
        // slow heartbeat, so the log stays quiet while a long test runs.
        if pending != last_pending || now >= next_report {
            logln!(
                "SELFTEST WAITING: passed={} pending={} passed_bitmap={:#x} pending_bitmap={:#x}",
                passed.count_ones(),
                pending.count_ones(),
                passed,
                pending
            );
            for test in TestId::ALL {
                if pending & bit(test) != 0 {
                    logln!("SELFTEST PENDING: {}", test.name());
                    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
                    if matches!(test, TestId::Device) {
                        let (waiter, driver) = crate::self_test::device::progress();
                        logln!("SELFTEST DEVICE PHASES: waiter={} driver={}", waiter, driver);
                    }
                }
            }
            last_pending = pending;
            next_report = now.saturating_add(10_000);
        }
        // Park on the results observable with a 1s watchdog for the periodic
        // report: the LP idles (and the timer/device wake paths stay live)
        // instead of this reporter busy-spinning. A test resolution wakes us
        // immediately; the watchdog re-admits us for the next WAITING line.
        crate::cpu::scheduler::block_until(&ResultsObservable, 1_000, || {
            let passed = PASSED.load(Ordering::Acquire);
            let failed = FAILED.load(Ordering::Acquire);
            failed != 0 || passed == EXPECTED.load(Ordering::Acquire)
        });
        prune_results_observers();
    }
}
