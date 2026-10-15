//! Exact executing-thread retention shared by abort sweeps and stack growth.

use core::sync::atomic::Ordering;

use super::{
    MASTER_THREAD_TABLE,
    ThreadGeneration,
    ThreadId,
};
use crate::cpu::{
    multiprocessor::interrupt_tracking::LocalInterruptMask,
    scheduler::system_scheduler::{
        Error,
        SYSTEM_SCHEDULER,
    },
};

/// Owns the executing lifetime before root admission. Drop deliberately leaves
/// the inline fence installed: a discarded transaction cannot authorize stack
/// retirement or migration. The containing operation must retain its complete
/// root/backing/authority owners; this fence is not a physical cleanup receipt.
pub(crate) struct ThreadExecution {
    identity: Option<(ThreadId, ThreadGeneration)>,
}

impl ThreadExecution {
    pub(crate) fn identity(&self) -> Option<(ThreadId, ThreadGeneration)> {
        self.identity
    }

    pub(crate) fn acquire() -> Result<Self, Error> {
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
            if thread.operation_executor_lp.load(Ordering::Acquire) != usize::MAX {
                return Err(Error::ThreadRetirementFailed);
            }
            thread
                .operation_executor_lp
                .store(crate::cpu::isa::lp::ops::get_lp_id() as usize, Ordering::Release);
        }
        Ok(Self {
            identity,
        })
    }

    pub(crate) fn release(self, _mask: &LocalInterruptMask) {
        if let Some((tid, generation)) = self.identity {
            let scheduler = SYSTEM_SCHEDULER.read();
            let local = scheduler.get_lp_scheduler().lock();
            assert_eq!(local.get_current_handle(), Some((tid, generation)));
            let mut table = MASTER_THREAD_TABLE.write();
            let thread = table.get_mut(tid).expect("retained thread execution missing");
            assert_eq!(thread.generation, generation);
            assert_eq!(
                thread.operation_executor_lp.load(Ordering::Acquire),
                crate::cpu::isa::lp::ops::get_lp_id() as usize
            );
            thread.operation_executor_lp.store(usize::MAX, Ordering::Release);
            if thread.abort_requested.load(Ordering::Acquire) {
                local.set_ctx_switch_pending();
            }
        }
    }
}
