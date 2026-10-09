//! Allocation-free whole-domain abort with exact root and thread identities.

use core::sync::atomic::Ordering;

use super::{
    Error,
    MASTER_THREAD_TABLE,
    SYSTEM_SCHEDULER,
    THREAD_PUBLICATION_GATE,
    ThreadGeneration,
    ThreadId,
};
use crate::{
    cpu::multiprocessor::interrupt_tracking::LocalInterruptMask,
    memory::{
        ADDRESS_SPACE_TABLE,
        AddressSpaceHandle,
        operation::AddressSpaceOperation,
    },
};

/// Owns the executing lifetime before root admission. Drop deliberately leaves
/// the inline fence installed: a discarded transaction cannot authorize stack
/// retirement or migration. This is scoped to abort-sweep execution, not a
/// general kernel cancellation or custody API.
struct AbortExecutor {
    identity: Option<(ThreadId, ThreadGeneration)>,
}

impl AbortExecutor {
    fn acquire() -> Result<Self, Error> {
        let _mask = LocalInterruptMask::new();
        let scheduler = SYSTEM_SCHEDULER.read();
        let local = scheduler.get_lp_scheduler().lock();
        let identity = local.get_current_handle();
        if let Some((tid, generation)) = identity {
            let mut table = MASTER_THREAD_TABLE.write();
            let thread = table.get_mut(tid).map_err(|_| Error::InvalidThread)?;
            if thread.generation != generation || thread.abort_requested.load(Ordering::Acquire) {
                return Err(Error::ThreadTerminated);
            }
            if thread.abort_executor_lp.load(Ordering::Acquire) != usize::MAX {
                return Err(Error::ThreadRetirementFailed);
            }
            thread
                .abort_executor_lp
                .store(crate::cpu::isa::lp::ops::get_lp_id() as usize, Ordering::Release);
        }
        Ok(Self {
            identity,
        })
    }

    fn release(self, _mask: &LocalInterruptMask) {
        if let Some((tid, generation)) = self.identity {
            let scheduler = SYSTEM_SCHEDULER.read();
            let local = scheduler.get_lp_scheduler().lock();
            assert_eq!(local.get_current_handle(), Some((tid, generation)));
            let mut table = MASTER_THREAD_TABLE.write();
            let thread = table.get_mut(tid).expect("retained abort executor missing");
            assert_eq!(thread.generation, generation);
            assert_eq!(
                thread.abort_executor_lp.load(Ordering::Acquire),
                crate::cpu::isa::lp::ops::get_lp_id() as usize
            );
            thread.abort_executor_lp.store(usize::MAX, Ordering::Release);
            if thread.abort_requested.load(Ordering::Acquire) {
                local.set_ctx_switch_pending();
            }
        }
    }
}

/// Abandonment retains both the root lease and its terminal admission fence.
/// The inline fence is never cleared; only a fresh root begins unfenced.
struct DomainAbortSweep {
    root: AddressSpaceOperation,
    executor: AbortExecutor,
    ceiling: usize,
}

impl DomainAbortSweep {
    fn begin(handle: AddressSpaceHandle) -> Result<Self, Error> {
        // Lifecycle admission precedes scheduler/publication serialization.
        let executor = AbortExecutor::acquire()?;
        let root = match AddressSpaceOperation::acquire(handle) {
            Ok(root) => root,
            Err(_) => {
                let mask = LocalInterruptMask::new();
                executor.release(&mask);
                return Err(Error::ThreadTerminated);
            }
        };
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
            executor,
            ceiling,
        })
    }

    fn run(self, mut before_abort: impl FnMut(ThreadId, ThreadGeneration)) -> Result<(), Error> {
        let handle = self.root.handle();
        let caller = self.executor.identity;
        tests::before_concurrent_sweep(handle, caller);
        let mut deferred_caller = None;
        let outcome = (|| {
            for tid in 0..self.ceiling {
                let generation = {
                    let table = MASTER_THREAD_TABLE.read();
                    match table.get(tid) {
                        Ok(thread) if thread.address_space == Some(handle) => thread.generation,
                        _ => continue,
                    }
                };
                if caller == Some((tid, generation)) {
                    // Requesting self-abort here could retire this stack with
                    // the sweep's root operation still held. Finish peers first.
                    deferred_caller = caller;
                    continue;
                }
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
        tests::before_self_handoff(handle, deferred_caller);
        tests::before_concurrent_completion(handle, deferred_caller);
        // Only the final executing-thread request and scalar lease completion
        // are non-preemptible. No sweep, callback, physical cleanup or yield is
        // allowed in this interval. Preserve any enclosing IRQ mask.
        let handoff = LocalInterruptMask::new();
        let outcome = outcome.and_then(|()| {
            if let Some((tid, generation)) = deferred_caller {
                SYSTEM_SCHEDULER
                    .read()
                    .request_executing_abort(tid, generation, handle, &handoff)?;
            }
            Ok(())
        });
        // This releases only the sweep's lease, not the terminal fence. Pending
        // thread contexts retain their own stack/root owners until reaped.
        self.root.release().map_err(|_| Error::ThreadRetirementFailed)?;
        self.executor.release(&handoff);
        tests::after_self_handoff(handle, deferred_caller, outcome.is_ok());
        tests::after_concurrent_completion(handle, deferred_caller, outcome.is_ok());
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
        let mask = LocalInterruptMask::new();
        sweep.root.release().map_err(|_| Error::ThreadRetirementFailed)?;
        sweep.executor.release(&mask);
        return Err(error);
    }
    sweep.run(|_, _| {})
}

pub(crate) mod tests;
