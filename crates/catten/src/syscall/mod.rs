//! # Syscall Dispatch Subsystem
//!
//! This module defines the AArch64 syscall dispatch table and the per-ISA
//! [`TrapFrame`] type that the [`sync_dispatcher`] (when handling an SVC from a
//! lower EL) passes into the dispatch function. It also contains the public
//! `syscall_dispatch` entry point that [`sync_dispatcher`] calls after decoding
//! the exception class.
//!
//! At this prototype stage the syscalls mirror the completion-capability ABI
//! operations in [`crate::completion`] and the experimental endpoint IPC in
//! [`crate::ipc`]. Completion queues remain the kernel/device-operation ABI;
//! endpoint IPC is the cross-address-space service ABI and starts with scalar
//! messages plus reply tokens.
//!
//! ## Syscall number convention
//!
//! The AArch64 `SVC #imm` instruction encodes a 16-bit immediate in
//! `ESR_EL1[15:0]` (the ISS field for SVC). This kernel uses that immediate as
//! the syscall number.

use alloc::collections::BTreeMap;

use crate::{
    cpu::isa::{
        interface::memory::AddressSpaceInterface,
        lp::{
            LpId,
            ops::get_lp_id,
        },
    },
    ipc,
    logln,
    memory::{
        AddressSpaceId,
        VAddr,
        object,
    },
};

/// A snapshot of the volatile register set and architectural state at the moment
/// a synchronous exception was taken from a lower EL on AArch64.
///
/// `regs[0]` = x0, …, `regs[18]` = x18 as saved by `push_volatile_regs` on the
/// kernel stack. The caller ([`sync_dispatcher`]) populates `elr_el1`, `spsr_el1`,
/// and `sp_el0` from the saved system registers.
#[derive(Debug)]
pub struct TrapFrame {
    pub regs: [u64; 19],
    pub elr_el1: u64,
    pub spsr_el1: u64,
    pub sp_el0: u64,
    pub lp_id: LpId,
    /// Address-space id of the calling thread, looked up from the scheduler at
    /// exception entry.  For the real SVC path this is the EL0 thread's AS;
    /// self-tests that call `syscall_dispatch` directly may set it manually.
    pub asid: AddressSpaceId,
}

