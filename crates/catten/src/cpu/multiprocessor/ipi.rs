//! # Inter-Processor Interrupts (IPIs)
//!
//! The Catten IPI protocol uses unicast IPIs exclusively. Each LP has a
//! **bounded** command queue (`IPI_CMD_QUEUES`) — when a target LP's inbox is
//! full, the sender gets the RPC back (backpressure) instead of the queue
//! growing without limit. This matches sitas's bounded `ShardSender<M>`
//! semantics and is the kernel side of the cross-shard backpressure contract
//! specified in `docs/async-syscall-abi.md` §6.
//!
//! The [`IpiRpc`] enum carries the kernel's own TLB shootdown and wakeup
//! commands plus a [`Closure`](IpiRpc::Closure) variant that packages arbitrary
//! `FnOnce() + Send` work for cross-LP execution — the seed of a typed
//! `ShardMailbox<M>` (Option B / Phase 3).

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

/// Default per-LP IPI queue capacity. `push` on a full bounded queue returns
/// `Err(Full(rpc))` — first-class, non-fatal backpressure.
const DEFAULT_QUEUE_CAPACITY: usize = 256;

pub static IPI_CMD_QUEUES: spin::LazyLock<IpiCmdQueues> =
    spin::LazyLock::new(|| IpiCmdQueues::new(DEFAULT_QUEUE_CAPACITY));

#[inline(always)]
pub fn send_ipi(target_lp: LpId) {
    LocalIntCtlr::send_unicast_ipi(target_lp, ASYNC_IPI_VECTOR)
        .expect(&format!("Failed to send an IPI from LP {} to LP {target_lp}", get_lp_id()));
}

pub struct IpiCmdQueues {
    queues: Vec<ConcurrentQueue<IpiRpc>>,
}

impl core::fmt::Debug for IpiCmdQueues {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.queues.iter().map(|q| q.len())).finish()
    }
}

impl IpiCmdQueues {
    pub fn new(capacity: usize) -> Self {
        Self {
            queues: (0..get_lp_count())
                .map(|_| ConcurrentQueue::bounded(capacity))
                .collect::<Vec<_>>(),
        }
    }

    /// Push with backpressure: returns `Err(rpc)` if the target queue is full.
    pub fn try_push_to(&self, target_lp: usize, ipi: IpiRpc) -> Result<(), IpiRpc> {
        self.queues[target_lp]
            .push(ipi)
            .map_err(|e| e.into_inner())
    }

    /// Force-push for kernel-internal must-not-drop RPCs (TLB shootdown, wakeup).
    /// If the queue is full the oldest entry is evicted. The capacity is sized
    /// generously so this path should virtually never be entered.
    pub fn push_to(&self, target_lp: usize, ipi: IpiRpc) {
        if let Err(e) = self.queues[target_lp].push(ipi) {
            let _evicted = self.queues[target_lp].force_push(e.into_inner());
        }
    }

    pub fn pop_local(&self, lp_id: usize) -> Option<IpiRpc> {
        self.queues[lp_id].pop().ok()
    }

    pub fn queue_len(&self, lp_id: usize) -> usize {
        self.queues[lp_id].len()
    }
}

pub enum IpiRpc {
    VMemInval(AddressSpaceId, VAddr, usize),
    AsidInval(AddressSpaceId),
    Wakeup,
    /// A closure executed on the target LP. The sender boxes arbitrary work;
    /// the allocation happens on the sending LP, not in the IRQ handler.
    #[allow(clippy::type_complexity)]
    Closure(alloc::boxed::Box<dyn FnOnce() + Send>),
}

// Manual Debug because `FnOnce` is not `Debug`.
impl core::fmt::Debug for IpiRpc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::VMemInval(a, v, s) => f
                .debug_tuple("VMemInval")
                .field(a)
                .field(v)
                .field(s)
                .finish(),
            Self::AsidInval(a) => f.debug_tuple("AsidInval").field(a).finish(),
            Self::Wakeup => write!(f, "Wakeup"),
            Self::Closure(_) => write!(f, "Closure(opaque)"),
        }
    }
}

/// Enqueue an RPC on a target LP's bounded queue and deliver the IPI. Returns
/// the RPC back (`Err(rpc)`) if the target's queue is full — first-class,
/// non-fatal backpressure (the kernel-side analog of submit-WouldBlock).
pub fn try_send_ipi_rpc(target_lp: LpId, rpc: IpiRpc) -> Result<(), IpiRpc> {
    let lp = target_lp as usize;
    IPI_CMD_QUEUES.try_push_to(lp, rpc)?;
    send_ipi(target_lp);
    Ok(())
}

/// Non-fallible send for kernel-internal RPCs (TLB shootdown, wakeup). These
/// must not be dropped; the queue force-pushes if full.
pub fn send_ipi_rpc(target_lp: LpId, rpc: IpiRpc) {
    let lp = target_lp as usize;
    IPI_CMD_QUEUES.push_to(lp, rpc);
    send_ipi(target_lp);
}

/// Enqueue an arbitrary closure for execution on the target LP and deliver the
/// IPI. Returns `Ok(())` on success or `Err(f)` with the original closure if
/// the target's bounded queue is full (backpressure).
pub fn try_run_on_lp<F: FnOnce() + Send + 'static>(
    target_lp: LpId,
    f: F,
) -> Result<(), F> {
    // Pack the closure into a Box whose layout is known: first the vtable
    // pointer, then the data pointer. After we recover the IpiRpc::Closure box
    // on the backpressure path we transmute it back to the concrete type via
    // the pointer, because `Box<dyn FnOnce>` does not support `downcast`.
    //
    // Safety: we created the Box from an `F` and only transmute it back when
    // the outer function owns it again (the closure was never invoked by the
    // kernel). The type `F` is known to the caller, and the layout of
    // `Box<F>` and `Box<dyn FnOnce() + Send>` share the same representation
    // (fat pointer: (data, vtable)).
    let rpc = IpiRpc::Closure(alloc::boxed::Box::new(f) as alloc::boxed::Box<dyn FnOnce() + Send>);
    match try_send_ipi_rpc(target_lp, rpc) {
        Ok(()) => Ok(()),
        Err(IpiRpc::Closure(b)) => {
            // Recover the original F from the fat-pointer Box via unsafe
            // pointer cast. The Box was never invoked, so the concrete type is
            // still `F`.
            let recovered: alloc::boxed::Box<F> = unsafe {
                alloc::boxed::Box::from_raw(
                    alloc::boxed::Box::into_raw(b) as *mut F
                )
            };
            Err(*recovered)
        }
        Err(_) => unreachable!(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_interprocessor_interrupt(ipi_queue: &'static mut Mutex<VecDeque<IpiRpc>>) {
    while let Some(ipi) = ipi_queue.lock().pop_front() {
        dispatch_ipi_rpc(ipi);
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
        dispatch_ipi_rpc(ipi);
    }
}

fn dispatch_ipi_rpc(ipi: IpiRpc) {
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
            SYSTEM_SCHEDULER
                .read()
                .get_lp_scheduler()
                .lock()
                .set_ctx_switch_pending();
        }
        IpiRpc::Closure(f) => {
            f();
        }
    }
}
