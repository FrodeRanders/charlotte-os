use alloc::sync::Weak;
use core::hint::unreachable_unchecked;

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::{
            MASTER_THREAD_TABLE,
            Thread,
            ThreadId,
        },
    },
    klib::{
        observer::{
            Observable as _,
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

/// Creates a new thread and submit it to the system scheduler for assignment to a logical processor
/// and then execution.
pub fn spawn_thread(asid: AddressSpaceId, entry_point: extern "C" fn()) -> ThreadId {
    spawn_thread_with_migration(asid, entry_point, false)
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
    let tid = MASTER_THREAD_TABLE.write().add_element(thread);
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
    let tid = MASTER_THREAD_TABLE.write().add_element(thread);
    SYSTEM_SCHEDULER
        .read()
        .submit_ready_thread(tid as ThreadId)
        .expect("Error submitting ready thread to system scheduler");
    tid
}

/// Returns the address-space id of the currently running thread, if execution
/// is currently inside scheduler-managed thread context.
pub fn current_thread_asid() -> Option<AddressSpaceId> {
    let tid = system_scheduler::get_thread_id()?;
    MASTER_THREAD_TABLE.read().get(tid).ok().map(|thread| thread.asid)
}

/// Unconditionally yields the current logical processor to the scheduler for a context switch.
///
/// This can safely be called from anywhere including outside of thread context. However if it is
/// called from interrupt context then it will cause an immediate context switch never to return
/// which will essentially cause the remainder of the ISR to get skipped. This is almost never what
/// is intended thus for interrupt service it is recommended instead to set the context switch
/// pending variable on the current LP's local scheduler and then have the switch happen at the end
/// of the ISR at which point all ISRs with the sole exception of double fault and other ISA
/// specific analogues call `cond_yield_lp` to carry out pending context switches.
pub fn yield_lp() {
    // Deliver any device-interrupt wakes queued from interrupt context
    // (architecture doc §10.2): the interrupt path is lock-free and defers the
    // actual `completion::wake` to thread context. Draining here — on every
    // cooperative yield across every LP — makes a driver blocked in `CQ_WAIT`
    // runnable promptly without the interrupt handler ever taking a lock.
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
/// This is the default way to exit a thread in the kernel since kernel threads should not carry any
/// state that is so complex that it requires exit handlers. For the userspace exit call this should
/// only be called after exit handlers have been run and any pending upcalls have been attempted to
/// be delivered. It is expected that exit handlers will be called from userspace itself via a given
/// program's runtime library, however upcalls are still solely the purview of the kernel and we
/// should at least attempt delivery prior to abort.
pub fn abort() -> ! {
    // Bind `tid` to a value so the temporary SYSTEM_SCHEDULER read guard and LP
    // scheduler lock in the scrutinee are released before the body runs;
    // otherwise `abort_thread` (which re-locks the LP scheduler) would deadlock.
    let tid = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid();
    if let Some(tid) = tid {
        logln!("Thread {} is aborting execution.", tid);
        SYSTEM_SCHEDULER.read().abort_thread(tid).expect("Error aborting thread");
    }
    yield_lp();
    unsafe { unreachable_unchecked() }
}

/// Blocks the current thread for at least the specified duration.
pub fn sleep(duration: ExtDuration) {
    let mut timer_event = TimerEvent::from(duration);
    // Bind `tid` first so the read guard + LP scheduler lock in the scrutinee
    // are released before `block_thread` (which takes SYSTEM_SCHEDULER.write());
    // holding the read guard across the write would deadlock the RwLock.
    let tid = SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().get_tid();
    if let Some(tid) = tid {
        SYSTEM_SCHEDULER
            .read()
            .block_thread(tid, &mut timer_event)
            .expect("Error putting thread to sleep");
        crate::timers::enqueue_event(timer_event);
        // Yield so the sleep takes effect: `block_thread` marks the thread
        // Blocked and registers its waker on the timer event; this yield saves
        // the thread's context and switches away. When the timer expires it
        // fires the waker, re-admitting the thread, which resumes here.
        yield_lp();
    }
}

pub fn sleep_millis(milliseconds: u64) {
    sleep(ExtDuration::from_millis(milliseconds as u128));
}

/// Registers an observer to be notified when the specified thread exits.
pub fn observe_thread_exit(
    thread_id: ThreadId,
    observer: Weak<dyn Observer>,
) -> Result<(), system_scheduler::Error> {
    if let Ok(thread) = MASTER_THREAD_TABLE.read().get(thread_id) {
        thread.register_observer(observer);
        Ok(())
    } else {
        Err(system_scheduler::Error::InvalidThread)
    }
}
