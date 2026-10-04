use alloc::sync::Weak;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use lock_api::{
    GuardNoSend,
    RawMutex,
};

use crate::{
    cpu::scheduler::system_scheduler::{
        SYSTEM_SCHEDULER,
        get_thread_id,
    },
    klib::observer::{
        Observable,
        Observer,
        WaitRegistration,
        WaitSponsor,
        registration::RegistrationError,
        waiter_source::WaiterSource,
    },
};

pub type Mutex<T> = lock_api::Mutex<MutexCore, T>;

#[derive(Debug)]
pub struct MutexCore {
    raw_lock: AtomicBool,
    waitlist: WaiterSource,
}

impl Default for MutexCore {
    fn default() -> Self {
        Self::new()
    }
}

impl MutexCore {
    pub const fn new() -> Self {
        MutexCore {
            raw_lock: AtomicBool::new(false),
            waitlist: WaiterSource::new(),
        }
    }

    /// Count linked entries, not live/retained detached notification batches.
    pub(super) fn waiter_count(&self) -> usize {
        self.waitlist.registered()
    }
}

impl Observable for MutexCore {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        if !self.raw_lock.load(Ordering::Acquire) {
            return Ok(WaitRegistration::ready());
        }
        self.waitlist.register(observer, sponsor)
    }
}

unsafe impl RawMutex for MutexCore {
    type GuardMarker = GuardNoSend;

    const INIT: Self = Self::new();

    fn lock(&self) {
        loop {
            if self
                .raw_lock
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return; // acquired
            }
            let Some(tid) = get_thread_id() else {
                panic!("Attempted to acquire a blocking mutex from outside thread context.");
            };
            // Unlock can precede list insertion. Keep park + lost-wake recheck
            // non-preemptible so a quantum cannot strand a newly Blocked caller
            // before it sees that no future unlock is coming.
            let setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
            let generation = match SYSTEM_SCHEDULER.read().block_thread_with_constraint_generation(
                tid,
                self,
                crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
            ) {
                Ok(generation) => Some(generation),
                Err(error) => {
                    if matches!(
                        error,
                        crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed
                    ) {
                        super::LOCK_WAIT_ADMISSION_RETRIES[0].fetch_add(1, Ordering::Relaxed);
                    }
                    None
                }
            };
            // Lost-wake guard: unlock may have run between the failed CAS and
            // observer registration, in which case no future unlock is coming.
            if let Some(generation) = generation
                && !self.raw_lock.load(Ordering::Acquire)
            {
                let _ = SYSTEM_SCHEDULER.read().submit_woken_thread(tid, generation);
            }
            drop(setup);
            crate::cpu::scheduler::yield_lp();
        }
    }

    fn is_locked(&self) -> bool {
        self.raw_lock.load(Ordering::Acquire)
    }

    fn try_lock(&self) -> bool {
        self.waitlist.registered() == 0
            && self
                .raw_lock
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    unsafe fn unlock(&self) {
        self.raw_lock.store(false, Ordering::Release);
        // Notification is a hint, not an ownership handoff. Wake all candidates
        // after release so an expired/cancelled first candidate cannot strand
        // another waiter. CAS still chooses the next owner.
        self.waitlist.drain().notify();
    }
}
