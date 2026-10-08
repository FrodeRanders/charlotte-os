//! Allocation-free whole-domain abort with exact root and thread identities.

use super::{
    Error,
    MASTER_THREAD_TABLE,
    SYSTEM_SCHEDULER,
    THREAD_PUBLICATION_GATE,
    ThreadGeneration,
    ThreadId,
};
use crate::memory::{
    ADDRESS_SPACE_TABLE,
    AddressSpaceHandle,
    operation::AddressSpaceOperation,
};

/// Abandonment retains both the root lease and its terminal admission fence.
/// The inline fence is never cleared; only a fresh root begins unfenced.
struct DomainAbortSweep {
    root: AddressSpaceOperation,
    ceiling: usize,
}

impl DomainAbortSweep {
    fn begin(handle: AddressSpaceHandle) -> Result<Self, Error> {
        // Lifecycle admission precedes scheduler/publication serialization.
        let root = AddressSpaceOperation::acquire(handle).map_err(|_| Error::ThreadTerminated)?;
        {
            let _gate = THREAD_PUBLICATION_GATE.lock();
            let mut table = ADDRESS_SPACE_TABLE.lock();
            // The admitted lease excludes root/slot replacement. Still qualify
            // exact identity before changing this root's admission state.
            assert_eq!(table.generation(handle.id()).ok(), Some(handle.generation()));
            table.get_mut(handle.id()).unwrap().thread_admission_closed = true;
        }
        let ceiling = MASTER_THREAD_TABLE.read().iter().len();
        Ok(Self {
            root,
            ceiling,
        })
    }

    fn run(self, mut before_abort: impl FnMut(ThreadId, ThreadGeneration)) -> Result<(), Error> {
        let handle = self.root.handle();
        let outcome = (|| {
            for tid in 0..self.ceiling {
                let generation = {
                    let table = MASTER_THREAD_TABLE.read();
                    match table.get(tid) {
                        Ok(thread) if thread.address_space == Some(handle) => thread.generation,
                        _ => continue,
                    }
                };
                // No publication/thread-table or lifecycle guard survives
                // into the scheduler claim. A reused TID must reject using
                // the captured generation, regardless of its new owner.
                before_abort(tid, generation);
                match SYSTEM_SCHEDULER.read().abort_thread_generation(tid, generation) {
                    Ok(_) | Err(Error::InvalidThread) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })();
        // This releases only the sweep's lease, not the terminal fence. Pending
        // thread contexts retain their own stack/root owners until reaped.
        self.root.release().map_err(|_| Error::ThreadRetirementFailed)?;
        outcome
    }
}

pub(crate) fn abort_domain_threads(handle: AddressSpaceHandle) -> Result<(), Error> {
    abort_domain_threads_with_request(handle, |_| Ok(()))
}

/// Publish a force request only after exact root admission/fencing, and keep
/// that root leased through publication and the complete abort sweep. The
/// request callback runs after publication/lifecycle/table guards leave.
pub(crate) fn abort_domain_threads_with_request(
    handle: AddressSpaceHandle,
    publish_request: impl FnOnce(&AddressSpaceOperation) -> Result<(), Error>,
) -> Result<(), Error> {
    let sweep = DomainAbortSweep::begin(handle)?;
    if let Err(error) = publish_request(&sweep.root) {
        // Ordinary publication rejection releases the lease, retaining the
        // terminal thread-admission fence. Panic retains both.
        sweep.root.release().map_err(|_| Error::ThreadRetirementFailed)?;
        return Err(error);
    }
    sweep.run(|_, _| {})
}

pub(crate) mod tests;
