//! The CharlotteOS Reliable Message Layer service.
//!
//! Provides sequenced, acknowledged, retransmitted message delivery
//! between services. Clients connect to "relmsg" and use OP_SEND to
//! send messages addressed to peer service names, and OP_RECV to
//! receive messages destined for them.
//!
//! In the current prototype, messages are delivered locally (same-machine
//! loopback). When the NIC driver is runtime-validated on KVM, the
//! service will be extended to encapsulate messages in Ethernet frames
//! via `charlotte-protocol-msg`.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    ns,
    relmsg,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;
const STAGE_OFFSET: usize = 0;
const SCRATCH_VADDR: usize = 0x0000_0000_0080_0000;

struct QueuedMsg {
    data_cap: u64,
    data_len: usize,
}

struct PendingRecv {
    reply_token: u64,
}

struct RmlState {
    queue: Vec<QueuedMsg>,
    pending_recv: Option<PendingRecv>,
}

impl RmlState {
    fn new() -> Self {
        Self {
            queue: Vec::new(),
            pending_recv: None,
        }
    }

    fn try_deliver(&mut self) {
        if let Some(pr) = self.pending_recv.take() {
            if let Some(msg) = self.queue.pop() {
                let out_cap = memory_alloc(1);
                if out_cap == 0 {
                    self.pending_recv = Some(pr);
                    return;
                }
                if memory_map(out_cap, SCRATCH_VADDR, true) != 0 {
                    memory_close(out_cap);
                    self.pending_recv = Some(pr);
                    return;
                }
                if memory_map(msg.data_cap, SCRATCH_VADDR + 0x1000, false) != 0 {
                    memory_unmap(out_cap);
                    memory_close(out_cap);
                    self.queue.push(msg);
                    self.pending_recv = Some(pr);
                    return;
                }
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        (SCRATCH_VADDR + 0x1000) as *const u8,
                        SCRATCH_VADDR as *mut u8,
                        msg.data_len,
                    );
                }
                memory_unmap(msg.data_cap);
                memory_close(msg.data_cap);
                memory_unmap(out_cap);
                if ipc_reply_move(pr.reply_token, out_cap, msg.data_len as i64) != 0 {
                    memory_close(out_cap);
                }
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(c) => c,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2);

    let ep = ipc_endpoint_create(relmsg::INTERFACE, relmsg::VERSION, 8);
    if ep == 0 {
        unsafe { thread_exit() };
    }
    let reg = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        relmsg::NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL,
    );
    if reg == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { wait_reply(reg, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 3);

    let mut state = RmlState::new();
    let mut served: u32 = 0;

    loop {
        loop {
            let msg = ipc_recv(ep);
            if msg.status == ipc_status::NO_MESSAGE {
                break;
            }
            if msg.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !msg.is_ok() {
                break;
            }

            if msg.reply == 0 {
                if msg.memory != 0 {
                    memory_close(msg.memory);
                }
                continue;
            }

            match msg.opcode {
                relmsg::OP_SEND => {
                    if msg.memory == 0 {
                        ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                        continue;
                    }
                    if memory_map(msg.memory, SCRATCH_VADDR, false) != 0 {
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                        continue;
                    }
                    let mut header = [0u8; charlotte_protocol_msg::HEADER_SIZE];
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            SCRATCH_VADDR as *const u8,
                            header.as_mut_ptr(),
                            header.len(),
                        );
                    }
                    memory_unmap(msg.memory);
                    let Ok((seq, _ack, payload_len, flags)) =
                        charlotte_protocol_msg::parse_header_checked(&header)
                    else {
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                        continue;
                    };

                    if flags & charlotte_protocol_msg::FLAG_SYN != 0 {
                        memory_close(msg.memory);
                        let cap = memory_alloc(1);
                        if cap == 0 {
                            ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                            continue;
                        }
                        if memory_map(cap, SCRATCH_VADDR, true) != 0 {
                            memory_close(cap);
                            ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                            continue;
                        }
                        let mut ack_hdr = [0u8; 16];
                        charlotte_protocol_msg::build_header(
                            &mut ack_hdr,
                            0,
                            seq,
                            0,
                            charlotte_protocol_msg::FLAG_SYN | charlotte_protocol_msg::FLAG_ACK,
                        );
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                ack_hdr.as_ptr(),
                                SCRATCH_VADDR as *mut u8,
                                16,
                            );
                        }
                        memory_unmap(cap);
                        if ipc_reply_move(msg.reply, cap, 0) != 0 {
                            memory_close(cap);
                        }
                        continue;
                    }

                    // Regular data message: queue it for delivery.
                    let data_cap = memory_alloc(1);
                    if data_cap == 0 {
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                        continue;
                    }
                    if memory_map(data_cap, SCRATCH_VADDR + 0x2000, true) != 0 {
                        memory_close(data_cap);
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                        continue;
                    }
                    // Copy the payload (after 16-byte header) from sender's memory.
                    if memory_map(msg.memory, SCRATCH_VADDR, false) != 0 {
                        memory_unmap(data_cap);
                        memory_close(data_cap);
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, relmsg::ERR_UNKNOWN);
                        continue;
                    }
                    unsafe {
                        let src = (SCRATCH_VADDR + 16) as *const u8;
                        let dst = (SCRATCH_VADDR + 0x2000) as *mut u8;
                        let len = payload_len as usize;
                        core::ptr::copy_nonoverlapping(src, dst, len);
                    }
                    memory_unmap(msg.memory);
                    memory_close(msg.memory);
                    memory_unmap(data_cap);

                    served += 1;
                    config::write::<u32>(4, served);
                    state.queue.push(QueuedMsg {
                        data_cap,
                        data_len: payload_len as usize,
                    });
                    state.try_deliver();
                    ipc_reply(msg.reply, payload_len as i64);
                }

                relmsg::OP_RECV => {
                    if state.pending_recv.is_some() {
                        ipc_reply(msg.reply, 0);
                        continue;
                    }
                    state.pending_recv = Some(PendingRecv {
                        reply_token: msg.reply,
                    });
                    state.try_deliver();
                }

                _ => {
                    ipc_reply(msg.reply, relmsg::ERR_BAD_OPCODE);
                }
            }
        }

        cq_wait(1, 0);
    }
}

catten_rt::entry!(main);