/// Syscall numbers.
pub mod call_no {
    pub const LOG: u16 = 0;
    pub const COMPLETION_SUBMIT: u16 = 1;
    pub const COMPLETION_COMPLETE: u16 = 2;
    pub const COMPLETION_POLL: u16 = 3;
    pub const COMPLETION_WAIT: u16 = 4;
    pub const COMPLETION_CANCEL: u16 = 5;
    pub const COMPLETION_CLOSE: u16 = 6;
    pub const SPAWN_THREAD: u16 = 7;
    /// Terminate the calling EL0 thread.
    pub const THREAD_EXIT: u16 = 8;
    /// Send a 64-bit message to a specific LP's mailbox.
    pub const MAILBOX_SEND: u16 = 9;
    /// Receive a 64-bit message from the calling LP's mailbox.
    pub const MAILBOX_RECV: u16 = 10;
    /// Block on a completion with a timeout (milliseconds in x2).
    pub const COMPLETION_WAIT_TIMEOUT: u16 = 11;
    /// Block until CQ `x2` of the caller has at least `x1` pending entries or
    /// an explicit wake is posted to it.
    pub const CQ_WAIT: u16 = 12;
    /// Open a sender capability targeting LP `x1`. Returns cap in `x0`.
    pub const MAILBOX_OPEN_SEND: u16 = 13;
    /// Open a receiver capability for the caller's current LP. Returns cap in `x0`.
    pub const MAILBOX_OPEN_RECV: u16 = 14;
    /// Send `x2` through sender capability `x1`. Returns status in `x0`.
    pub const MAILBOX_SEND_CAP: u16 = 15;
    /// Receive through receiver capability `x1`. Returns msg in `x0`, status in `x1`.
    pub const MAILBOX_RECV_CAP: u16 = 16;
    /// Close mailbox capability `x1`. Returns status in `x0`.
    pub const MAILBOX_CLOSE: u16 = 17;
    /// Create an endpoint owned by the caller. x1=interface, x2=version,
    /// x3=queue capacity. Returns endpoint cap in x0, or 0 on failure.
    pub const IPC_ENDPOINT_CREATE: u16 = 18;
    /// Mint a same-address-space connection from endpoint cap x1 with rights x2.
    pub const IPC_CONNECT: u16 = 19;
    /// Send scalar message through connection x1. x2=opcode, x3=arg0.
    pub const IPC_SCALAR_SEND: u16 = 20;
    /// Call through connection x1. x2=opcode, x3=arg0. Returns pending-call cap.
    pub const IPC_SCALAR_CALL: u16 = 21;
    /// Receive from endpoint x1. Returns status in x0 and message fields in x1+.
    pub const IPC_RECV: u16 = 22;
    /// Reply via reply-token x1 with signed result x2.
    pub const IPC_REPLY: u16 = 23;
    /// Poll pending-call x1. Returns 0+result when ready, 1 when pending.
    pub const IPC_REPLY_POLL: u16 = 24;
    /// Close an endpoint IPC cap x1.
    pub const IPC_CLOSE: u16 = 25;
    /// Reply with a delegated connection cap. x1=reply cap, x2=endpoint cap,
    /// x3=connection rights. Returns status in x0.
    pub const IPC_REPLY_CONNECTION: u16 = 26;
    /// Block until endpoint x1 is readable, then receive one message. Returns
    /// the same register shape as IPC_RECV.
    pub const IPC_RECV_BLOCK: u16 = 27;
    /// Allocate a memory object owned by the caller. x1=pages. Returns cap in x0.
    pub const MEMORY_ALLOC: u16 = 28;
    /// Map memory object x1 at user VA x2. x3=1 writable, 0 read-only.
    pub const MEMORY_MAP: u16 = 29;
    /// Unmap memory object x1 from the caller.
    pub const MEMORY_UNMAP: u16 = 30;
    /// Close memory object cap x1.
    pub const MEMORY_CLOSE: u16 = 31;
    /// Send scalar message with moved memory cap x4.
    pub const IPC_SCALAR_SEND_MOVE: u16 = 32;
    /// Call with moved memory cap x4. Returns pending-call cap.
    pub const IPC_SCALAR_CALL_MOVE: u16 = 33;
    /// Reply and move memory cap x2 back to caller. x3=result.
    pub const IPC_REPLY_MOVE: u16 = 34;
    /// Call with read-borrowed memory cap x4.
    pub const IPC_SCALAR_CALL_BORROW_READ: u16 = 35;
    /// Call with writable borrowed memory cap x4.
    pub const IPC_SCALAR_CALL_BORROW_WRITE: u16 = 36;
    /// Send scalar message with copied memory cap x4.
    pub const IPC_SCALAR_SEND_COPY: u16 = 37;
    /// Call with copied memory cap x4. Returns pending-call cap.
    pub const IPC_SCALAR_CALL_COPY: u16 = 38;
    /// Call carrying a delegated connection. x1=connection, x2=opcode,
    /// x3=arg0, x4=mintable endpoint/connection cap, x5=delegated rights.
    /// Returns pending-call cap in x0. The receiver observes the minted
    /// connection cap in x8 of IPC_RECV/IPC_RECV_BLOCK.
    pub const IPC_SCALAR_CALL_CONNECTION: u16 = 39;
    /// Call carrying a delegated connection *and* a copied memory object.
    /// x1=connection, x2=opcode, x3=arg0, x4=mintable endpoint/connection
    /// cap, x5=delegated rights, x6=memory cap to copy. Returns pending-call
    /// cap in x0. The receiver observes the copied memory cap in x7 and the
    /// minted connection cap in x8 of IPC_RECV/IPC_RECV_BLOCK.
    pub const IPC_SCALAR_CALL_CONNECTION_COPY: u16 = 40;
    /// Post an explicit wake to CQ `x1`'s waiters (cross-shard reactor wake).
    /// Returns 0.
    pub const CQ_WAKE: u16 = 41;
    /// Block until CQ `x3` of the caller has at least `x1` entries, an
    /// explicit wake is posted to it, or `x2` milliseconds elapse. Returns the
    /// pending entry count in x0 and 1 in x1 if the deadline fired first, 0
    /// otherwise.
    pub const CQ_WAIT_TIMEOUT: u16 = 42;
    /// Bind endpoint `x1`'s readiness to the caller's CQ `x2`: the kernel
    /// posts a coalesced wake to that queue on the endpoint's
    /// empty-to-nonempty transition and on closure. Returns status in x0.
    pub const IPC_ENDPOINT_BIND_CQ: u16 = 43;
    /// Map MMIO region capability `x1` into the caller's address space at user
    /// virtual address `x2`; `x3`=1 writable, 0 read-only. Returns a device
    /// status code in x0.
    pub const DEVICE_MMIO_MAP: u16 = 44;
    /// Unmap MMIO region capability `x1` from the caller. Returns a device
    /// status code in x0.
    pub const DEVICE_MMIO_UNMAP: u16 = 45;
    /// Bind interrupt capability `x1` to the caller's CQ `x2` and arm the
    /// source. Delivered interrupts post a coalesced readiness wake to that
    /// queue. Returns a device status code in x0.
    pub const DEVICE_IRQ_BIND_CQ: u16 = 46;
    /// Acknowledge interrupt capability `x1`: clear its pending count and
    /// re-arm the source. Returns a device status code in x0 and the number
    /// of coalesced interrupts consumed in x1.
    pub const DEVICE_IRQ_ACK: u16 = 47;
    /// Close device capability `x1` (unmap an MMIO region or mask and unroute
    /// an interrupt). Returns a device status code in x0.
    pub const DEVICE_CLOSE: u16 = 48;
    /// Return the physical base address (PAddr) of the first frame of memory
    /// object `x1` in `x0`, or 0 on error. The caller must own the cap and
    /// the object must not be lent.
    pub const MEMORY_GET_PHYS: u16 = 49;
    /// Request the supervisor to spawn a replacement domain for a live
    /// upgrade.  x1 = name-service connection cap (caller's AS), x2 = ELF
    /// selector (0 = echo service ELF), x3 = state memory cap (0 if none),
    /// x4 = old endpoint cap (in the old service's cap table — the
    /// supervisor finds the owner ASID).  Returns the new generation in x0,
    /// or 0 on failure.
    pub const SPAWN_UPGRADE: u16 = 50;
    /// Send a vector of memory-object caps. x1=connection, x2=opcode,
    /// x3=arg0, x4=cap_vector_page. Returns an IPC status code in x0.
    pub const IPC_VECTOR_SEND: u16 = 51;
    /// Call carrying a vector of memory-object caps. x1=connection,
    /// x2=opcode, x3=arg0, x4=cap_vector_page. Returns pending-call cap
    /// in x0, or 0 on error.
    pub const IPC_VECTOR_CALL: u16 = 52;
    /// Receive a message and fill the caller's result page at x1 with
    /// cap IDs of delivered memory objects. Returns the same register
    /// shape as IPC_RECV, plus the result page contents.
    pub const IPC_RECV_VEC: u16 = 53;
}

/// The upper bound on the SVC immediate we will try to dispatch.
pub const MAX_SYSCALL: u16 = call_no::IPC_RECV_VEC;

