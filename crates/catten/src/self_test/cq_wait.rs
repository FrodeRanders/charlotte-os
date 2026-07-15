//! Self-test: the wake-aware, blocking CQ wait used by the sitas reactor.
//!
//! Proves that a kernel thread blocked in [`completion::wait_on_cq`] is
//! released both by a posted completion and by an explicit
//! [`completion::wake`] with no completion entry — the two release conditions
//! the migrated `sitas-charlotte` reactor relies on instead of busy polling.
//!
//! The waiter and driver run as scheduled kernel threads (the same deferred
//! pattern the EL0 blocking-receive test uses); the flows are robust to
//! scheduling order because both release conditions are also observed by the
//! wait's fast path if they are posted before the waiter blocks.

use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use crate::{
    completion::{
        self,
        OpCode,
        OpResult,
    },
    cpu::scheduler::{
        spawn_thread,
        yield_lp,
    },
    logln,
    memory::KERNEL_ASID,
};

const CQW_ASID: usize = 0x000c_9a17;
const MAX_SPINS: u64 = 80_000_000;

/// Phase flags: waiter released by completion / driver started round 2 /
/// waiter released by wake / round 3 (second queue) start and release.
static ROUND1_RELEASED: AtomicU32 = AtomicU32::new(0);
static ROUND2_START: AtomicU32 = AtomicU32::new(0);
static ROUND2_RELEASED: AtomicU32 = AtomicU32::new(0);
static ROUND3_START: AtomicU32 = AtomicU32::new(0);
static ROUND3_RELEASED: AtomicU32 = AtomicU32::new(0);

fn spin_until(flag: &AtomicU32, what: &str) {
    let mut spins: u64 = 0;
    while flag.load(Ordering::Acquire) == 0 {
        spins += 1;
        assert!(spins < MAX_SPINS, "[cq wait] FAILED waiting for {}", what);
        yield_lp();
    }
}

extern "C" fn cq_waiter() {
    // Round 1: released by a posted completion entry.
    completion::wait_on_cq(CQW_ASID, 0, 1);
    assert!(
        completion::cq_pending(CQW_ASID, 0) >= 1,
        "[cq wait] round 1 release must observe a pending CQ entry"
    );
    ROUND1_RELEASED.store(1, Ordering::Release);

    // Round 2: released by an explicit wake, with no completion entry.
    spin_until(&ROUND2_START, "round 2 start");
    completion::wait_on_cq(CQW_ASID, 0, 1);
    assert_eq!(
        completion::cq_pending(CQW_ASID, 0),
        0,
        "[cq wait] round 2 release must be a wake, not a completion"
    );
    ROUND2_RELEASED.store(1, Ordering::Release);

    // Round 3: a wait on a second (per-shard) queue is released by a wake
    // targeted at that queue.
    spin_until(&ROUND3_START, "round 3 start");
    completion::wait_on_cq(CQW_ASID, 1, 1);
    assert_eq!(
        completion::cq_pending(CQW_ASID, 1),
        0,
        "[cq wait] round 3 release must be a per-queue wake, not a completion"
    );
    ROUND3_RELEASED.store(1, Ordering::Release);

    loop {
        yield_lp();
    }
}

extern "C" fn cq_driver() {
    // Give the waiter a chance to block first; the fast path covers the case
    // where it has not.
    for _ in 0..64 {
        yield_lp();
    }
    // Round 1: a capability-free operation completes and posts a CQ entry.
    let operation = completion::submit_detached(CQW_ASID, 0, OpCode::Nop, 0xd1)
        .expect("[cq wait] detached submit failed");
    completion::complete_detached(CQW_ASID, operation, OpResult::Ok(1))
        .expect("[cq wait] detached complete failed");
    spin_until(&ROUND1_RELEASED, "completion release");

    // Drain the ring so round 2 can prove a wake releases without entries.
    let ring_ptr = completion::cq_ring_of(CQW_ASID, 0).expect("[cq wait] CQ ring missing");
    while unsafe { &mut *ring_ptr }.read().is_some() {}
    assert_eq!(completion::cq_pending(CQW_ASID, 0), 0);

    ROUND2_START.store(1, Ordering::Release);
    for _ in 0..64 {
        yield_lp();
    }
    completion::wake(CQW_ASID, 0);
    spin_until(&ROUND2_RELEASED, "wake release");

    ROUND3_START.store(1, Ordering::Release);
    for _ in 0..64 {
        yield_lp();
    }
    completion::wake(CQW_ASID, 1);
    spin_until(&ROUND3_RELEASED, "per-queue wake release");

    logln!(
        "[cq wait] SUCCESS: blocking CQ wait released by completion, by explicit wake, and by a \
         per-queue wake on a second shard queue."
    );
    loop {
        yield_lp();
    }
}

pub fn test_cq_wait_wake() {
    logln!("Testing blocking CQ wait (completion and wake release paths)...");
    completion::open_address_space_with_cq(CQW_ASID, 8, 8);
    completion::open_cq(CQW_ASID, 1, 8);
    let _waiter = spawn_thread(KERNEL_ASID, cq_waiter);
    let _driver = spawn_thread(KERNEL_ASID, cq_driver);
    logln!("[cq wait] waiter and driver deferred");
}
