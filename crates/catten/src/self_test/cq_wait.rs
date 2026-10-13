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
    cpu::scheduler::spawn_thread,
    ipc::{
        self,
        ConnectionRights,
    },
    logln,
    memory::KERNEL_ASID,
};

const CQW_ASID: usize = 0x000c_9a17;
const CQW_SENDER_ASID: usize = 0x000c_9a18;
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
static ADMISSION_WORKER_RAN: AtomicU32 = AtomicU32::new(0);

extern "C" fn admission_worker() {
    ADMISSION_WORKER_RAN.store(1, Ordering::Release);
}

fn test_scheduler_waiter_admission() {
    use alloc::sync::Weak;

    use crate::{
        cpu::{
            isa::lp::ops::get_lp_id,
            scheduler::{
                system_scheduler::{
                    Error,
                    SYSTEM_SCHEDULER,
                    get_thread_id,
                    publish_thread,
                },
                threads::{
                    MASTER_THREAD_TABLE,
                    MigrationConstraint,
                    Thread,
                    ThreadState,
                },
            },
        },
        klib::observer::{
            Observable,
            Observer,
            WaitRegistration,
            WaitSponsor,
            registration::{
                ObserverList,
                RegistrationError,
            },
            waiter_budget,
        },
    };
    struct Reject;
    impl Observable for Reject {
        fn try_register_waiter(
            &self,
            _: Weak<dyn Observer>,
            _: &WaitSponsor,
        ) -> Result<WaitRegistration, RegistrationError> {
            Err(RegistrationError::ResourceLimit)
        }
    }
    struct Source(crate::klib::observer::registration::ListRef<waiter_budget::Charge>);
    impl Observable for Source {
        fn try_register_waiter(
            &self,
            observer: Weak<dyn Observer>,
            sponsor: &WaitSponsor,
        ) -> Result<WaitRegistration, RegistrationError> {
            sponsor.register(&self.0, observer)
        }
    }
    let current = get_thread_id().unwrap();
    let constraints = MASTER_THREAD_TABLE.read().get(current).unwrap().migration_constraints;
    assert!(matches!(
        SYSTEM_SCHEDULER.read().block_thread(current, &Reject),
        Err(Error::WaitRegistrationFailed)
    ));
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(current).unwrap();
        assert!(matches!(thread.state, ThreadState::Running(_)));
        assert_eq!(thread.migration_constraints, constraints);
    }

    // This IRQ-safe guard prevents local dispatch while examining a Ready
    // worker. It is pinned to this LP and never certified for migration.
    let prepared = Thread::new(KERNEL_ASID, admission_worker);
    let barrier = crate::cpu::multiprocessor::spin::mutex::Mutex::new(());
    let guard = barrier.lock();
    let worker = publish_thread(prepared).unwrap();
    SYSTEM_SCHEDULER.read().submit_to_lp(worker, get_lp_id()).unwrap();
    assert!(matches!(
        SYSTEM_SCHEDULER.read().block_thread(worker, &Reject),
        Err(Error::WaitRegistrationFailed)
    ));
    let (generation, constraints) = {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(worker).unwrap();
        assert!(matches!(thread.state, ThreadState::Ready(lp) if lp == get_lp_id()));
        assert_eq!(
            thread.migration_constraints, 0,
            "rejected Ready registration must not add constraints"
        );
        (thread.generation, thread.migration_constraints)
    };
    let source = Source(ObserverList::try_new(waiter_budget::SOURCE_LIMIT, false).unwrap());
    SYSTEM_SCHEDULER
        .read()
        .block_thread_with_constraint(worker, &source, MigrationConstraint::GeneralWait)
        .unwrap();
    let retained_waker = {
        let table = MASTER_THREAD_TABLE.read();
        let ThreadState::Blocked(waker) = &table.get(worker).unwrap().state else {
            panic!("admitted worker must be Blocked");
        };
        waker.clone()
    };
    assert_eq!(source.0.registered(), 1);
    SYSTEM_SCHEDULER.read().submit_woken_thread(worker, generation).unwrap();
    assert_eq!(source.0.registered(), 0, "Ready cancels even a retained Waker's registration");
    assert_eq!(MASTER_THREAD_TABLE.read().get(worker).unwrap().migration_constraints, constraints);
    drop(retained_waker);
    drop(guard);
    spin_until(&ADMISSION_WORKER_RAN, "worker left runnable after admission checks");

    // A never-dispatched Blocked thread releases admission on reaping even
    // when its Waker survives. It is never submitted to a runnable queue.
    let staged = publish_thread(Thread::new(KERNEL_ASID, admission_worker)).unwrap();
    assert!(matches!(
        SYSTEM_SCHEDULER.read().block_thread(staged, &Reject),
        Err(Error::WaitRegistrationFailed)
    ));
    assert!(matches!(
        MASTER_THREAD_TABLE.read().get(staged).unwrap().state,
        ThreadState::NeedsLpAssignment
    ));
    let generation = SYSTEM_SCHEDULER
        .read()
        .block_thread_with_constraint_generation(staged, &source, MigrationConstraint::GeneralWait)
        .unwrap();
    let retained_waker = {
        let table = MASTER_THREAD_TABLE.read();
        let ThreadState::Blocked(waker) = &table.get(staged).unwrap().state else {
            panic!("staged worker must be Blocked");
        };
        waker.clone()
    };
    SYSTEM_SCHEDULER.read().abort_thread_generation(staged, generation).unwrap();
    let deadline = crate::self_test::results::Deadline::after_millis(5_000);
    while source.0.registered() != 0 {
        deadline.assert_pending("aborted waiter registration released by reaper");
        crate::cpu::scheduler::yield_lp();
    }
    drop(retained_waker);

    let cap = completion::submit(CQW_ASID, OpCode::Nop, Some(alloc::vec![0xa5; 32])).unwrap();
    for _ in 0..64 {
        assert!(!completion::wait_timeout(CQW_ASID, cap, 1).unwrap());
        assert_eq!(completion::waiter_count(CQW_ASID, cap), 0);
        assert!(!completion::wait_on_cq_timeout(CQW_ASID, 2, 1, 1));
        assert_eq!(completion::cq_waiter_count(CQW_ASID, 2), 0);
        assert_eq!(completion::wait_on_cq_checked(CQW_ASID, 2, Some(1)), Ok(false));
        assert_eq!(completion::cq_waiter_count(CQW_ASID, 2), 0);
    }
    // Exercise both public adapters against the same retained completion.
    // Admission rejection must leave its result, buffer and Running state alone.
    let completion_source = completion::completion_of(CQW_ASID, cap).unwrap();
    let sponsor = WaitSponsor::new(false);
    let observer: alloc::sync::Arc<dyn Observer> = crate::klib::observer::CallOnNotify::new(|| {});
    let tokens: alloc::vec::Vec<_> = (0..waiter_budget::SOURCE_LIMIT)
        .map(|_| {
            completion_source
                .try_register_waiter(alloc::sync::Arc::downgrade(&observer), &sponsor)
                .unwrap()
        })
        .collect();
    let (_, timers) = crate::timers::thread_timer_context(current).unwrap();
    // Do not compare the ambient timer account across fallible preparation:
    // allocator contention may schedule, and older cancellations may reclaim.
    assert_eq!(
        completion::wait_timeout(CQW_ASID, cap, 10_000),
        Err(completion::CapError::WaitAdmissionFailed)
    );
    assert_eq!(
        crate::self_test::syscall::test_completion_wait_result(CQW_ASID, cap, 10_000),
        (catten_syscall::completion_status::WAIT_ADMISSION_FAILED, 0)
    );
    assert_eq!(completion::waiter_count(CQW_ASID, cap), waiter_budget::SOURCE_LIMIT);
    assert!(matches!(
        MASTER_THREAD_TABLE.read().get(current).unwrap().state,
        ThreadState::Running(_)
    ));
    assert_eq!(completion::state_of(CQW_ASID, cap), Ok(completion::OpStateKind::InFlight));
    drop(tokens);
    // All vector allocation precedes the non-parking counter probe.
    let mut charges = alloc::vec::Vec::with_capacity(crate::timers::budget::MAX_DOMAIN_TIMERS);
    {
        // Saturation also guarantees rejection; keep old queue reclamation
        // from changing this exact sponsor during the counter/charge probe.
        let _setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        let timer_baseline = timers.used();
        // The unpublished driver receives a private fixture sponsor below.
        // Production kernel threads share their root's sponsor, so saturating
        // the ordinary captured kernel account would affect other LPs.
        while let Ok(charge) = timers.reserve() {
            charges.push(charge);
        }
        assert_eq!(timers.used(), crate::timers::budget::MAX_DOMAIN_TIMERS);
        assert_eq!(completion::waiter_count(CQW_ASID, cap), 0);
        assert_eq!(
            completion::wait_timeout(CQW_ASID, cap, 10_000),
            Err(completion::CapError::WaitAdmissionFailed)
        );
        assert_eq!(
            crate::self_test::syscall::test_completion_wait_result(CQW_ASID, cap, 10_000),
            (catten_syscall::completion_status::WAIT_ADMISSION_FAILED, 0)
        );
        assert_eq!(completion::wait_timeout(CQW_ASID, cap, 0), Ok(false));
        assert_eq!(
            crate::self_test::syscall::test_completion_wait_result(CQW_ASID, cap, 0),
            (catten_syscall::completion_status::PENDING_OR_TIMEOUT, 0)
        );
        assert_eq!(completion::waiter_count(CQW_ASID, cap), 0);
        assert_eq!(completion::state_of(CQW_ASID, cap), Ok(completion::OpStateKind::InFlight));
        assert!(matches!(
            MASTER_THREAD_TABLE.read().get(current).unwrap().state,
            ThreadState::Running(_)
        ));
        charges.clear(); // Counter-only Drop; keep vector deallocation outside the mask.
        assert!(completion::holds_buffer(CQW_ASID, cap).unwrap());
        assert_eq!(timers.used(), timer_baseline);
    }
    drop(charges);
    assert_eq!(sponsor.used(), 0);
    drop(completion_source);
    let cq_source = completion::capture_cq_interest(CQW_ASID, 2).unwrap();
    let sponsor = WaitSponsor::new(false);
    let observer: alloc::sync::Arc<dyn Observer> = crate::klib::observer::CallOnNotify::new(|| {});
    let tokens: alloc::vec::Vec<_> = (0..waiter_budget::SOURCE_LIMIT)
        .map(|_| {
            cq_source.try_register_waiter(alloc::sync::Arc::downgrade(&observer), &sponsor).unwrap()
        })
        .collect();
    for timeout in [None, Some(10_000)] {
        assert_eq!(
            completion::wait_on_cq_checked(CQW_ASID, 2, timeout),
            Err(completion::CqWaitError::AdmissionRejected)
        );
        assert_eq!(completion::cq_waiter_count(CQW_ASID, 2), waiter_budget::SOURCE_LIMIT);
        assert!(matches!(
            MASTER_THREAD_TABLE.read().get(get_thread_id().unwrap()).unwrap().state,
            ThreadState::Running(_)
        ));
    }
    drop(tokens);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(
        completion::wait_on_cq_checked(CQW_ASID, u32::MAX, Some(0)),
        Err(completion::CqWaitError::UnknownQueue)
    );
    completion::complete(CQW_ASID, cap, OpResult::Ok(0)).unwrap();
    completion::close(CQW_ASID, cap).unwrap();
    logln!(
        "[waiters] SUCCESS: Running/Ready rejection is non-mutating; retained-Waker cancellation; \
         64 completion/CQ timeout cleanups"
    );
}

