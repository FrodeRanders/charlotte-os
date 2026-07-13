//! # Syscall Dispatch Subsystem
//!
//! This module defines the AArch64 syscall dispatch table and the per-ISA
//! [`TrapFrame`] type that the [`sync_dispatcher`] (when handling an SVC from a
//! lower EL) passes into the dispatch function. It also contains the public
//! `syscall_dispatch` entry point that [`sync_dispatcher`] calls after decoding
//! the exception class.
//!
//! At this prototype stage the syscalls mirror the completion-capability ABI
//! operations in [`crate::completion`], making only what already compiles
//! callable from a (future) user thread. The real user-register-to-semantic
//! mapping (which registers carry the buffer pointer, how buffer ownership
//! crosses the EL boundary, etc.) depends on the shared-memory CQ/SQ ring or a
//! copy-based IPC channel — neither exists yet. This module wires the dispatch
//! path itself, which is the prerequisite for everything else.
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
    memory::AddressSpaceId,
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

/// The upper bound on the SVC immediate we will try to dispatch.
pub const MAX_SYSCALL: u16 = 17;

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
    /// Block until the caller's CQ ring has at least `x1` pending entries.
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
}

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
        _ => panic!("Unknown syscall number: {}", syscall_no),
    }
}

// ---- individual syscall implementations ------------------------------------

fn caller_asid(frame: &TrapFrame) -> crate::memory::AddressSpaceId {
    if frame.asid != crate::memory::KERNEL_ASID {
        // Real EL0 syscall: asid was captured from the thread at entry.
        frame.asid
    } else {
        // Synthetic self-test frame (asid defaults to KERNEL_ASID): the test
        // passes the ASID explicitly in x0.
        frame.regs[0] as crate::memory::AddressSpaceId
    }
}

fn sys_log(frame: &mut TrapFrame) {
    let _ptr = frame.regs[0] as *const u8;
    let _len = frame.regs[1] as usize;
    let lp = frame.lp_id;
    crate::early_logln!("[EL0 SYSCALL] LOG from userspace on LP {}", lp);
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
    let _ = crate::completion::close(asid, cap);
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
    SYSTEM_SCHEDULER
        .read()
        .submit_to_lp(tid, target_lp)
        .expect("SPAWN_THREAD: failed to pin shard thread to target LP");
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
        .write()
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
    crate::completion::wait_on_cq(asid, min_complete);
    frame.regs[0] = crate::completion::cq_pending(asid) as u64;
}
