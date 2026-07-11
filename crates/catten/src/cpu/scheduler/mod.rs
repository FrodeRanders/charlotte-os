use alloc::sync::Weak;
use core::hint::unreachable_unchecked;

use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, Thread, ThreadId};
use crate::klib::observer::{Observable as _, Observer};
use crate::klib::time::duration::ExtDuration;
use crate::logln;
use crate::memory::AddressSpaceId;
use crate::timers::{TIMER_QUEUES, TimerEvent};

pub mod lp_schedulers;
pub mod sync;
pub mod system_scheduler;
pub mod threads;

/// Creates a new thread and submit it to the system scheduler for assignment to a logical processor
/// and then execution.
pub fn spawn_thread(asid: AddressSpaceId, entry_point: extern "C" fn()) -> ThreadId {
    let thread = Thread::new(asid, entry_point);
    let tid = MASTER_THREAD_TABLE.write().add_element(thread);
    SYSTEM_SCHEDULER
        .read()
        .submit_ready_thread(tid as ThreadId)
        .expect("Error submitting ready thread to system scheduler");
    tid
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
            .write()
            .block_thread(tid, &mut timer_event)
            .expect("Error putting thread to sleep");
        TIMER_QUEUES.try_get_mut().unwrap().add_event(timer_event);
        // Yield so the sleep takes effect: `block_thread` marks the thread
        // Blocked and registers its waker on the timer event; this yield saves
        // the thread's context and switches away. When the timer expires it
        // fires the waker, re-admitting the thread, which resumes here.
        yield_lp();
    }
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