// Private scheduled fixture state only. Parent owns the target completion and
// both exact-generation exit subscriptions until each child has terminated.
static TIMED_TARGET: crate::memory::Mutex<Option<(usize, u64, u64)>> =
    crate::memory::Mutex::new(None);
static TIMED_REPARKS: AtomicU32 = AtomicU32::new(0);

extern "C" fn unrelated_wake_worker() {
    use crate::cpu::scheduler::{
        self,
        system_scheduler::SYSTEM_SCHEDULER,
        threads::{
            MASTER_THREAD_TABLE,
            ThreadState,
        },
    };
    let (tid, generation, cap) = TIMED_TARGET.lock().take().unwrap();
    let wait_for_block = || {
        let deadline = crate::self_test::results::Deadline::after_millis(5_000);
        loop {
            let blocked = MASTER_THREAD_TABLE.read().get(tid).is_ok_and(|thread| {
                thread.generation == generation && matches!(thread.state, ThreadState::Blocked(_))
            });
            if blocked && completion::waiter_count(CQW_ASID, cap) == 1 {
                return;
            }
            deadline.assert_pending("timed completion waiter must register/repark");
            scheduler::sleep_millis(1); // Bounded fixture observation, no production polling.
        }
    };
    wait_for_block();
    {
        // Both fixture threads are pinned to this LP. Keep wake and cancelled
        // registration observation atomic against a quantum/repark race.
        let _setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation).unwrap();
        assert_eq!(completion::waiter_count(CQW_ASID, cap), 0);
    }
    wait_for_block();
    TIMED_REPARKS.fetch_add(1, Ordering::Release);
    completion::complete(CQW_ASID, cap, OpResult::Ok(42)).unwrap();
}

