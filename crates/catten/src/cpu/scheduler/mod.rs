use alloc::sync::Weak;

use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::{MASTER_THREAD_TABLE, Thread, ThreadId};
use crate::klib::observer::{Observable as _, Observer};
use crate::memory::AddressSpaceId;

pub mod lp_schedulers;
pub mod sync;
pub mod system_scheduler;
pub mod threads;

pub fn spawn_thread(asid: AddressSpaceId, entry_point: extern "C" fn()) -> ThreadId {
    let thread = Thread::new(asid, entry_point);
    let tid = MASTER_THREAD_TABLE.write().add_element(thread);
    SYSTEM_SCHEDULER
        .read()
        .submit_ready_thread(tid as ThreadId)
        .expect("Error submitting ready thread to system scheduler");
    tid
}

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
