//! A kernel thread prepared and table-published, but never made runnable.
//! Ordinary cancellation leaves local publication guards before stack release;
//! abandonment retains the exact table payload and its original charges.

use super::{
    system_scheduler::{
        Error,
        SYSTEM_SCHEDULER,
        publish_thread,
    },
    threads::{
        MASTER_THREAD_TABLE,
        Thread,
        ThreadGeneration,
        ThreadId,
        ThreadState,
    },
};
use crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask;

#[must_use]
#[derive(Debug)]
pub(crate) struct PreparedKernelThread {
    tid: ThreadId,
    generation: ThreadGeneration,
}

// Both variants keep the complete outstanding ownership; neither supplies
// permission to replay stack cleanup after an uncertain release.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum CancellationError {
    Published(PreparedKernelThread),
    Detached(Thread),
}

impl CancellationError {
    pub(crate) fn retain(self) {
        match self {
            Self::Published(owner) => drop(owner),
            Self::Detached(thread) => drop(thread),
        }
    }
}

impl PreparedKernelThread {
    #[cfg(feature = "shutdown_test")]
    pub(crate) fn identity(&self) -> (ThreadId, ThreadGeneration) {
        (self.tid, self.generation)
    }

    pub(crate) fn try_new(entry: extern "C" fn()) -> Result<Self, Error> {
        let thread = Thread::try_new(crate::memory::KERNEL_ASID, entry)?;
        let generation = thread.generation;
        let tid = publish_thread(thread)?;
        Ok(Self {
            tid,
            generation,
        })
    }

    /// The caller retains this mask through publication of the worker's input.
    /// Admission pins to the current LP, so first dispatch cannot precede that
    /// publication. No allocation, IPI, callback or wait disposal occurs here.
    pub(crate) fn start_on_current_lp(self, _mask: &LocalInterruptMask) -> Result<(), Self> {
        let result =
            SYSTEM_SCHEDULER.read().submit_prepared_kernel_thread(self.tid, self.generation);
        match result {
            Ok(()) => Ok(()),
            Err(_) => Err(self),
        }
    }

    /// Consume a never-admitted preparation after every enclosing guard leaves.
    /// Rejected extraction retains the exact published owner; rejected physical
    /// cleanup returns the complete detached thread, without retry.
    #[allow(clippy::result_large_err)]
    pub(crate) fn cancel(self) -> Result<(), CancellationError> {
        let thread = {
            let mut table = MASTER_THREAD_TABLE.write();
            if !table.get(self.tid).is_ok_and(|thread| {
                thread.generation == self.generation
                    && thread.dispatch_count == 0
                    && matches!(thread.state, ThreadState::NeedsLpAssignment)
                    && !thread.run_queue_link.is_linked()
                    && !thread.context.is_on_cpu()
            }) {
                return Err(CancellationError::Published(self));
            }
            match table.take_element(self.tid) {
                Ok(thread) => thread,
                Err(_) => return Err(CancellationError::Published(self)),
            }
        };
        thread.release_unstarted().map_err(CancellationError::Detached)
    }
}

impl Drop for PreparedKernelThread {
    fn drop(&mut self) {
        // The table still owns any unstarted payload. A lost receipt neither
        // removes it nor attempts physical cleanup under an unknown guard.
    }
}
