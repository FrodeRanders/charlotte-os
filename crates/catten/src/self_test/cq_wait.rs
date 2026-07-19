//! Self-test: the wake-aware, blocking CQ wait used by the sitas reactor.
//!
//! Proves that a kernel thread blocked in [`completion::wait_on_cq`] is
//! released by a posted completion, by an explicit [`completion::wake`] with
//! no completion entry, by a per-queue wake on a second shard queue, and —
//! for a CQ-bound endpoint — by an incoming IPC message (the unified shard
//! wait of architecture doc §7 / Phase 7).
//!
//! The waiter and driver run as scheduled kernel threads (the same deferred
//! pattern the EL0 blocking-receive test uses); the flows are robust to
//! scheduling order because all release conditions are also observed by the
//! wait's fast path if they are posted before the waiter blocks.

use core::sync::atomic::{
    AtomicU32,
    AtomicU64,
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
    ipc::{
        self,
        ConnectionRights,
    },
    logln,
    memory::KERNEL_ASID,
};

const CQW_ASID: usize = 0x000c_9a17;
const CQW_SENDER_ASID: usize = 0x000c_9a18;
const MAX_SPINS: u64 = 80_000_000;

/// Phase flags: waiter released by completion / driver started round 2 /
/// waiter released by wake / round 3 (second queue) start and release /
/// round 4 (endpoint readiness) start and release.
static ROUND1_RELEASED: AtomicU32 = AtomicU32::new(0);
static ROUND2_START: AtomicU32 = AtomicU32::new(0);
static ROUND2_RELEASED: AtomicU32 = AtomicU32::new(0);
static ROUND3_START: AtomicU32 = AtomicU32::new(0);
static ROUND3_RELEASED: AtomicU32 = AtomicU32::new(0);
static ROUND4_START: AtomicU32 = AtomicU32::new(0);
static ROUND4_RELEASED: AtomicU32 = AtomicU32::new(0);

/// The CQ-bound endpoint (owner side) and the sender's connection cap.
static ENDPOINT_CAP: AtomicU64 = AtomicU64::new(0);
static SENDER_CONN: AtomicU64 = AtomicU64::new(0);

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

    // Round 4: unified shard wait — the same CQ wait is released by an
    // incoming IPC message on a CQ-bound endpoint (a coalesced readiness
    // wake, not a completion entry). The reactor contract is: released with
    // an empty ring means "inspect your endpoints".
    spin_until(&ROUND4_START, "round 4 start");
    completion::wait_on_cq(CQW_ASID, 0, 1);
    assert_eq!(
        completion::cq_pending(CQW_ASID, 0),
        0,
        "[cq wait] round 4 release must be endpoint readiness, not a completion"
    );
    let endpoint_cap = ENDPOINT_CAP.load(Ordering::Acquire);
    let message = ipc::receive(CQW_ASID, endpoint_cap)
        .expect("[cq wait] readiness wake must find a receivable message");
    assert_eq!(message.opcode, 0x77);
    assert_eq!(message.arg0, 0x1234);
    ROUND4_RELEASED.store(1, Ordering::Release);

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

    ROUND4_START.store(1, Ordering::Release);
    for _ in 0..64 {
        yield_lp();
    }
    let connection = SENDER_CONN.load(Ordering::Acquire);
    ipc::scalar_send(CQW_SENDER_ASID, connection, 0x77, 0x1234)
        .expect("[cq wait] endpoint readiness send failed");
    spin_until(&ROUND4_RELEASED, "endpoint readiness release");

    logln!(
        "[cq wait] SUCCESS: blocking CQ wait released by completion, by explicit wake, by a \
         per-queue wake on a second shard queue, and by CQ-bound endpoint readiness."
    );
}

pub fn test_cq_wait_wake() {
    logln!("Testing blocking CQ wait (completion, wake, and endpoint readiness releases)...");
    completion::open_address_space_with_cq(CQW_ASID, 8, 8);
    completion::open_cq(CQW_ASID, 1, 8);

    // A CQ-bound endpoint: readiness is delivered as a coalesced wake on
    // queue 0 (unified shard wait).
    let endpoint_cap = ipc::endpoint_create(CQW_ASID, 0x4351_4550, 1, 4)
        .expect("[cq wait] endpoint_create failed");
    ipc::endpoint_bind_cq(CQW_ASID, endpoint_cap, 0).expect("[cq wait] endpoint_bind_cq failed");
    let connection =
        ipc::connection_delegate(CQW_ASID, endpoint_cap, CQW_SENDER_ASID, ConnectionRights::SEND)
            .expect("[cq wait] connection_delegate failed");
    ENDPOINT_CAP.store(endpoint_cap, Ordering::Release);
    SENDER_CONN.store(connection, Ordering::Release);

    // A connection cap must not be bindable: only the endpoint owner routes
    // its readiness.
    assert_eq!(
        ipc::endpoint_bind_cq(CQW_SENDER_ASID, connection, 0),
        Err(ipc::IpcError::WrongType),
        "binding a connection cap to a CQ must be rejected"
    );

    let _waiter = spawn_thread(KERNEL_ASID, cq_waiter);
    let _driver = spawn_thread(KERNEL_ASID, cq_driver);
    logln!("[cq wait] waiter and driver deferred");
}