fn test_timed_completion_wakes() {
    use crate::cpu::scheduler::{
        self,
        system_scheduler::get_thread_id,
        threads::MASTER_THREAD_TABLE,
    };
    let tid = get_thread_id().unwrap();
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    TIMED_REPARKS.store(0, Ordering::Release);
    for syscall in [false, true] {
        let cap = completion::submit(CQW_ASID, OpCode::Nop, Some(alloc::vec![0xa5; 32])).unwrap();
        *TIMED_TARGET.lock() = Some((tid, generation, cap));
        let mut worker_generation = 0;
        let worker = scheduler::spawn_thread_after_publish(
            KERNEL_ASID,
            unrelated_wake_worker,
            |_, captured| worker_generation = captured,
        );
        let exited = completion::observe_thread_exit_with_generation(
            CQW_ASID,
            worker,
            Some(worker_generation),
        )
        .unwrap();
        if syscall {
            assert_eq!(
                crate::self_test::syscall::test_completion_wait_result(CQW_ASID, cap, 5_000),
                (catten_syscall::completion_status::READY, 42)
            );
        } else {
            assert_eq!(completion::wait_timeout(CQW_ASID, cap, 5_000), Ok(true));
            let result = completion::poll(CQW_ASID, cap).unwrap().unwrap();
            assert_eq!(result.result, OpResult::Ok(42));
            assert_eq!(result.buffer.unwrap(), alloc::vec![0xa5; 32]);
        }
        assert_eq!(completion::waiter_count(CQW_ASID, cap), 0);
        completion::close(CQW_ASID, cap).unwrap();
        assert_eq!(completion::wait_timeout(CQW_ASID, exited, 5_000), Ok(true));
        completion::close(CQW_ASID, exited).unwrap();
    }
    assert_eq!(TIMED_REPARKS.load(Ordering::Acquire), 2);
    logln!(
        "[completion timed wait] kernel/dispatcher source and watchdog rejection preserve pending \
         ownership; unrelated wakes repark before actual completion with the original deadline"
    );
}

