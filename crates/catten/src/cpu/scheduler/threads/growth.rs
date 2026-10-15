//! Retain the exact executing thread while stack growth leaves serialization.

use super::{
    MASTER_THREAD_TABLE,
    ThreadExecution,
    ThreadGeneration,
    ThreadId,
};
use crate::{
    cpu::multiprocessor::interrupt_tracking::LocalInterruptMask,
    memory::{
        AddressSpaceId,
        KERNEL_ASID,
        thread_stack::Growth,
    },
};

pub(crate) mod tests;

/// The complete original thread/context/stack/root/charge remains in the
/// master table behind `execution`. Only this operation owns new page progress
/// and provisional backing. Abandonment retains both fences and all fields;
/// neither an address nor a plan is a standalone retry/cleanup owner.
struct CurrentGrowth {
    execution: Option<ThreadExecution>,
    identity: (ThreadId, ThreadGeneration),
    growth: Option<Growth>,
}

impl CurrentGrowth {
    fn claim(asid: AddressSpaceId, address: usize) -> Option<Self> {
        if asid == KERNEL_ASID {
            return None;
        }
        let execution = ThreadExecution::acquire().ok()?;
        let captured = execution.identity().and_then(|(tid, generation)| {
            let mut table = MASTER_THREAD_TABLE.write();
            let thread = table.get_mut(tid).ok()?;
            if thread.generation != generation || thread.asid != asid {
                return None;
            }
            let growth = thread.context.begin_stack_growth(address)?;
            assert_eq!(thread.address_space, Some(growth.handle()));
            Some(((tid, generation), growth))
        });
        match captured {
            Some((identity, growth)) => Some(Self {
                execution: Some(execution),
                identity,
                growth: Some(growth),
            }),
            None => {
                let mask = LocalInterruptMask::new();
                execution.release(&mask);
                None
            }
        }
    }

    fn run(mut self) -> Option<usize> {
        tests::before_growth(self.growth.as_ref().unwrap().handle(), self.identity);
        self.growth.as_mut().unwrap().run();
        self.finish()
    }

    fn finish(mut self) -> Option<usize> {
        // Only owner handback and scalar fence completion are masked. No
        // allocator, mapper, physical release or callback runs in this scope.
        let mask = LocalInterruptMask::new();
        let growth = self.growth.take().unwrap();
        let handle = growth.handle();
        let (tid, generation) = self.identity;
        let result = {
            let mut table = MASTER_THREAD_TABLE.write();
            match table.get_mut(tid) {
                Ok(thread)
                    if thread.generation == generation
                        && thread.address_space == Some(growth.handle()) =>
                {
                    thread.context.finish_stack_growth(growth)
                }
                _ => Err(growth),
            }
        };
        match result {
            Ok(low) => {
                tests::before_release(handle, self.identity, low);
                self.execution.take().unwrap().release(&mask);
                low
            }
            Err(growth) => {
                self.growth = Some(growth);
                None // Retain the execution and original slot fences; no retry.
            }
        }
    }
}

/// Synchronous EL0 fault entry preserves the enclosing IRQ policy. Capture
/// and handback hold short table guards; allocation, mapping and rollback do
/// not. Preemption/remote abort cannot extract or migrate this exact owner.
pub(crate) fn grow_current_user_stack(asid: AddressSpaceId, address: usize) -> Option<usize> {
    CurrentGrowth::claim(asid, address)?.run()
}
