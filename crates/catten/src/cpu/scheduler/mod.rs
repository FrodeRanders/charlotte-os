//! Kernel thread scheduler — spawn, block, abort, yield.
//!
//! A thread is a kernel-scheduled execution context. The scheduler
//! assigns threads to logical processors (LPs), runs a per-LP
//! round-robin policy with a configurable quantum, and provides
//! cooperative `yield_lp`, blocking `sleep`, and thread exit.
//!
//! Threads carry an optional LP affinity — a preference for a
//! specific LP set at first admission.  Re-admission after a wake
//! (timer, endpoint message, device interrupt) returns the thread
//! to its affinity LP rather than the globally least-loaded one.

use alloc::sync::Weak;
use core::{
    hint::unreachable_unchecked,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use crate::{
    cpu::scheduler::{
        system_scheduler::{
            SYSTEM_SCHEDULER,
            publish_thread,
        },
        threads::{
            MASTER_THREAD_TABLE,
            Thread,
            ThreadGeneration,
            ThreadId,
        },
    },
    klib::{
        observer::{
            Observable,
            Observer,
        },
        time::duration::ExtDuration,
    },
    logln,
    memory::AddressSpaceId,
    timers::TimerEvent,
};

pub mod lp_schedulers;
pub mod sync;
pub mod system_scheduler;
pub mod threads;

const SCHED_TRACE: bool = false;
const REBALANCE_SAMPLE_MILLIS: u64 = 10;
static LAST_REBALANCE_SAMPLE_MILLIS: AtomicU64 = AtomicU64::new(0);

/// Diagnostic only: sleeps that remained runnable after waiter rejection.
#[unsafe(no_mangle)]
pub static SLEEP_WAIT_ADMISSION_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Current monotonic time in milliseconds since the architecture counter's
/// epoch. Observe that this is not wall-clock time and is meant to handle
/// deadlines.
pub fn monotonic_millis() -> u64 {
    use crate::cpu::isa::{
        interface::timers::LpTimerIfce,
        timers::LpTimer,
    };

    ((LpTimer::now() as u128 * LpTimer::get_ts_cycle_period().as_picos()) / 1_000_000_000) as u64
}

/// Raw architecture-counter ticks used for low-overhead interval statistics.
///
/// Snapshots carry the counter frequency so userspace can convert ticks to
/// seconds without requiring floating point in the kernel.
pub fn monotonic_ticks() -> u64 {
    use crate::cpu::isa::{
        interface::timers::LpTimerIfce,
        timers::LpTimer,
    };

    LpTimer::now()
}

pub fn counter_frequency_hz() -> u64 {
    use crate::cpu::isa::{
        interface::timers::LpTimerIfce,
        timers::LpTimer,
    };

    1_000_000_000_000 / LpTimer::get_ts_cycle_period().as_picos() as u64
}

/// Creates a new thread and submit it to the system scheduler for assignment
/// to a logical processor (LP) and then execution.
pub fn spawn_thread(asid: AddressSpaceId, entry_point: extern "C" fn()) -> ThreadId {
    spawn_thread_with_migration(asid, entry_point, false)
}

/// Create a thread, publish caller-owned metadata keyed by its TID, and only
/// then admit it to the scheduler. This closes the spawn-vs-first-instruction
/// race for facilities such as self-test panic attribution.
pub(crate) fn spawn_thread_after_publish(
    asid: AddressSpaceId,
    entry_point: extern "C" fn(),
    publish: impl FnOnce(ThreadId, threads::ThreadGeneration),
) -> ThreadId {
    let thread = Thread::new(asid, entry_point);
    let tid = publish_thread(thread).expect("address space rejected thread publication");
    let generation = MASTER_THREAD_TABLE
        .read()
        .get(tid)
        .expect("newly published thread disappeared before admission")
        .generation;
    publish(tid, generation);
    let lp = crate::cpu::isa::lp::ops::get_lp_id();
    SYSTEM_SCHEDULER
        .read()
        .submit_to_lp(tid, lp)
        .expect("Error submitting published thread to caller LP");
    tid
}

/// Spawn non-migratable work on a specific LP.
///
/// This is used when the creator is about to block on the new thread's
/// startup handshake. Placing both on one LP makes that block handing
/// execution directly to the child without depending on a remote admission IPI.
pub fn spawn_thread_on_lp(
    asid: AddressSpaceId,
    entry_point: extern "C" fn(),
    lp: crate::cpu::isa::lp::LpId,
) -> ThreadId {
    let thread = Thread::new(asid, entry_point);
    let tid = publish_thread(thread).expect("address space rejected thread publication");
    SYSTEM_SCHEDULER.read().submit_to_lp(tid, lp).expect("Error submitting thread to requested LP");
    tid
}

/// Fallible user-domain launch; failure leaves no published thread.
pub(crate) fn try_spawn_thread_on_lp(
    asid: AddressSpaceId,
    entry_point: extern "C" fn(),
    lp: crate::cpu::isa::lp::LpId,
) -> Result<ThreadId, system_scheduler::Error> {
    if lp >= crate::cpu::multiprocessor::get_lp_count() {
        return Err(system_scheduler::Error::InvalidThread);
    }
    let thread = Thread::try_new(asid, entry_point)?;
    let generation = thread.generation;
    let tid = publish_thread(thread)?;
    let submission = SYSTEM_SCHEDULER.read().submit_to_lp(tid, lp);
    if let Err(error) = submission {
        let removed = {
            let mut table = MASTER_THREAD_TABLE.write();
            if table.get(tid).is_ok_and(|thread| thread.generation == generation) {
                table.take_element(tid).ok()
            } else {
                None
            }
        };
        if let Some(thread) = removed {
            let _ = thread.release_unstarted();
        }
        return Err(error);
    }
    Ok(tid)
}

/// Spawn work whose creator explicitly certifies that it owns no LP-local
/// resources while Ready. This is intentionally separate from `spawn_thread`:
/// migration must be opt-in, never inferred from scheduler state alone.
pub fn spawn_migratable_thread(asid: AddressSpaceId, entry_point: extern "C" fn()) -> ThreadId {
    spawn_thread_with_migration(asid, entry_point, true)
}

/// Spawn certified migratable work with an explicit initial soft placement.
pub fn spawn_migratable_thread_on_lp(
    asid: AddressSpaceId,
    entry_point: extern "C" fn(),
    lp: crate::cpu::isa::lp::LpId,
) -> ThreadId {
    let mut thread = Thread::new(asid, entry_point);
    thread.migration_safe = true;
    let tid = publish_thread(thread).expect("address space rejected thread publication");
    SYSTEM_SCHEDULER
        .read()
        .submit_migratable_to_lp(tid, lp)
        .expect("Error submitting migratable thread to requested LP");
    tid
}

fn spawn_thread_with_migration(
    asid: AddressSpaceId,
    entry_point: extern "C" fn(),
    migration_safe: bool,
) -> ThreadId {
    let mut thread = Thread::new(asid, entry_point);
    thread.migration_safe = migration_safe;
    let tid = publish_thread(thread).expect("address space rejected thread publication");
    SYSTEM_SCHEDULER
        .read()
        .submit_new_thread(tid as ThreadId)
        .expect("Error submitting ready thread to system scheduler");
    tid
}

pub fn maybe_sample_rebalance() {
    use crate::cpu::isa::lp::ops::get_lp_id;

    if get_lp_id() != 0 {
        return;
    }
    let now_millis = monotonic_millis();
    let previous = LAST_REBALANCE_SAMPLE_MILLIS.load(Ordering::Relaxed);
    if now_millis.saturating_sub(previous) < REBALANCE_SAMPLE_MILLIS
        || LAST_REBALANCE_SAMPLE_MILLIS
            .compare_exchange(previous, now_millis, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    SYSTEM_SCHEDULER.read().try_rebalance_sustained(now_millis);
}

/// Returns the address-space id of the currently running thread, if execution
/// is currently inside a scheduler-managed thread context.
pub fn current_thread_asid() -> Option<AddressSpaceId> {
    let tid = system_scheduler::get_thread_id()?;
    MASTER_THREAD_TABLE.read().get(tid).ok().map(|thread| thread.asid)
}

/// Unconditionally yields the current logical processor to the scheduler
/// for a context switch.
///
/// This can safely be called from anywhere including outside of thread context.
/// However, if it is called from interrupt context then it will cause an immediate
/// context switch, never to return, which will essentially cause the remainder of
/// the ISR to get skipped. This is almost never what is intended!
///
/// For interrupt service it is recommended instead to set the context switch pending
/// variable on the current LP's local scheduler and then have the switch happen at
/// the end of the ISR, at which point all ISRs (with the sole exception of double
/// fault and other ISA specific analogues) call `cond_yield_lp` to carry out pending
/// context switches.
pub fn yield_lp() {
    // Deliver any device-interrupt wakes queued from interrupt context
    // (architecture doc §10.2): the interrupt path is lock-free and defers the
    // actual `completion::wake` to thread context. Draining here (on every
    // cooperative yield across every LP) makes a driver blocked in `CQ_WAIT`
    // promptly runnable without the interrupt handler ever taking a lock.
    crate::device::drain_deferred_wakes();
    if SCHED_TRACE {
        let sched = SYSTEM_SCHEDULER.read();
        let lsched = sched.get_lp_scheduler().lock();
        let current = lsched.get_tid();
        let pending = lsched.is_ctx_switch_pending();
        let idle = lsched.is_idle();
        drop(lsched);
        drop(sched);
        logln!(
            "[sched] yield_lp LP{:?} current={:?} ctx_pending={} idle={}",
            crate::cpu::isa::lp::ops::get_lp_id(),
            current,
            pending,
            idle
        );
    }
    SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().set_ctx_switch_pending();
    crate::cpu::isa::lp::ops::cond_yield_lp();
}

/// Aborts the current thread without calling any exit handlers.
///
/// This is the default way to exit a thread in the kernel since kernel threads
/// should not carry any state that is so complex that it requires exit handlers.
/// For the userspace exit call, this should only be called after exit handlers
/// have been run and any pending upcalls have been attempted to be delivered.
/// It is expected that exit handlers will be called from userspace itself via a
/// given program's runtime library. However, upcalls are still solely the purview
/// of the kernel and we should at least attempt delivery prior to abort.
pub fn abort() -> ! {
    // Bind `tid` to a value so the temporary SYSTEM_SCHEDULER read guard and LP
    // scheduler lock in the scrutinee are released before the body runs;
    // otherwise `abort_thread` (which re-locks the LP scheduler) would deadlock.
    let tid = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid();
    if let Some(tid) = tid {
        let lp = crate::cpu::isa::lp::ops::get_lp_id();
        let asid = MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .map(|thread| thread.asid)
            .unwrap_or(crate::memory::KERNEL_ASID);
        logln!("[thread] abort tid={} lp={} asid={}", tid, lp, asid);
        match SYSTEM_SCHEDULER.read().abort_thread(tid) {
            Ok(_) | Err(system_scheduler::Error::InvalidThread) => {}
            Err(error) => panic!("Error aborting thread: {:?}", error),
        }
    }
    yield_lp();
    unsafe { unreachable_unchecked() }
}

/// Abort every thread that belongs to `asid`, including the caller.
///
/// Running threads on remote LPs are first marked for acknowledged abort and
/// interrupted. The exact root is fenced against new thread publication; the
/// calling thread remains owned until this LP switches away from its stack.
pub fn abort_address_space(asid: AddressSpaceId) -> ! {
    assert_ne!(asid, crate::memory::KERNEL_ASID, "refusing to abort the kernel address space");
    crate::early_logln!("Aborting user address space {}", asid);
    let handle = crate::memory::current_address_space_handle(asid)
        .expect("aborting thread's address space missing");
    system_scheduler::abort_domain_threads(handle).expect("domain thread abort rejected");
    yield_lp();
    unsafe { unreachable_unchecked() }
}

/// Return the current kernel thread without waiting on scheduler locks.
///
/// Panic handling uses this best-effort path to avoid deadlocking if the panic
/// occurred while scheduler state was already locked.
pub fn current_tid_nonblocking() -> Option<ThreadId> {
    let scheduler = SYSTEM_SCHEDULER.try_read()?;
    let local = scheduler.get_lp_scheduler();
    local.try_lock()?.get_tid()
}

pub fn current_thread_identity_nonblocking() -> Option<(ThreadId, threads::ThreadGeneration)> {
    let scheduler = SYSTEM_SCHEDULER.try_read()?;
    let local = scheduler.get_lp_scheduler();
    local.try_lock()?.get_current_handle()
}

/// Wait for at least the specified duration. Normally parks the thread; if
/// waiter admission fails, cooperatively waits while runnable without queuing
/// an event. This void ABI cannot report pressure as a successful early sleep.
pub fn sleep(duration: ExtDuration) {
    sleep_with_event(duration, TimerEvent::from(duration));
}

/// Shared acquisition path; kernel fixtures can supply a full source to force
/// rejection without saturating the node's live scheduler sponsor.
pub(crate) fn sleep_with_event(duration: ExtDuration, timer_event: TimerEvent) {
    // Bind `tid` first so the read guard + LP scheduler lock in the scrutinee
    // are released before `block_thread` (which takes SYSTEM_SCHEDULER.write());
    // holding the read guard across the write would deadlock the RwLock.
    let tid = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid();
    if let Some(tid) = tid {
        // Publishing Blocked before the timer is queued must be one local
        // non-preemptible transition. Otherwise a scheduler-quantum IRQ can
        // switch this thread out in the gap, leaving it permanently Blocked
        // with no event capable of waking it. Once the event is in the local
        // queue it is safe to restore IRQs before yielding: if it fires first,
        // the scheduler's wake-before-save handling re-admits this thread.
        let setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        let prepared = crate::timers::thread_timer_context(tid)
            .ok_or(())
            .and_then(|(_, sponsor)| timer_event.prepare_sleep(&sponsor));
        let Ok(mut prepared) = prepared else {
            drop(setup);
            sleep_runnable(duration);
            return;
        };
        let admitted = SYSTEM_SCHEDULER
            .read()
            .block_thread_with_constraint(
                tid,
                prepared.event(),
                threads::MigrationConstraint::TimerWait,
            )
            .is_ok();
        // Start the requested interval only after the blocked state and waker
        // are installed. In particular, a 1 ms boot-time sleep must not arrive
        // already expired after contending on the global scheduler locks.
        prepared.event_mut().reset_after(duration);
        if !admitted {
            // No Blocked state, retained source or queued timer on rejection.
            drop(prepared);
            drop(setup);
            sleep_runnable(duration);
            return;
        }
        prepared.enqueue();
        drop(setup);
        // Yield so the sleep takes effect: `block_thread` marks the thread
        // Blocked and registers its waker on the timer event; this yield saves
        // the thread's context and switches away. When the timer expires it
        // fires the waker, re-admitting the thread, which resumes here.
        yield_lp();
    }
}

fn sleep_runnable(duration: ExtDuration) {
    let deadline = TimerEvent::from(duration).get_deadline();
    SLEEP_WAIT_ADMISSION_FALLBACKS.fetch_add(1, Ordering::Relaxed);
    while monotonic_ticks() < deadline {
        yield_lp();
    }
}

pub fn sleep_millis(milliseconds: u64) {
    sleep(ExtDuration::from_millis(milliseconds as u128));
}

/// Block the current thread until `condition` holds, waking on `observable`
/// notifications, with a `timeout_ms` watchdog that re-admits the thread when
/// it expires. Returns whether `condition` held after the wait.
///
/// This is the event-driven counterpart to the busy `yield_lp` poll loops:
/// the thread parks in `Blocked` state with its waker registered on
/// `observable` (exactly like `sleep`, `wait_reply`, and `cq_wait`), so the
/// LP is free to idle, drain deferred device wakes, and deliver timer PPIs
/// while the wait is in flight.
///
/// The deadline watchdog is a timer event whose observer re-submits the thread,
/// so a missed observable notification cannot hang the system silently — the
/// caller re-checks `condition` and fails loudly.
///
/// Returns `false` if the condition is still false at timeout or waiter
/// admission failure. Owned sources cancel the competing registration when
/// the thread becomes Ready. Every scheduler source uses owning registration.
/// `condition` must be a short, non-parking check: the lost-wake recheck runs
/// under a local interrupt mask, which is released before yielding.
pub fn block_until(
    observable: &dyn Observable,
    timeout_ms: u64,
    condition: impl Fn() -> bool,
) -> bool {
    struct BlockTimeoutWake {
        tid: ThreadId,
        generation: threads::ThreadGeneration,
    }
    impl Observer for BlockTimeoutWake {
        fn notify(self: alloc::sync::Arc<Self>) {
            let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(self.tid, self.generation);
        }
    }

    let deadline = monotonic_millis().saturating_add(timeout_ms);
    loop {
        if condition() {
            return true;
        }
        let now = monotonic_millis();
        if now >= deadline {
            return false;
        }
        let tid = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid();
        let Some(tid) = tid else {
            return condition();
        };
        // Do not let a quantum switch out a newly Blocked thread before its
        // watchdog is queued. Early rejection restores IRQ state through Drop.
        let setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        let Some((expected_generation, sponsor)) = crate::timers::thread_timer_context(tid) else {
            return condition();
        };
        let Ok(timeout_obs) = alloc::sync::Arc::try_new(BlockTimeoutWake {
            tid,
            generation: expected_generation,
        }) else {
            return condition();
        };
        let remaining = deadline.saturating_sub(now).max(1);
        let Ok((timer_event, timeout_handle)) =
            crate::timers::prepare_watchdog(ExtDuration::from_millis(remaining as u128), &sponsor)
        else {
            return condition();
        };
        timer_event
            .event()
            .register_observer(alloc::sync::Arc::downgrade(&timeout_obs) as Weak<dyn Observer>);
        let generation = match SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
            tid,
            observable,
            threads::MigrationConstraint::GeneralWait,
        ) {
            Ok(generation) => generation,
            Err(_) => return condition(),
        };

        // Watchdog: re-admit the thread when the absolute deadline expires
        // even if the observable never fires. A notification is only a hint:
        // shared observables may wake for an unrelated state transition, so
        // the outer loop re-checks and parks again with the remaining budget.
        debug_assert_eq!(generation, expected_generation);
        timer_event.enqueue();

        // Lost-wake guard: if the condition became true while the waker was
        // being registered, re-admit the thread before it yields.
        if condition() {
            let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
        }
        drop(setup);
        yield_lp();

        // Observable notification may have won. Do not retain the unused
        // watchdog until the original absolute deadline.
        let _ = crate::timers::cancel_event(timeout_handle);
    }
}

/// Registers an observer to be notified when the specified thread exits.
///
/// The master-thread table is taken in **write** mode for the whole
/// check-and-register sequence. Thread retirement (`retire_requested_threads`)
/// removes the dying thread from the same table, so this write lock makes the
/// race between "the thread is still registered" and "the thread is already
/// gone" impossible: either the observer is registered before the thread is
/// taken (and fires when the thread is dropped), or the lookup fails here and
/// the caller completes the capability immediately.
///
/// Without the write lock, a thread could be taken and dropped between the
/// lookup and the registration, orphaning the observer forever.
/// The returned token owns cancellation. InvalidThread means absent/stale;
/// WaitRegistrationFailed is admission/allocation failure, never proof of exit.
pub(crate) fn observe_thread_exit(
    thread_id: ThreadId,
    observer: Weak<dyn Observer>,
    charge: crate::completion::watch_budget::Charge,
) -> Result<threads::exit_source::ExitRegistration, system_scheduler::Error> {
    observe_thread_exit_matching(thread_id, None, None, observer, charge)
}

/// Generation-bound variant of [`observe_thread_exit`].
///
/// Thread IDs are recycled (`IdTable` LIFO reuse), so by the time a caller
/// registers an exit observer the slot may already hold a *different* thread
/// that shares the caller's tid. Registering on that thread would leave the
/// joiner waiting for an exit that never comes (or, worse, complete on the
/// wrong thread's drop).
///
/// Callers that captured a thread at spawn time — e.g. a `ServiceDomain` handle —
/// must pass its `generation` so a recycled slot is reported as `InvalidThread`,
/// letting the caller complete immediately instead of joining a stranger.
pub(crate) fn observe_thread_exit_with_generation(
    thread_id: ThreadId,
    expected_generation: ThreadGeneration,
    observer: Weak<dyn Observer>,
    charge: crate::completion::watch_budget::Charge,
) -> Result<threads::exit_source::ExitRegistration, system_scheduler::Error> {
    observe_thread_exit_matching(thread_id, Some(expected_generation), None, observer, charge)
}

pub(crate) fn observe_thread_exit_in_domain(
    caller: crate::memory::AddressSpaceHandle,
    thread_id: ThreadId,
    expected_generation: Option<ThreadGeneration>,
    observer: Weak<dyn Observer>,
    charge: crate::completion::watch_budget::Charge,
) -> Result<threads::exit_source::ExitRegistration, system_scheduler::Error> {
    observe_thread_exit_matching(thread_id, expected_generation, Some(caller), observer, charge)
}

fn observe_thread_exit_matching(
    thread_id: ThreadId,
    expected_generation: Option<ThreadGeneration>,
    caller: Option<crate::memory::AddressSpaceHandle>,
    observer: Weak<dyn Observer>,
    charge: crate::completion::watch_budget::Charge,
) -> Result<threads::exit_source::ExitRegistration, system_scheduler::Error> {
    let table = MASTER_THREAD_TABLE.write();
    if let Ok(thread) = table.get(thread_id) {
        if caller.is_some_and(|caller| thread.address_space != Some(caller)) {
            return Err(system_scheduler::Error::PermissionDenied);
        }
        let generation_matches = expected_generation.is_none_or(|expected| {
            let captured = charlotte_lifecycle::ThreadIdentity::new(thread_id as u64, expected);
            let current =
                charlotte_lifecycle::ThreadIdentity::new(thread_id as u64, thread.generation);
            charlotte_lifecycle::classify_join(captured, Some(current))
                == charlotte_lifecycle::JoinDisposition::ObserveCurrent
        });
        if generation_matches {
            thread
                .try_observe_exit(observer, charge)
                .map_err(|_| system_scheduler::Error::WaitRegistrationFailed)
        } else {
            drop(table);
            Err(system_scheduler::Error::InvalidThread)
        }
    } else {
        drop(table);
        Err(system_scheduler::Error::InvalidThread)
    }
}

/// Bind a worker's exit before publishing or admitting it. No numeric TID
/// lookup or post-spawn registration can race the worker's first instruction.
/// Kernel stack/thread-table construction retains the ordinary spawn API's
/// allocator contract; callback/list/entry admission is fallible beforehand.
pub(crate) fn spawn_worker_after_observe(
    entry_point: extern "C" fn(),
    observer: Weak<dyn Observer>,
    charge: crate::completion::watch_budget::Charge,
    install: impl FnOnce(
        threads::exit_source::ExitRegistration,
    ) -> Result<(), crate::completion::SubmitError>,
) -> Result<ThreadId, crate::completion::SubmitError> {
    let thread = Thread::new(crate::memory::KERNEL_ASID, entry_point);
    let registration = thread
        .try_observe_exit(observer, charge)
        .map_err(|_| crate::completion::SubmitError::WouldBlock)?;
    install(registration)?;
    // Kernel publication is not subject to user-domain retirement/thread limits.
    let tid = publish_thread(thread).expect("kernel worker publication failed");
    SYSTEM_SCHEDULER.read().submit_new_thread(tid).expect("kernel worker admission failed");
    Ok(tid)
}
