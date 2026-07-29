//! Reliable, sequenced messages over the node's raw Ethernet service.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::VecDeque,
    vec::Vec,
};

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    net,
    ns,
    relmsg,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait_timeout,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_recv_block,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_msg::{
    FLAG_ACK,
    FRAME_HEADER_SIZE,
    MAX_PAYLOAD_SIZE,
    build_frame_header,
    pack_address_and_len,
    parse_frame_header_checked,
    unpack_address_and_len,
};
use charlotte_protocol_net::decode_status;

const REPLY_SPINS: u64 = 50_000_000;
const STAGE_OFFSET: usize = 0;
const RX_SCRATCH: usize = 0x0000_0000_0090_0000;
const TX_SCRATCH: usize = 0x0000_0000_0090_1000;
const PAYLOAD_SCRATCH: usize = 0x0000_0000_0090_2000;

struct ReceivedMessage {
    source: [u8; 6],
    cap: u64,
    len: usize,
}

struct Outbound {
    seq: u32,
    reply: u64,
    payload_len: usize,
    frame: Vec<u8>,
    retries: u32,
}

struct Peer {
    mac: [u8; 6],
    next_tx_seq: u32,
    next_rx_seq: u32,
    pending: Option<Outbound>,
}

impl Peer {
    fn new(mac: [u8; 6]) -> Self {
        Self {
            mac,
            next_tx_seq: 1,
            next_rx_seq: 1,
            pending: None,
        }
    }
}

fn peer_index(peers: &mut Vec<Peer>, mac: [u8; 6]) -> Option<usize> {
    if let Some(index) = peers.iter().position(|peer| peer.mac == mac) {
        return Some(index);
    }
    if peers.len() >= relmsg::MAX_PEERS {
        return None;
    }
    peers.push(Peer::new(mac));
    Some(peers.len() - 1)
}

fn send_frame(net_conn: u64, frame: &[u8]) -> bool {
    if frame.len() > 4096 {
        return false;
    }
    let cap = memory_alloc(1);
    if cap == 0 {
        return false;
    }
    if memory_map(cap, TX_SCRATCH, true) != 0 {
        memory_close(cap);
        return false;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), TX_SCRATCH as *mut u8, frame.len());
    }
    memory_unmap(cap);
    let call = ipc_scalar_call_move(net_conn, net::OP_SEND, frame.len() as u64, cap);
    if call == 0 {
        config::write::<u32>(16, 1);
        memory_close(cap);
        return false;
    }
    let (result, _) = unsafe { wait_reply(call, REPLY_SPINS) };
    config::write::<i64>(16, result);
    result == 0
}

fn make_frame(
    destination: [u8; 6],
    source: [u8; 6],
    seq: u32,
    ack: u32,
    flags: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = alloc::vec![0u8; FRAME_HEADER_SIZE + payload.len()];
    let mut header = [0u8; FRAME_HEADER_SIZE];
    build_frame_header(&mut header, destination, source, seq, ack, payload.len() as u16, flags);
    frame[..FRAME_HEADER_SIZE].copy_from_slice(&header);
    frame[FRAME_HEADER_SIZE..].copy_from_slice(payload);
    frame
}

fn deliver_received(queue: &mut VecDeque<ReceivedMessage>, pending_recv: &mut u64) {
    if *pending_recv == 0 {
        return;
    }
    let Some(message) = queue.pop_front() else {
        return;
    };
    let result = pack_address_and_len(message.source, message.len as u16) as i64;
    if ipc_reply_move(*pending_recv, message.cap, result) != 0 {
        memory_close(message.cap);
    }
    *pending_recv = 0;
}

fn retransmit_pending(net_conn: u64, peers: &mut [Peer]) {
    for peer in peers {
        let Some(pending) = peer.pending.as_mut() else {
            continue;
        };
        if pending.retries >= relmsg::MAX_RETRIES {
            let pending = peer.pending.take().expect("pending checked above");
            ipc_reply(pending.reply, relmsg::ERR_PEER_UNREACHABLE);
            continue;
        }
        pending.retries += 1;
        let _ = send_frame(net_conn, &pending.frame);
    }
}

