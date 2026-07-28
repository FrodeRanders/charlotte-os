//! Authoritative completion state for deferred kernel self-tests.

use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        spawn_thread,
        threads::ThreadId,
        yield_lp,
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
}

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum TestId {
    El0 = 0,
    Raft = 1,
    RaftStorage = 2,
    El0Ipc = 3,
    El0IpcBlocking = 4,
    El0IpcCrossAs = 5,
    El0IpcMemory = 6,
    El0IpcMemoryCancel = 7,
    El0IpcMemoryCopy = 8,
    El0CrossLp = 9,
    PingPong = 10,
    Sitas = 11,
    Service = 12,
    CqWait = 13,
    Device = 14,
    Uart = 15,
    SchedulerLifecycle = 16,
    Nvme = 17,
    Net = 18,
}

impl TestId {
    const ALL: [Self; 19] = [
        Self::El0,
        Self::Raft,
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
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::El0 => "el0",
            Self::Raft => "raft",
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
    let tests = [
        TestId::El0,
        TestId::Raft,
        TestId::RaftStorage,
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
        TestId::Nvme,
    ];
    for test in tests {
        register(test);
    }
    #[cfg(all(feature = "virtio_net_test", not(feature = "hvf_compat"), target_arch = "aarch64"))]
    register(TestId::Net);
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
}

pub fn has_passed(id: TestId) -> bool {
    PASSED.load(Ordering::Acquire) & bit(id) != 0
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
}

/// Spawn a deferred verifier and associate its kernel TID with its result bit.
///
/// Boot-time verifiers are admitted before the scheduler starts, so the
/// mapping is published before the new thread can run.
pub fn spawn_verifier(id: TestId, entry: extern "C" fn()) -> ThreadId {
    let tid = spawn_thread(KERNEL_ASID, entry);
    VERIFIER_TIDS[id as usize].store(tid as u64, Ordering::Release);
    tid
}

/// Panic-handler hook: atomically fail the test owned by `tid`.
///
/// This deliberately performs no allocation, logging, or locking.
pub fn fail_verifier_tid(tid: u64) {
    for (index, verifier) in VERIFIER_TIDS.iter().enumerate() {
        if verifier.compare_exchange(tid, NO_VERIFIER, Ordering::AcqRel, Ordering::Acquire).is_ok()
        {
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
        let now = monotonic_millis();
        if now >= next_report {
            let pending = expected & !passed;
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
                }
            }
            next_report = now.saturating_add(1_000);
        }
        // This reporter exists only during boot and exits as soon as all
        // registered tests resolve. A timer sleep made the authoritative
        // result itself vulnerable to the timer-wake path under test; yielding
        // keeps it schedulable without depending on that same mechanism.
        yield_lp();
    }
}