/// Decode the exception class (EC) field from ESR_EL1 bits [31:26].
pub const fn ec_from_esr(esr: u64) -> u8 {
    ((esr >> 26) & 0x3f) as u8
}

/// Exception class for SVC from AArch64 state.
pub const EC_SVC_AARCH64: u8 = 0x15;

/// The single entry point from the ISA-specific [`sync_dispatcher`]. Panics on
/// an unknown syscall.
pub fn syscall_dispatch(frame: &mut TrapFrame, syscall_no: u16) {
    match syscall_no {
        call_no::LOG => sys_log(frame),
        call_no::COMPLETION_SUBMIT => sys_completion_submit(frame),
        call_no::COMPLETION_COMPLETE => sys_completion_complete(frame),
        call_no::COMPLETION_POLL => sys_completion_poll(frame),
        call_no::COMPLETION_WAIT => sys_completion_wait(frame),
        call_no::COMPLETION_CANCEL => sys_completion_cancel(frame),
        call_no::COMPLETION_CLOSE => sys_completion_close(frame),
        call_no::SPAWN_THREAD => sys_spawn_thread(frame),
        call_no::THREAD_EXIT => sys_thread_exit(frame),
        call_no::MAILBOX_SEND => sys_mailbox_send(frame),
        call_no::MAILBOX_RECV => sys_mailbox_recv(frame),
        call_no::COMPLETION_WAIT_TIMEOUT => sys_completion_wait_timeout(frame),
        call_no::CQ_WAIT => sys_cq_wait(frame),
        call_no::MAILBOX_OPEN_SEND => sys_mailbox_open_send(frame),
        call_no::MAILBOX_OPEN_RECV => sys_mailbox_open_recv(frame),
        call_no::MAILBOX_SEND_CAP => sys_mailbox_send_cap(frame),
        call_no::MAILBOX_RECV_CAP => sys_mailbox_recv_cap(frame),
        call_no::MAILBOX_CLOSE => sys_mailbox_close(frame),
        call_no::IPC_ENDPOINT_CREATE => sys_ipc_endpoint_create(frame),
        call_no::IPC_CONNECT => sys_ipc_connect(frame),
        call_no::IPC_SCALAR_SEND => sys_ipc_scalar_send(frame),
        call_no::IPC_SCALAR_CALL => sys_ipc_scalar_call(frame),
        call_no::IPC_RECV => sys_ipc_recv(frame),
        call_no::IPC_REPLY => sys_ipc_reply(frame),
        call_no::IPC_REPLY_POLL => sys_ipc_reply_poll(frame),
        call_no::IPC_CLOSE => sys_ipc_close(frame),
        call_no::IPC_REPLY_CONNECTION => sys_ipc_reply_connection(frame),
        call_no::IPC_RECV_BLOCK => sys_ipc_recv_block(frame),
        call_no::MEMORY_ALLOC => sys_memory_alloc(frame),
        call_no::MEMORY_MAP => sys_memory_map(frame),
        call_no::MEMORY_UNMAP => sys_memory_unmap(frame),
        call_no::MEMORY_CLOSE => sys_memory_close(frame),
        call_no::IPC_SCALAR_SEND_MOVE => sys_ipc_scalar_send_move(frame),
        call_no::IPC_SCALAR_CALL_MOVE => sys_ipc_scalar_call_move(frame),
        call_no::IPC_REPLY_MOVE => sys_ipc_reply_move(frame),
        call_no::IPC_SCALAR_CALL_BORROW_READ => sys_ipc_scalar_call_borrow_read(frame),
        call_no::IPC_SCALAR_CALL_BORROW_WRITE => sys_ipc_scalar_call_borrow_write(frame),
        call_no::IPC_SCALAR_SEND_COPY => sys_ipc_scalar_send_copy(frame),
        call_no::IPC_SCALAR_CALL_COPY => sys_ipc_scalar_call_copy(frame),
        call_no::IPC_SCALAR_CALL_CONNECTION => sys_ipc_scalar_call_connection(frame),
        call_no::IPC_SCALAR_CALL_CONNECTION_COPY => sys_ipc_scalar_call_connection_copy(frame),
        call_no::CQ_WAKE => sys_cq_wake(frame),
        call_no::CQ_WAIT_TIMEOUT => sys_cq_wait_timeout(frame),
        call_no::IPC_ENDPOINT_BIND_CQ => sys_ipc_endpoint_bind_cq(frame),
        call_no::DEVICE_MMIO_MAP => sys_device_mmio_map(frame),
        call_no::DEVICE_MMIO_UNMAP => sys_device_mmio_unmap(frame),
        call_no::DEVICE_IRQ_BIND_CQ => sys_device_irq_bind_cq(frame),
        call_no::DEVICE_IRQ_ACK => sys_device_irq_ack(frame),
        call_no::DEVICE_CLOSE => sys_device_close(frame),
        call_no::MEMORY_GET_PHYS => sys_memory_get_phys(frame),
        call_no::SPAWN_UPGRADE => {
            #[cfg(target_arch = "aarch64")]
            sys_spawn_upgrade(frame);
            #[cfg(not(target_arch = "aarch64"))]
            {
                frame.regs[0] = 0;
            }
        }
        call_no::IPC_VECTOR_SEND => sys_ipc_vector_send(frame),
        call_no::IPC_VECTOR_CALL => sys_ipc_vector_call(frame),
        call_no::IPC_RECV_VEC => sys_ipc_recv_vec(frame),
        _ => panic!("Unknown syscall number: {}", syscall_no),
    }
}

// ---- individual syscall implementations ------------------------------------

fn caller_asid(frame: &TrapFrame) -> crate::memory::AddressSpaceId {
    assert_ne!(
        frame.asid,
        crate::memory::KERNEL_ASID,
        "address-space-scoped syscalls require a non-kernel caller ASID"
    );
    frame.asid
}