fn spin_until(flag: &AtomicU32, what: &str) {
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    while flag.load(Ordering::Acquire) == 0 {
        deadline.assert_pending(what);
        crate::cpu::scheduler::yield_lp();
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
    // Deliberately do not order the driver behind the waiter. Every release
    // condition is persistent (ring entry, work generation, or queued IPC),
    // so this also exercises the wait fast path without depending on timer
    // wakeups or an arbitrary scheduling delay.
    // Round 1: a capability-free operation completes and posts a CQ entry.
    let operation = completion::submit_detached(CQW_ASID, 0, OpCode::Nop, 0xd1)
        .expect("[cq wait] detached submit failed");
    completion::complete_detached(CQW_ASID, operation, OpResult::Ok(1))
        .expect("[cq wait] detached complete failed");
    spin_until(&ROUND1_RELEASED, "completion release");

    // Drain the ring so round 2 can prove a wake releases without entries.
    let ring_ptr =
        unsafe { completion::cq_ring_of(CQW_ASID, 0) }.expect("[cq wait] CQ ring missing");
    while unsafe { &mut *ring_ptr }.read().is_some() {}
    assert_eq!(completion::cq_pending(CQW_ASID, 0), 0);

    ROUND2_START.store(1, Ordering::Release);
    completion::wake(CQW_ASID, 0);
    spin_until(&ROUND2_RELEASED, "wake release");

    ROUND3_START.store(1, Ordering::Release);
    completion::wake(CQW_ASID, 1);
    spin_until(&ROUND3_RELEASED, "per-queue wake release");

    ROUND4_START.store(1, Ordering::Release);
    let connection = SENDER_CONN.load(Ordering::Acquire);
    ipc::scalar_send(CQW_SENDER_ASID, connection, 0x77, 0x1234)
        .expect("[cq wait] endpoint readiness send failed");
    spin_until(&ROUND4_RELEASED, "endpoint readiness release");

    test_scheduler_waiter_admission();
    ipc::waiter_tests::test_scheduled_cleanup();
    ipc::waiter_tests::test_scheduled_reply_cq();
    crate::cpu::scheduler::sync::tests::test_scheduled_cleanup();
    crate::timers::waiter_tests::test_scheduled_cleanup();
    crate::timers::event_tests::test_scheduled_cleanup();
    completion::exit_tests::test_scheduled_cleanup();
    completion::callback_tests::test_scheduled_cleanup();
    test_timed_completion_wakes();
    crate::memory::object::test_scheduled_transfer_close();
    crate::device::test_scheduled_creation_readiness();
    ipc::claim_wait::tests::scheduled();

    logln!(
        "[cq wait] SUCCESS: blocking CQ wait released by completion, by explicit wake, by a \
         per-queue wake on a second shard queue, and by CQ-bound endpoint readiness."
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::CqWait);
}

pub fn test_cq_wait_wake() {
    logln!("Testing blocking CQ wait (completion, wake, and endpoint readiness releases)...");
    completion::open_address_space_with_cq(CQW_ASID, 8, 8).expect("CQ setup failed");
    completion::open_cq(CQW_ASID, 1, 8).expect("CQ setup failed");
    completion::open_cq(CQW_ASID, 2, 8).expect("CQ setup failed");

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
    // Raw boot accounting fixture only: isolate the driver before publication
    // or any watchdog captures its sponsor. Production Thread construction
    // keeps its generation-owned root account; never replace a live sponsor.
    // This trusted kernel fixture retains platform classification and uses
    // the existing admitted timer budget, not a new registry or bypass.
    let mut driver = crate::cpu::scheduler::threads::Thread::new(KERNEL_ASID, cq_driver);
    driver.timer_sponsor = crate::timers::budget::SchedulerSponsor::new(true);
    let driver = crate::cpu::scheduler::system_scheduler::publish_thread(driver)
        .expect("[cq wait] driver publication rejected");
    crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
        .read()
        .submit_new_thread(driver)
        .expect("[cq wait] driver scheduler admission rejected");
    logln!("[cq wait] waiter and driver deferred");
}
