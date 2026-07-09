//! # Inter-Processor Interrupts (IPIs)
//!
//! The Catten IPI protocol is designed to work using remote procedure calls (RPCs).
//! This allows for a flexible and extensible way to send IPIs between processors.
//! The protocol uses unicast IPIs exculusively to avoid having to overcomplicate the
//! implementation which is also kept as architecture indepent as possible.

use alloc::collections::vec_deque::VecDeque;
use alloc::format;
use alloc::vec::Vec;

use concurrent_queue::ConcurrentQueue;

use crate::cpu::isa::constants::interrupt_vectors::ASYNC_IPI_VECTOR;
use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interrupts::LocalIntCtlr;
use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::isa::memory::tlb;
use crate::cpu::multiprocessor::get_lp_count;
use crate::cpu::multiprocessor::spin::mutex::Mutex;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::memory::linear::VAddr;
use crate::memory::{AddressSpaceId, KERNEL_ASID};

pub static IPI_CMD_QUEUES: spin::LazyLock<IpiCmdQueues> = spin::LazyLock::new(IpiCmdQueues::new);

#[inline(always)]
pub fn send_ipi(target_lp: LpId) {
    LocalIntCtlr::send_unicast_ipi(target_lp, ASYNC_IPI_VECTOR)
        .expect(&format!("Failed to send an IPI from LP {} to LP {target_lp}", get_lp_id()));
}

#[derive(Debug)]
pub struct IpiCmdQueues {
    queues: Vec<ConcurrentQueue<IpiRpc>>,
}

impl IpiCmdQueues {
    pub fn new() -> Self {
        Self {
            queues: (0..get_lp_count()).map(|_| ConcurrentQueue::unbounded()).collect::<Vec<_>>(),
        }
    }

    pub fn push_to(&self, target_lp: usize, ipi: IpiRpc) {
        self.queues[target_lp].push(ipi).expect("Failed to push IPI command to target LP");
    }

    pub fn pop_local(&self, lp_id: usize) -> Option<IpiRpc> {
        self.queues[lp_id].pop().ok()
    }
}

#[derive(Debug)]
pub enum IpiRpc {
    VMemInval(AddressSpaceId, VAddr, usize),
    AsidInval(AddressSpaceId),
    Wakeup,
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_interprocessor_interrupt(ipi_queue: &'static mut Mutex<VecDeque<IpiRpc>>) {
    while let Some(ipi) = ipi_queue.lock().pop_front() {
        match ipi {
            IpiRpc::VMemInval(asid, base, size) => {
                if asid == KERNEL_ASID {
                    tlb::inval_range_kernel(base, size);
                } else {
                    tlb::inval_range_user(asid, base, size);
                }
            }
            IpiRpc::AsidInval(asid) => tlb::inval_asid(asid),
            IpiRpc::Wakeup => {
                SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().set_ctx_switch_pending();
            }
        }
    }
}

/// Drain and execute all IPI RPCs queued for the calling logical processor.
///
/// This is the architecture-independent handler invoked from an interrupt
/// controller's IPI dispatch path (e.g. the AArch64 GIC IRQ dispatcher). Each
/// queued RPC is executed in order; TLB maintenance RPCs are honoured locally
/// (on AArch64 the invalidation itself is broadcast in hardware) and a wakeup
/// RPC marks a context switch pending so the dispatcher's yield takes effect.
pub fn drain_local_ipi_queue() {
    let lp_id = get_lp_id() as usize;
    while let Some(ipi) = IPI_CMD_QUEUES.pop_local(lp_id) {
        match ipi {
            IpiRpc::VMemInval(asid, base, size) => {
                if asid == KERNEL_ASID {
                    tlb::inval_range_kernel(base, size);
                } else {
                    tlb::inval_range_user(asid, base, size);
                }
            }
            IpiRpc::AsidInval(asid) => tlb::inval_asid(asid),
            IpiRpc::Wakeup => {
                SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().set_ctx_switch_pending();
            }
        }
    }
}