fn sys_log(frame: &mut TrapFrame) {
    let a = frame.regs[1];
    let b = frame.regs[2];
    let lp = frame.lp_id;
    let asid = frame.asid;
    crate::early_logln!("[EL0 LOG] lp={} asid={} a={:#x} b={:#x}", lp, asid, a, b);
}

fn sys_completion_submit(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let op_code = frame.regs[1];
    let buf_ptr = frame.regs[2] as usize;
    let buf_len = frame.regs[3] as usize;

    let op = match op_code {
        0 => crate::completion::OpCode::Nop,
        1 => crate::completion::OpCode::Read,
        2 => crate::completion::OpCode::Write,
        3 => crate::completion::OpCode::Timer,
        _ => panic!("Unknown op_code in syscall submit: {}", op_code),
    };

    // For Read/Write operations the user may pass a buffer (VA, len) in x2/x3.
    // Translate the user VA to an HHDM pointer so the kernel can access the
    // buffer, allocate a cap, do the "work" synchronously (write a sentinel
    // into the buffer via HHDM), and complete — proving the full buffer-
    // ownership round-trip through the ABI with one syscall.
    let did_read = if op == crate::completion::OpCode::Read && buf_ptr != 0 && buf_len >= 4 {
        let vaddr = crate::memory::linear::VAddr::from(buf_ptr);
        let paddr = {
            let mut table = crate::memory::ADDRESS_SPACE_TABLE.lock();
            table
                .get_mut(asid)
                .expect("SUBMIT: address space not found")
                .translate_address(vaddr)
                .expect("SUBMIT: failed to translate user buffer VA")
        };
        let hhdm: *mut u8 = paddr.into();
        Some((hhdm, buf_len))
    } else {
        None
    };

    if op == crate::completion::OpCode::Timer {
        let timeout_ms = buf_len as u64;
        match crate::completion::submit_timer(asid, timeout_ms) {
            Ok(cap) => {
                crate::cpu::scheduler::bump_active_timers();
                frame.regs[0] = cap as u64;
            }
            Err(_) => frame.regs[0] = 0,
        }
        return;
    }

    match crate::completion::submit(asid, op, None) {
        Ok(cap) => {
            if let Some((hhdm, len)) = did_read {
                // Do the "work": write a sentinel into the user's buffer.
                unsafe {
                    core::ptr::write_volatile(hhdm as *mut u32, 0xfeed_f00d);
                }
                let _ = crate::completion::complete(
                    asid,
                    cap,
                    crate::completion::OpResult::Ok(len as i64),
                );
            }
            frame.regs[0] = cap as u64;
        }
        Err(_) => panic!("syscall completion submit failed"),
    }
}

fn sys_completion_complete(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as usize;
    let result_code = frame.regs[2] as i64;
    let result = if result_code >= 0 {
        crate::completion::OpResult::Ok(result_code)
    } else {
        crate::completion::OpResult::Err(result_code as i32)
    };
    let _ = crate::completion::complete(asid, cap, result);
}

fn sys_completion_poll(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as usize;
    match crate::completion::poll(asid, cap) {
        Ok(Some(completed)) => {
            crate::cpu::scheduler::drop_active_timer();
            frame.regs[0] = 0;
            frame.regs[1] = crate::completion::cq::op_result_to_i64(completed.result) as u64;
            frame.regs[2] = completed.buffer.as_ref().map_or(0, |buf| buf.len()) as u64;
        }
        Ok(None) => {
            frame.regs[0] = 1;
            frame.regs[1] = 0;
            frame.regs[2] = 0;
        }
        Err(_) => panic!("syscall completion poll failed: unknown cap {}", cap),
    }
}

fn sys_completion_wait(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as usize;
    let _ = crate::completion::wait(asid, cap);
}

fn sys_completion_cancel(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as usize;
    let _ = crate::completion::cancel(asid, cap);
}

fn sys_completion_close(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as usize;
    if crate::completion::close(asid, cap).is_ok() {
        crate::cpu::scheduler::drop_active_timer();
    }
}

fn sys_spawn_thread(frame: &mut TrapFrame) {
    use crate::cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::{
            MASTER_THREAD_TABLE,
            Thread,
        },
    };
    let asid = caller_asid(frame);
    let entry_vaddr = frame.regs[1] as usize;
    let target_lp = frame.regs[2] as LpId;

    // A shard runs at EL0 in the *caller's* address space. `Thread::new` with a
    // non-kernel ASID builds a user thread context that drops to EL0 via
    // `user_trampoline`, loading the entry into ELR_EL1 and switching TTBR0 to
    // the caller's AS. The entry is therefore a virtual address in that AS and
    // must NOT be translated to a physical/HHDM pointer — doing so (and using
    // KERNEL_ASID) would run the target as an EL1 kernel thread.
    assert!(
        asid != crate::memory::KERNEL_ASID,
        "SPAWN_THREAD: refusing to spawn a shard into the kernel address space",
    );
    let entry_fn: extern "C" fn() =
        unsafe { core::mem::transmute::<usize, extern "C" fn()>(entry_vaddr) };

    // Create the thread and pin it directly to the requested LP. We must not go
    // through `scheduler::spawn_thread` (which submits to the least-loaded LP)
    // and then `submit_to_lp`: that would enqueue the same thread on two run
    // queues.
    let thread = Thread::new(asid, entry_fn);
    let tid = MASTER_THREAD_TABLE.write().add_element(thread);
    SYSTEM_SCHEDULER.read().submit_to_lp(tid, target_lp).unwrap_or_else(|_| {
        let lpc = crate::cpu::multiprocessor::get_lp_count();
        logln!(
            "SPAWN_THREAD: target LP {target_lp} does not exist (lp_count={lpc}); falling back to \
             least-loaded LP"
        );
        SYSTEM_SCHEDULER
            .read()
            .submit_ready_thread(tid)
            .map(|_| ())
            .expect("SPAWN_THREAD: submit_ready_thread fallback failed")
    });
    // Return the thread id in x0.
    frame.regs[0] = tid as u64;
}