fn process_frame(
    net_conn: u64,
    local_mac: [u8; 6],
    frame: &[u8],
    peers: &mut Vec<Peer>,
    received: &mut VecDeque<ReceivedMessage>,
) {
    let Ok((destination, source, seq, ack, payload_len, flags)) = parse_frame_header_checked(frame)
    else {
        return;
    };
    if destination != local_mac || source == local_mac {
        return;
    }
    let Some(index) = peer_index(peers, source) else {
        return;
    };
    if flags & FLAG_ACK != 0 {
        let peer = &mut peers[index];
        if peer.pending.as_ref().is_some_and(|pending| pending.seq == ack) {
            let pending = peer.pending.take().expect("pending checked above");
            ipc_reply(pending.reply, pending.payload_len as i64);
        }
    }

    if payload_len == 0 {
        return;
    }
    let payload_len = payload_len as usize;
    let payload = &frame[FRAME_HEADER_SIZE..FRAME_HEADER_SIZE + payload_len];
    let peer = &mut peers[index];
    if seq == peer.next_rx_seq {
        let cap = memory_alloc(1);
        if cap != 0 && memory_map(cap, PAYLOAD_SCRATCH, true) == 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    payload.as_ptr(),
                    PAYLOAD_SCRATCH as *mut u8,
                    payload_len,
                );
            }
            memory_unmap(cap);
            received.push_back(ReceivedMessage {
                source,
                cap,
                len: payload_len,
            });
            peer.next_rx_seq = peer.next_rx_seq.wrapping_add(1).max(1);
        } else if cap != 0 {
            memory_close(cap);
        }
    }

    // Cumulative acknowledgement: duplicates are acknowledged again, while
    // out-of-order frames acknowledge the last contiguous sequence.
    let last_contiguous = peer.next_rx_seq.wrapping_sub(1);
    let ack_frame = make_frame(source, local_mac, 0, last_contiguous, FLAG_ACK, &[]);
    let _ = send_frame(net_conn, &ack_frame);
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };

    let lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (generation, net_conn) = unsafe { wait_reply(lookup, REPLY_SPINS) };
    if generation < 1 || net_conn == 0 {
        unsafe { thread_exit() };
    }
    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    let (link, local_mac) = decode_status(status);
    if link == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 2);

    let ep = ipc_endpoint_create(relmsg::INTERFACE, relmsg::VERSION, 16);
    if ep == 0 {
        unsafe { thread_exit() };
    }
    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    let registration = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        relmsg::NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if registration == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { wait_reply(registration, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 3);

    let mut peers: Vec<Peer> = Vec::new();
    let mut received = VecDeque::new();
    let mut pending_recv = 0;
    config::write::<u32>(STAGE_OFFSET, 4);

    loop {
        config::write::<u32>(STAGE_OFFSET, 5);
        deliver_received(&mut received, &mut pending_recv);

        config::write::<u32>(STAGE_OFFSET, 6);
        let message = if peers.iter().any(|peer| peer.pending.is_some()) {
            ipc_recv(ep)
        } else {
            ipc_recv_block(ep)
        };
        config::write::<u32>(STAGE_OFFSET, 7);
        if message.status == ipc_status::NO_MESSAGE {
            let (_, timed_out) = cq_wait_timeout(1, relmsg::RETRANSMIT_MS, 0);
            if timed_out != 0 {
                retransmit_pending(net_conn, &mut peers);
            }
            continue;
        }
        if message.status == ipc_status::ENDPOINT_CLOSED {
            unsafe { thread_exit() };
        }
        if !message.is_ok() {
            continue;
        }
        if message.reply == 0 {
            if message.memory != 0 {
                memory_close(message.memory);
            }
            continue;
        }
        config::write::<u32>(4, message.opcode);
        let handled = unsafe { config::read::<u32>(8) };
        config::write::<u32>(8, handled.wrapping_add(1));

        match message.opcode {
            relmsg::OP_SEND => {
                let (destination, payload_len) = unpack_address_and_len(message.arg0);
                let payload_len = payload_len as usize;
                if message.memory == 0
                    || payload_len == 0
                    || payload_len > MAX_PAYLOAD_SIZE.min(relmsg::MAX_MSG)
                    || destination == local_mac
                {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                }
                let Some(index) = peer_index(&mut peers, destination) else {
                    memory_close(message.memory);
                    ipc_reply(message.reply, relmsg::ERR_PEER_UNREACHABLE);
                    continue;
                };
                if peers[index].pending.is_some() {
                    memory_close(message.memory);
                    ipc_reply(message.reply, relmsg::ERR_BUSY);
                    continue;
                }
                if memory_map(message.memory, PAYLOAD_SCRATCH, false) != 0 {
                    memory_close(message.memory);
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                }
                let payload = unsafe {
                    core::slice::from_raw_parts(PAYLOAD_SCRATCH as *const u8, payload_len)
                };
                let seq = peers[index].next_tx_seq;
                let frame = make_frame(destination, local_mac, seq, 0, 0, payload);
                memory_unmap(message.memory);
                memory_close(message.memory);
                config::write::<u32>(12, 1);
                if !send_frame(net_conn, &frame) {
                    config::write::<u32>(12, 0xe001);
                    ipc_reply(message.reply, relmsg::ERR_PEER_UNREACHABLE);
                    continue;
                }
                config::write::<u32>(12, 2);
                peers[index].next_tx_seq = seq.wrapping_add(1).max(1);
                peers[index].pending = Some(Outbound {
                    seq,
                    reply: message.reply,
                    payload_len,
                    frame,
                    retries: 0,
                });
            }
            relmsg::OP_RECV => {
                if pending_recv != 0 {
                    ipc_reply(message.reply, relmsg::ERR_BUSY);
                } else {
                    pending_recv = message.reply;
                    deliver_received(&mut received, &mut pending_recv);
                }
            }
            relmsg::OP_STATUS => {
                config::write::<u32>(STAGE_OFFSET, 8);
                ipc_reply(message.reply, pack_address_and_len(local_mac, 0) as i64);
            }
            relmsg::OP_FRAME => {
                let frame_len = message.arg0 as usize;
                if message.memory == 0 || frame_len > 4096 {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                }
                if memory_map(message.memory, RX_SCRATCH, false) == 0 {
                    let frame =
                        unsafe { core::slice::from_raw_parts(RX_SCRATCH as *const u8, frame_len) };
                    process_frame(net_conn, local_mac, frame, &mut peers, &mut received);
                    memory_unmap(message.memory);
                }
                memory_close(message.memory);
                deliver_received(&mut received, &mut pending_recv);
                ipc_reply(message.reply, 0);
            }
            relmsg::OP_SHUTDOWN => {
                ipc_reply(message.reply, 0);
                unsafe { thread_exit() };
            }
            _ => {
                ipc_reply(message.reply, relmsg::ERR_BAD_OPCODE);
            }
        }
    }
}

catten_rt::entry!(main);