// ---- new syscalls -----------------------------------------------------------

fn sys_thread_exit(_frame: &mut TrapFrame) {
    // `abort` deschedules the thread, reaps it, and never returns.
    crate::cpu::scheduler::abort();
}

use spin::{
    LazyLock,
    RwLock,
};

/// A kernel-global mailbox set for EL0-to-EL0 inter-LP messaging. One bounded
/// MPSC queue per LP; senders target a specific LP, receivers drain their own.
use crate::cpu::multiprocessor::{
    get_lp_count,
    shard_mailbox::ShardMailboxSet,
};
static USER_MAILBOX: LazyLock<ShardMailboxSet<u64>> = LazyLock::new(|| ShardMailboxSet::new(256));

type MailboxCap = u64;

#[derive(Clone, Copy)]
enum MailboxEndpoint {
    Sender {
        target_lp: LpId,
    },
    Receiver {
        lp: LpId,
    },
}

struct AsMailboxCaps {
    next: MailboxCap,
    endpoints: BTreeMap<MailboxCap, MailboxEndpoint>,
}

impl AsMailboxCaps {
    fn new() -> Self {
        Self {
            next: 1,
            endpoints: BTreeMap::new(),
        }
    }

    fn insert(&mut self, endpoint: MailboxEndpoint) -> MailboxCap {
        let cap = self.next;
        self.next = self.next.checked_add(1).expect("mailbox capability id overflow");
        self.endpoints.insert(cap, endpoint);
        cap
    }

    fn receiver_for_or_insert(&mut self, lp: LpId) -> MailboxCap {
        if let Some((cap, _)) = self.endpoints.iter().find(|(_, endpoint)| {
            matches!(
                endpoint,
                MailboxEndpoint::Receiver {
                    lp: endpoint_lp,
                } if *endpoint_lp == lp
            )
        }) {
            return *cap;
        }
        self.insert(MailboxEndpoint::Receiver {
            lp,
        })
    }
}

static USER_MAILBOX_CAPS: LazyLock<RwLock<BTreeMap<AddressSpaceId, AsMailboxCaps>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

pub fn close_mailbox_address_space(asid: AddressSpaceId) {
    USER_MAILBOX_CAPS.write().remove(&asid);
}

fn sys_mailbox_send(frame: &mut TrapFrame) {
    let _asid = caller_asid(frame);
    let target_lp = frame.regs[1] as u32;
    let message = frame.regs[2];
    frame.regs[0] = match USER_MAILBOX.try_send_to(target_lp, message) {
        Ok(()) => 0,
        Err(_) => 1, // invalid LP or queue full
    };
}

fn sys_mailbox_recv(frame: &mut TrapFrame) {
    let _asid = caller_asid(frame);
    match USER_MAILBOX.try_recv_for_current_lp() {
        Some(msg) => {
            frame.regs[0] = msg;
            frame.regs[1] = 0; // got a message
        }
        None => {
            frame.regs[0] = 0;
            frame.regs[1] = 1; // empty
        }
    }
}

fn sys_mailbox_open_send(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let target_lp = frame.regs[1] as LpId;
    if target_lp >= get_lp_count() {
        frame.regs[0] = 0;
        return;
    }
    let mut tables = USER_MAILBOX_CAPS.write();
    let caps = tables.entry(asid).or_insert_with(AsMailboxCaps::new);
    frame.regs[0] = caps.insert(MailboxEndpoint::Sender {
        target_lp,
    });
}

fn sys_mailbox_open_recv(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let lp = get_lp_id();
    let mut tables = USER_MAILBOX_CAPS.write();
    let caps = tables.entry(asid).or_insert_with(AsMailboxCaps::new);
    frame.regs[0] = caps.receiver_for_or_insert(lp);
}

fn mailbox_endpoint(asid: AddressSpaceId, cap: MailboxCap) -> Option<MailboxEndpoint> {
    USER_MAILBOX_CAPS.read().get(&asid).and_then(|caps| caps.endpoints.get(&cap).copied())
}

fn sys_mailbox_send_cap(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as MailboxCap;
    let message = frame.regs[2];
    frame.regs[0] = match mailbox_endpoint(asid, cap) {
        Some(MailboxEndpoint::Sender {
            target_lp,
        }) => match USER_MAILBOX.try_send_to(target_lp, message) {
            Ok(()) => 0,
            Err(_) => 1,
        },
        _ => 2,
    };
}

fn sys_mailbox_recv_cap(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as MailboxCap;
    match mailbox_endpoint(asid, cap) {
        Some(MailboxEndpoint::Receiver {
            lp,
        }) if lp == get_lp_id() => match USER_MAILBOX.try_recv_for_current_lp() {
            Some(msg) => {
                frame.regs[0] = msg;
                frame.regs[1] = 0;
            }
            None => {
                frame.regs[0] = 0;
                frame.regs[1] = 1;
            }
        },
        _ => {
            frame.regs[0] = 0;
            frame.regs[1] = 2;
        }
    }
}

fn sys_mailbox_close(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1] as MailboxCap;
    let mut tables = USER_MAILBOX_CAPS.write();
    frame.regs[0] = match tables.get_mut(&asid).and_then(|caps| caps.endpoints.remove(&cap)) {
        Some(_) => 0,
        None => 1,
    };
}

fn ipc_status(error: ipc::IpcError) -> u64 {
    match error {
        ipc::IpcError::QueueFull => 1,
        ipc::IpcError::NoMessage => 2,
        ipc::IpcError::Pending => 3,
        ipc::IpcError::UnknownCapability => 4,
        ipc::IpcError::WrongType => 5,
        ipc::IpcError::PermissionDenied => 6,
        ipc::IpcError::EndpointClosed => 7,
        ipc::IpcError::ReplyAlreadyUsed => 8,
        ipc::IpcError::MemoryTransferFailed => 9,
    }
}

fn memory_status(error: object::MemoryObjectError) -> u64 {
    match error {
        object::MemoryObjectError::UnknownCapability => 1,
        object::MemoryObjectError::WrongOwner => 2,
        object::MemoryObjectError::AlreadyMapped => 3,
        object::MemoryObjectError::NotMapped => 4,
        object::MemoryObjectError::InvalidLength => 5,
        object::MemoryObjectError::NotPageAligned => 6,
        object::MemoryObjectError::AddressSpaceMissing => 7,
        object::MemoryObjectError::MapFailed => 8,
        object::MemoryObjectError::UnmapFailed => 9,
        object::MemoryObjectError::FrameAllocFailed => 10,
        object::MemoryObjectError::FrameFreeFailed => 11,
        object::MemoryObjectError::MissingRight => 12,
        object::MemoryObjectError::LendingActive => 13,
        object::MemoryObjectError::NotLent => 14,
    }
}

fn sys_memory_alloc(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let pages = frame.regs[1] as usize;
    frame.regs[0] = object::allocate(asid, pages).unwrap_or(0);
}

fn sys_memory_map(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    let base = VAddr::from(frame.regs[2] as usize);
    let writable = frame.regs[3] != 0;
    frame.regs[0] = match object::map(asid, cap, base, writable) {
        Ok(()) => 0,
        Err(error) => memory_status(error),
    };
}

fn sys_memory_unmap(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    frame.regs[0] = match object::unmap(asid, cap) {
        Ok(()) => 0,
        Err(error) => memory_status(error),
    };
}

fn sys_memory_close(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    frame.regs[0] = match object::close_cap(asid, cap) {
        Ok(()) => 0,
        Err(error) => memory_status(error),
    };
}

fn sys_memory_get_phys(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    frame.regs[0] = object::get_phys(asid, cap);
}

fn sys_ipc_endpoint_create(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let interface = frame.regs[1];
    let version = frame.regs[2] as u32;
    let capacity = frame.regs[3] as usize;
    frame.regs[0] = ipc::endpoint_create(asid, interface, version, capacity).unwrap_or(0);
}

fn sys_ipc_connect(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let endpoint = frame.regs[1];
    let rights = ipc::ConnectionRights::from_bits(frame.regs[2] as u32);
    frame.regs[0] = ipc::connection_mint(asid, endpoint, rights).unwrap_or(0);
}

fn sys_ipc_scalar_send(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    frame.regs[0] = match ipc::scalar_send(asid, connection, opcode, arg0) {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn sys_ipc_scalar_call(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    frame.regs[0] = ipc::scalar_call(asid, connection, opcode, arg0).unwrap_or(0);
}

fn sys_ipc_recv(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let endpoint = frame.regs[1];
    recv_into_frame(frame, asid, endpoint);
}

fn recv_into_frame(frame: &mut TrapFrame, asid: AddressSpaceId, endpoint: u64) {
    match ipc::receive(asid, endpoint) {
        Ok(message) => {
            frame.regs[0] = 0;
            frame.regs[1] = message.opcode as u64;
            frame.regs[2] = message.arg0;
            frame.regs[3] = message.reply.unwrap_or(0);
            frame.regs[4] = message.sender as u64;
            frame.regs[5] = message.interface;
            frame.regs[6] = message.version as u64;
            frame.regs[7] = message.memory.unwrap_or(0);
            frame.regs[8] = message.connection.unwrap_or(0);
        }
        Err(error) => {
            frame.regs[0] = ipc_status(error);
            for reg in &mut frame.regs[1..=8] {
                *reg = 0;
            }
        }
    }
}

fn sys_ipc_recv_block(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let endpoint = frame.regs[1];
    if let Err(error) = ipc::wait_readable(asid, endpoint) {
        frame.regs[0] = ipc_status(error);
        for reg in &mut frame.regs[1..=8] {
            *reg = 0;
        }
        return;
    }
    recv_into_frame(frame, asid, endpoint);
}

fn sys_ipc_reply(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let reply = frame.regs[1];
    let result = frame.regs[2] as i64;
    frame.regs[0] = match ipc::reply(asid, reply, result) {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn sys_ipc_reply_poll(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let call = frame.regs[1];
    match ipc::poll_reply(asid, call) {
        Ok(Some(result)) => {
            frame.regs[0] = 0;
            frame.regs[1] = result.result as u64;
            frame.regs[2] = result.cap.unwrap_or(0);
            frame.regs[3] = result.memory.unwrap_or(0);
        }
        Ok(None) => {
            frame.regs[0] = 1;
            frame.regs[1] = 0;
            frame.regs[2] = 0;
            frame.regs[3] = 0;
        }
        Err(error) => {
            frame.regs[0] = ipc_status(error);
            frame.regs[1] = 0;
            frame.regs[2] = 0;
            frame.regs[3] = 0;
        }
    }
}

fn sys_ipc_close(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    frame.regs[0] = match ipc::close_cap(asid, cap) {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn sys_ipc_reply_connection(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let reply = frame.regs[1];
    let endpoint = frame.regs[2];
    let rights = ipc::ConnectionRights::from_bits(frame.regs[3] as u32);
    let result = frame.regs[4] as i64;
    frame.regs[0] = match ipc::reply_with_connection(asid, reply, endpoint, rights, result) {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn sys_ipc_scalar_call_connection(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let delegate = frame.regs[4];
    let rights = ipc::ConnectionRights::from_bits(frame.regs[5] as u32);
    frame.regs[0] =
        ipc::scalar_call_with_connection(asid, connection, opcode, arg0, delegate, rights)
            .unwrap_or(0);
}

fn sys_ipc_scalar_call_connection_copy(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let delegate = frame.regs[4];
    let rights = ipc::ConnectionRights::from_bits(frame.regs[5] as u32);
    let memory = frame.regs[6];
    frame.regs[0] = ipc::scalar_call_with_connection_copy(
        asid, connection, opcode, arg0, delegate, rights, memory,
    )
    .unwrap_or(0);
}

fn sys_ipc_scalar_send_move(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let memory = frame.regs[4];
    frame.regs[0] = match ipc::scalar_send_with_memory_move(asid, connection, opcode, arg0, memory)
    {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn sys_ipc_scalar_call_move(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let memory = frame.regs[4];
    frame.regs[0] =
        ipc::scalar_call_with_memory_move(asid, connection, opcode, arg0, memory).unwrap_or(0);
}

fn sys_ipc_reply_move(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let reply = frame.regs[1];
    let memory = frame.regs[2];
    let result = frame.regs[3] as i64;
    frame.regs[0] = match ipc::reply_with_memory_move(asid, reply, memory, result) {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn sys_ipc_scalar_call_borrow_read(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let memory = frame.regs[4];
    frame.regs[0] =
        ipc::scalar_call_with_memory_borrow_read(asid, connection, opcode, arg0, memory)
            .unwrap_or(0);
}

fn sys_ipc_scalar_call_borrow_write(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let memory = frame.regs[4];
    frame.regs[0] =
        ipc::scalar_call_with_memory_borrow_write(asid, connection, opcode, arg0, memory)
            .unwrap_or(0);
}

fn sys_ipc_scalar_send_copy(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let memory = frame.regs[4];
    frame.regs[0] = match ipc::scalar_send_with_memory_copy(asid, connection, opcode, arg0, memory)
    {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn sys_ipc_scalar_call_copy(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let memory = frame.regs[4];
    frame.regs[0] =
        ipc::scalar_call_with_memory_copy(asid, connection, opcode, arg0, memory).unwrap_or(0);
}

fn sys_ipc_vector_send(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let cap_vector = frame.regs[4];
    frame.regs[0] = match ipc::vector_send(asid, connection, opcode, arg0, cap_vector) {
        Ok(()) => 0,
        Err(error) => error as u64,
    };
}

fn sys_ipc_vector_call(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let connection = frame.regs[1];
    let opcode = frame.regs[2] as u32;
    let arg0 = frame.regs[3];
    let cap_vector = frame.regs[4];
    frame.regs[0] = ipc::vector_call(asid, connection, opcode, arg0, cap_vector).unwrap_or(0);
}

fn sys_ipc_recv_vec(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let endpoint = frame.regs[1];
    let result_page = frame.regs[3];
    match ipc::receive_vec(asid, endpoint, result_page) {
        Ok(msg) => {
            frame.regs[0] = 0;
            frame.regs[1] = msg.opcode as u64;
            frame.regs[2] = msg.arg0;
            frame.regs[3] = msg.reply.unwrap_or(0);
            frame.regs[4] = msg.sender as u64;
            frame.regs[5] = msg.interface;
            frame.regs[6] = msg.version as u64;
            frame.regs[7] = msg.memory.unwrap_or(0) as u64;
            frame.regs[8] = msg.connection.unwrap_or(0) as u64;
        }
        Err(error) => {
            frame.regs[0] = error as u64;
        }
    }
}

fn sys_completion_wait_timeout(frame: &mut TrapFrame) {
    use alloc::sync::{
        Arc,
        Weak,
    };

    use crate::{
        cpu::scheduler::{
            system_scheduler::SYSTEM_SCHEDULER,
            yield_lp,
        },
        klib::{
            observer::Observable as _,
            time::duration::ExtDuration,
        },
        timers::{
            TIMER_QUEUES,
            TimerEvent,
        },
    };

    let asid = caller_asid(frame);
    let cap = frame.regs[1] as usize;
    let timeout_ms = frame.regs[2] as u64;

    // Structure that wakes the blocked thread when signalled (by either the
    // timer firing or the completion finishing).
    struct TimeoutWake {
        tid: crate::cpu::scheduler::threads::ThreadId,
    }
    impl crate::klib::observer::Observer for TimeoutWake {
        fn notify(self: alloc::sync::Arc<Self>) {
            let _ = SYSTEM_SCHEDULER.read().submit_ready_thread(self.tid);
        }
    }

    // Fast path: completion already posted.
    match crate::completion::poll(asid, cap) {
        Ok(Some(completed)) => {
            frame.regs[0] = 0;
            frame.regs[1] = crate::completion::cq::op_result_to_i64(completed.result) as u64;
            return;
        }
        _ => {}
    }

    let tid = SYSTEM_SCHEDULER
        .read()
        .get_lp_scheduler()
        .lock()
        .get_tid()
        .expect("COMPLETION_WAIT_TIMEOUT: no running thread");

    // Block on the completion.
    SYSTEM_SCHEDULER
        .read()
        .block_thread(
            tid,
            &*crate::completion::completion_of(asid, cap)
                .expect("COMPLETION_WAIT_TIMEOUT: unknown cap"),
        )
        .expect("COMPLETION_WAIT_TIMEOUT: failed to block thread");

    // Arm a timer that also wakes this thread (timeout path).
    let timeout_obs = Arc::new(TimeoutWake {
        tid,
    });
    let timer_event = TimerEvent::from(ExtDuration::from_millis(timeout_ms as u128));
    timer_event.register_observer(
        Arc::downgrade(&timeout_obs) as Weak<dyn crate::klib::observer::Observer>
    );
    // SAFETY: TIMER_QUEUES is initialised by bsp_init before self-tests or
    // any threads run.
    unsafe { TIMER_QUEUES.try_get_mut().unwrap_unchecked() }.add_event(timer_event);

    yield_lp();

    // Woke up: check whether the completion or the timeout won.
    match crate::completion::poll(asid, cap) {
        Ok(Some(completed)) => {
            // Write the result back: x0 = 0 (success), x1 = result code.
            let code = crate::completion::cq::op_result_to_i64(completed.result);
            frame.regs[0] = 0;
            frame.regs[1] = code as u64;
        }
        _ => {
            frame.regs[0] = 1; // timeout
        }
    }
}

fn sys_cq_wait(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let min_complete = frame.regs[1].max(1) as u32;
    let cq = frame.regs[2] as u32;
    crate::completion::wait_on_cq(asid, cq, min_complete);
    frame.regs[0] = crate::completion::cq_pending(asid, cq) as u64;
}

fn sys_cq_wake(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cq = frame.regs[1] as u32;
    crate::completion::wake(asid, cq);
    frame.regs[0] = 0;
}

fn sys_cq_wait_timeout(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let min_complete = frame.regs[1].max(1) as u32;
    let timeout_ms = frame.regs[2];
    let cq = frame.regs[3] as u32;
    let condition_met = crate::completion::wait_on_cq_timeout(asid, cq, min_complete, timeout_ms);
    frame.regs[0] = crate::completion::cq_pending(asid, cq) as u64;
    frame.regs[1] = if condition_met {
        0
    } else {
        1
    };
}

fn sys_ipc_endpoint_bind_cq(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let endpoint = frame.regs[1];
    let cq = frame.regs[2] as u32;
    frame.regs[0] = match ipc::endpoint_bind_cq(asid, endpoint, cq) {
        Ok(()) => 0,
        Err(error) => ipc_status(error),
    };
}

fn device_status(error: crate::device::DeviceError) -> u64 {
    use crate::device::DeviceError;
    match error {
        DeviceError::UnknownCapability => 1,
        DeviceError::WrongType => 2,
        DeviceError::AlreadyMapped => 3,
        DeviceError::NotMapped => 4,
        DeviceError::MapFailed => 5,
        DeviceError::NotBound => 6,
        DeviceError::AlreadyBound => 7,
        DeviceError::NotPageAligned => 8,
        DeviceError::InvalidInterrupt => 9,
    }
}

fn sys_device_mmio_map(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    let base = VAddr::from(frame.regs[2] as usize);
    let writable = frame.regs[3] != 0;
    frame.regs[0] = match crate::device::mmio_map(asid, cap, base, writable) {
        Ok(()) => 0,
        Err(error) => device_status(error),
    };
}

fn sys_device_mmio_unmap(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    frame.regs[0] = match crate::device::mmio_unmap(asid, cap) {
        Ok(()) => 0,
        Err(error) => device_status(error),
    };
}

fn sys_device_irq_bind_cq(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    let cq = frame.regs[2] as u32;
    frame.regs[0] = match crate::device::interrupt_bind_cq(asid, cap, cq) {
        Ok(()) => 0,
        Err(error) => device_status(error),
    };
}

fn sys_device_irq_ack(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    match crate::device::interrupt_ack(asid, cap) {
        Ok(consumed) => {
            frame.regs[0] = 0;
            frame.regs[1] = consumed as u64;
        }
        Err(error) => {
            frame.regs[0] = device_status(error);
            frame.regs[1] = 0;
        }
    }
}

fn sys_device_close(frame: &mut TrapFrame) {
    let asid = caller_asid(frame);
    let cap = frame.regs[1];
    frame.regs[0] = match crate::device::close_cap(asid, cap) {
        Ok(()) => 0,
        Err(error) => device_status(error),
    };
}

#[cfg(target_arch = "aarch64")]
fn sys_spawn_upgrade(frame: &mut TrapFrame) {
    let caller_asid = caller_asid(frame);
    let elf_selector = frame.regs[2];
    let state_cap = frame.regs[3];
    let _endpoint_cap = frame.regs[4];

    let elf = match crate::service::supervisor::elf_for_selector(elf_selector) {
        Some(image) => image,
        None => {
            frame.regs[0] = 0;
            return;
        }
    };

    let loaded = crate::service::loader::load_domain(elf);
    let ns_guard = crate::service::supervisor::LIVE_NS.lock();
    let ns_handle = match ns_guard.as_ref() {
        Some(h) => h,
        None => {
            frame.regs[0] = 0;
            return;
        }
    };

    // Delegate a bootstrap connection from the name service to the new domain.
    let bootstrap_conn = match crate::ipc::connection_delegate(
        ns_handle.domain.asid,
        ns_handle.endpoint_cap,
        loaded.asid,
        crate::ipc::ConnectionRights::CALL,
    ) {
        Ok(cap) => cap,
        Err(_) => {
            frame.regs[0] = 0;
            return;
        }
    };
    crate::service::bootstrap::write_bootstrap_cap(loaded.config_frame, bootstrap_conn);
    crate::service::bootstrap::write_argc(loaded.config_frame, 0);

    // Move the state cap from the caller to the new domain.
    if state_cap != 0 {
        let _ = crate::memory::object::move_to(caller_asid, state_cap, loaded.asid);
        crate::service::bootstrap::write_handoff_state(loaded.config_frame, 1, state_cap, 0);
    }

    // Start the replacement domain.
    let entry_vaddr = loaded.entry_vaddr;
    let entry: extern "C" fn() =
        unsafe { core::mem::transmute::<usize, extern "C" fn()>(entry_vaddr) };
    crate::cpu::scheduler::spawn_thread(loaded.asid, entry);

    // Report the new domain's ASID as evidence.
    frame.regs[0] = loaded.asid as u64;
}
