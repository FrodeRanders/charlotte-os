//! Reliable, sequenced messages over the node's raw Ethernet service.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    vec::Vec,
};
use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use catten_rt::{
    Context,
    ShutdownRequest,
    config,
};
use catten_services::{
    net,
    ns,
    relmsg,
    wait_for_registered_name_or_shutdown_owned,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_read,
    cq_wait_timeout,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_size,
    memory_unmap,
    submit_detached_timer,
    thread_exit,
};
use charlotte_launch::relmsg_status as status;
use charlotte_protocol_msg::{
    FLAG_ACK,
    FLAG_FRAG,
    FLAG_MORE,
    FLAG_SYN,
    FRAME_HEADER_SIZE,
    FrameHeader,
    IPC_MESSAGE_HEADER_SIZE,
    MAX_PAYLOAD_SIZE,
    MessageHeader,
    build_frame_header,
    build_ipc_message_header,
    initial_wire_session,
    next_wire_session,
    pack_mac,
    parse_frame_header_checked,
    parse_ipc_message_header,
    wire_session_is_newer,
};
use charlotte_protocol_net::decode_status;

const REPLY_SPINS: u64 = 50_000_000;
const MAX_RECEIVED_MESSAGES: usize = 32;
const MAX_REASSEMBLY_FRAGMENTS: usize = relmsg::MAX_MSG.div_ceil(MAX_PAYLOAD_SIZE);
const MAX_QUEUED_RECEIVE_BYTES: usize = 4 * relmsg::MAX_MSG;
const MAX_REASSEMBLY_BYTES: usize = 4 * relmsg::MAX_MSG;
const MAX_OUTBOUND_BYTES: usize = 4 * relmsg::MAX_MSG;
const RETRANSMIT_TIMER_COOKIE: u64 = 0x5245_4c4d_5347_544d;

static DIAG_HANDLED: AtomicU32 = AtomicU32::new(0);
static DIAG_RETRANSMITS: AtomicU32 = AtomicU32::new(0);
static DIAG_SEND_FAILURES: AtomicU32 = AtomicU32::new(0);
static DIAG_RECEIVED: AtomicU32 = AtomicU32::new(0);

struct ReceivedMessage {
    cap: u64,
    len: usize,
}

struct Outbound {
    seq: u32,
    reply: u64,
    payload_len: usize,
    /// One frame per fragment of the message (a single frame for messages
    /// that fit one Ethernet payload).
    frames: Vec<Vec<u8>>,
    retries: u32,
    ticks_until_retry: u32,
}

/// In-progress reassembly of a fragmented inbound message. Fragments may
/// arrive in any order; they are buffered by offset and assembled once the
/// last fragment (no `FLAG_MORE`) is present.
struct Reassembly {
    seq: u32,
    /// Fragment offset -> payload bytes, deduplicated by offset.
    fragments: BTreeMap<usize, Vec<u8>>,
    buffered_bytes: usize,
    /// Total message length advertised consistently by every fragment.
    total: usize,
}

/// Assemble the buffered fragments into one contiguous message, or `None` if
/// a fragment is still missing (a gap from offset 0 to `total`).
fn try_assemble(reassembly: &Reassembly) -> Option<Vec<u8>> {
    let total = reassembly.total;
    let mut out = Vec::with_capacity(total);
    let mut offset = 0usize;
    while offset < total {
        let bytes = reassembly.fragments.get(&offset)?;
        out.extend_from_slice(bytes);
        offset += bytes.len();
    }
    (offset == total).then_some(out)
}

struct Peer {
    mac: [u8; 6],
    /// Locally chosen wire-session identity for this peer. Abandoning an
    /// unacknowledged sequence starts a fresh session so the same sequence
    /// number can never denote two different messages.
    tx_session: u64,
    next_tx_seq: u32,
    next_rx_seq: u32,
    pending: Option<Outbound>,
    reassembling: Option<Reassembly>,
    remote_session: Option<u64>,
}

impl Peer {
    fn new(mac: [u8; 6], tx_session: u64) -> Self {
        Self {
            mac,
            tx_session,
            next_tx_seq: 1,
            next_rx_seq: 1,
            pending: None,
            reassembling: None,
            remote_session: None,
        }
    }

    fn accept_session(&mut self, session: u64, flags: u16) -> Option<bool> {
        if self.remote_session == Some(session) {
            return Some(false);
        }
        if flags & FLAG_SYN == 0
            || !wire_session_is_newer(session, self.remote_session.unwrap_or(0))
        {
            return None;
        }
        let restarted = self.remote_session.replace(session).is_some();
        self.next_rx_seq = 1;
        self.reassembling = None;
        Some(restarted)
    }

    /// Retire an uncertain transmit session after its retry lease expires.
    ///
    /// Reusing the abandoned sequence in the same session would be unsafe:
    /// the peer may have delivered the old message and only its ACK was lost.
    /// Advancing the packed retry epoch lets the peer reset its receive
    /// sequence, while monotonic acceptance rejects every delayed older
    /// session without a finite retirement window.
    fn abandon_transmit_session(&mut self) {
        // Zero is an explicit exhausted/disabled state. The send path rejects
        // it, so namespace exhaustion cannot silently reuse an old identity.
        self.tx_session = next_wire_session(self.tx_session).unwrap_or(0);
        self.next_tx_seq = 1;
    }
}

fn peer_index(peers: &mut Vec<Peer>, mac: [u8; 6], local_session: u64) -> Option<usize> {
    if let Some(index) = peers.iter().position(|peer| peer.mac == mac) {
        return Some(index);
    }
    if peers.len() >= relmsg::MAX_PEERS {
        return None;
    }
    peers.push(Peer::new(mac, local_session));
    Some(peers.len() - 1)
}

fn send_frame(net_conn: u64, frame: &[u8]) -> bool {
    if frame.len() > 4096 {
        DIAG_SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let cap = memory_alloc(1);
    if cap == 0 {
        DIAG_SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let (tx_scratch_map_status, tx_scratch_vaddr) = memory_map_any(cap, true);
    if tx_scratch_map_status != 0 {
        DIAG_SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
        memory_close(cap);
        return false;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), tx_scratch_vaddr as *mut u8, frame.len());
    }
    memory_unmap(cap);
    let call = ipc_scalar_call_move(net_conn, net::OP_SEND, frame.len() as u64, cap);
    if call == 0 {
        config::write::<u32>(status::LAST_SEND_RESULT, 1);
        DIAG_SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
        memory_close(cap);
        return false;
    }
    let (result, _) = unsafe { wait_reply(call, REPLY_SPINS) };
    config::write::<i64>(status::LAST_SEND_RESULT, result);
    if result != 0 {
        DIAG_SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    result == 0
}

fn make_frame(header_fields: FrameHeader, payload: &[u8]) -> Vec<u8> {
    let mut frame = alloc::vec![0u8; FRAME_HEADER_SIZE + payload.len()];
    let mut header = [0u8; FRAME_HEADER_SIZE];
    build_frame_header(&mut header, header_fields);
    frame[..FRAME_HEADER_SIZE].copy_from_slice(&header);
    frame[FRAME_HEADER_SIZE..].copy_from_slice(payload);
    frame
}

fn queue_received(queue: &mut VecDeque<ReceivedMessage>, source: [u8; 6], payload: &[u8]) -> bool {
    if queue.len() >= MAX_RECEIVED_MESSAGES
        || queue.iter().map(|message| message.len).sum::<usize>() + payload.len()
            > MAX_QUEUED_RECEIVE_BYTES
    {
        return false;
    }
    let object_len = IPC_MESSAGE_HEADER_SIZE + payload.len();
    let pages = object_len.div_ceil(4096).max(1);
    let cap = memory_alloc(pages);
    if cap == 0 {
        return false;
    }
    let (map_status, vaddr) = memory_map_any(cap, true);
    if map_status != 0 {
        memory_close(cap);
        return false;
    }
    let mut header = [0u8; IPC_MESSAGE_HEADER_SIZE];
    build_ipc_message_header(&mut header, source, payload.len() as u32);
    unsafe {
        core::ptr::copy_nonoverlapping(header.as_ptr(), vaddr as *mut u8, header.len());
        core::ptr::copy_nonoverlapping(
            payload.as_ptr(),
            (vaddr as *mut u8).add(IPC_MESSAGE_HEADER_SIZE),
            payload.len(),
        );
    }
    memory_unmap(cap);
    queue.push_back(ReceivedMessage {
        cap,
        len: payload.len(),
    });
    true
}

fn deliver_received(queue: &mut VecDeque<ReceivedMessage>, pending_recv: &mut u64) {
    if *pending_recv == 0 {
        return;
    }
    let Some(message) = queue.pop_front() else {
        return;
    };
    if ipc_reply_move(*pending_recv, message.cap, message.len as i64) != 0 {
        memory_close(message.cap);
    }
    *pending_recv = 0;
}

fn retransmit_pending(net_conn: u64, peers: &mut [Peer]) {
    for peer in peers {
        let Some(pending) = peer.pending.as_mut() else {
            continue;
        };
        if pending.ticks_until_retry > 1 {
            pending.ticks_until_retry -= 1;
            continue;
        }
        if pending.retries >= relmsg::MAX_RETRIES {
            let pending = peer.pending.take().expect("pending checked above");
            ipc_reply(pending.reply, relmsg::ERR_PEER_UNREACHABLE);
            peer.abandon_transmit_session();
            continue;
        }
        pending.retries += 1;
        pending.ticks_until_retry =
            (pending.frames.len() as u32).clamp(1, relmsg::MAX_RETRY_DELAY_TICKS);
        for frame in &pending.frames {
            let _ = send_frame(net_conn, frame);
            DIAG_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn process_frame(
    net_conn: u64,
    local_mac: [u8; 6],
    local_session: u64,
    frame: &[u8],
    peers: &mut Vec<Peer>,
    received: &mut VecDeque<ReceivedMessage>,
) {
    let Ok(header) = parse_frame_header_checked(frame) else {
        return;
    };
    let destination = header.destination;
    let source = header.source;
    let remote_session = header.message.session;
    let seq = header.message.sequence;
    let ack = header.message.acknowledgment;
    let frag_offset = header.message.fragment_offset;
    let total_message_len = header.message.total_message_len;
    let payload_len = header.message.payload_len;
    let flags = header.message.flags;
    if destination != local_mac || source == local_mac {
        return;
    }
    let Some(index) = peer_index(peers, source, local_session) else {
        return;
    };
    if flags & FLAG_ACK != 0 {
        // ACK-only frames carry the session being acknowledged, not a new
        // receive-side session belonging to the ACK sender. Fence completion
        // on both values: sequence numbers restart at one whenever a retry
        // lease advances `tx_session`, so sequence equality alone would let a
        // delayed ACK complete a call from the replacement session.
        if payload_len != 0 {
            return;
        }
        let peer = &mut peers[index];
        if remote_session == peer.tx_session
            && peer.pending.as_ref().is_some_and(|pending| pending.seq == ack)
        {
            let pending = peer.pending.take().expect("pending checked above");
            ipc_reply(pending.reply, pending.payload_len as i64);
        }
        return;
    }

    if peers[index].accept_session(remote_session, flags).is_none() || payload_len == 0 {
        return;
    }
    // The peer's receive session and our transmit session are independent.
    // A peer restart resets only receive-side ordering above. Resetting
    // `next_tx_seq` here would reuse a sequence number in our still-live
    // transmit session and could make a delayed ACK complete the wrong call.
    let payload_len = payload_len as usize;
    let total_message_len = total_message_len as usize;
    let payload = &frame[FRAME_HEADER_SIZE..FRAME_HEADER_SIZE + payload_len];
    let is_frag = flags & FLAG_FRAG != 0;
    let frag_offset = frag_offset as usize;

    if seq == peers[index].next_rx_seq {
        if !is_frag {
            // Single-frame message: deliver immediately.
            if queue_received(received, source, payload) {
                DIAG_RECEIVED.fetch_add(1, Ordering::Relaxed);
                peers[index].next_rx_seq = peers[index].next_rx_seq.wrapping_add(1).max(1);
            }
        } else {
            // Fragment of the expected message: buffer it by offset (any
            // arrival order) and assemble once the last fragment is present.
            let globally_buffered = peers
                .iter()
                .filter_map(|peer| peer.reassembling.as_ref())
                .map(|reassembly| reassembly.buffered_bytes)
                .sum::<usize>();
            if peers[index].reassembling.as_ref().is_none_or(|ra| ra.seq != seq) {
                peers[index].reassembling = Some(Reassembly {
                    seq,
                    fragments: BTreeMap::new(),
                    buffered_bytes: 0,
                    total: total_message_len,
                });
            }
            let peer = &mut peers[index];
            let within_ceiling = total_message_len <= relmsg::MAX_MSG
                && frag_offset.checked_add(payload_len).is_some_and(|end| end <= total_message_len);
            if within_ceiling && let Some(ra) = peer.reassembling.as_mut() {
                if ra.total != total_message_len {
                    peer.reassembling = None;
                    return;
                }
                let is_new = !ra.fragments.contains_key(&frag_offset);
                if is_new && ra.fragments.len() >= MAX_REASSEMBLY_FRAGMENTS {
                    return;
                }
                if is_new && globally_buffered.saturating_add(payload_len) > MAX_REASSEMBLY_BYTES {
                    return;
                }
                let replaced = ra.fragments.insert(frag_offset, payload.to_vec());
                ra.buffered_bytes = ra
                    .buffered_bytes
                    .saturating_sub(replaced.as_ref().map_or(0, Vec::len))
                    .saturating_add(payload_len);
                if ra.buffered_bytes > relmsg::MAX_MSG {
                    peer.reassembling = None;
                    return;
                }
            }
            // Assemble and deliver if every fragment is now present. On a
            // delivery failure, drop the reassembly state; the sender
            // retransmits and reassembly restarts (next_rx_seq did not
            // advance).
            let assembled = peer.reassembling.as_ref().and_then(try_assemble);
            if let Some(message_bytes) = assembled {
                if queue_received(received, source, &message_bytes) {
                    DIAG_RECEIVED.fetch_add(1, Ordering::Relaxed);
                    peer.next_rx_seq = peer.next_rx_seq.wrapping_add(1).max(1);
                }
                peer.reassembling = None;
            }
        }
    }

    // Do not emit a useless cumulative ACK for every incomplete fragment of
    // the expected message. The sender cannot complete until this sequence is
    // assembled and admitted to the receive queue; its fragment-count-scaled
    // retry grace handles a genuinely missing fragment without an ACK storm.
    if seq == peers[index].next_rx_seq {
        return;
    }

    // Cumulative acknowledgement: delivered duplicates are acknowledged
    // again, while future out-of-order messages acknowledge the last
    // contiguous sequence.
    let last_contiguous = peers[index].next_rx_seq.wrapping_sub(1);
    let ack_frame = make_frame(
        FrameHeader {
            destination: source,
            source: local_mac,
            message: MessageHeader {
                session: remote_session,
                sequence: 0,
                acknowledgment: last_contiguous,
                fragment_offset: 0,
                total_message_len: 0,
                payload_len: 0,
                flags: FLAG_SYN | FLAG_ACK,
            },
        },
        &[],
    );
    let _ = send_frame(net_conn, &ack_frame);
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);
    let names = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    let ns_conn = names.as_raw();

    let (_, net_connection) =
        match wait_for_registered_name_or_shutdown_owned(ctx, names, net::NAME) {
            Ok(found) => found,
            Err(request) => return request,
        };
    let net_conn = net_connection.as_raw();
    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    let (link, local_mac) = decode_status(status);
    if link == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 2);
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
    // Pack the service generation and retry epoch into disjoint namespaces.
    // Using the raw generation and incrementing it after an uncertain send
    // made generation N+1 collide with generation N's first retry session.
    let Some(local_session) = initial_wire_session(generation as u64) else {
        unsafe { thread_exit() };
    };
    config::write::<u32>(status::STAGE, 3);

    let mut peers: Vec<Peer> = Vec::new();
    let mut received: VecDeque<ReceivedMessage> = VecDeque::new();
    let mut pending_recv = 0;
    let cq = ctx.completion_queue_layout();
    let mut retransmit_timer_armed =
        submit_detached_timer(relmsg::RETRANSMIT_MS, 0, RETRANSMIT_TIMER_COOKIE) != u64::MAX;
    config::write::<u32>(status::STAGE, 4);

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            if pending_recv != 0 {
                ipc_reply(pending_recv, 0);
            }
            for message in received {
                memory_close(message.cap);
            }
            for peer in peers {
                if let Some(pending) = peer.pending {
                    ipc_reply(pending.reply, relmsg::ERR_PEER_UNREACHABLE);
                }
            }
            catten_rt::logln!("[relmsg] shutdown: pending receive and peer sessions released");
            return request;
        }
        // Retransmission cadence must be independent of endpoint traffic.
        // A timed CQ wait alone is insufficient: a busy peer can wake that
        // wait continuously, preventing its deadline from ever winning while
        // one of our own frames remains unacknowledged. The detached timer's
        // cookie is drained on every reactor iteration, including iterations
        // that already have an endpoint message to process.
        let mut retransmit_due = false;
        while let Some(completion) = unsafe { cq_read(cq.base, cq.entries) } {
            if completion.cookie == RETRANSMIT_TIMER_COOKIE {
                retransmit_due = true;
                retransmit_timer_armed = false;
            }
        }
        if retransmit_due {
            retransmit_pending(net_conn, &mut peers);
        }
        if !retransmit_timer_armed {
            retransmit_timer_armed =
                submit_detached_timer(relmsg::RETRANSMIT_MS, 0, RETRANSMIT_TIMER_COOKIE)
                    != u64::MAX;
        }

        config::write::<u32>(status::STAGE, 5);
        deliver_received(&mut received, &mut pending_recv);

        config::write::<u32>(status::STAGE, 6);
        let message = ipc_recv(ep);
        config::write::<u32>(status::STAGE, 7);
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
        config::write::<u32>(status::LAST_OPCODE, message.opcode);
        let handled = DIAG_HANDLED.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        config::write::<u32>(status::HANDLED, handled);

        match message.opcode {
            relmsg::OP_SEND => {
                if message.memory == 0 || message.arg0 != 0 {
                    if message.memory != 0 {
                        memory_close(message.memory);
                    }
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                }
                let capacity = memory_size(message.memory);
                if capacity < IPC_MESSAGE_HEADER_SIZE {
                    memory_close(message.memory);
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                }
                let (map_status, vaddr) = memory_map_any(message.memory, false);
                if map_status != 0 {
                    memory_close(message.memory);
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                }
                let object = unsafe { core::slice::from_raw_parts(vaddr as *const u8, capacity) };
                let Ok(envelope) = parse_ipc_message_header(object) else {
                    memory_unmap(message.memory);
                    memory_close(message.memory);
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                };
                let destination = envelope.peer;
                let payload_len = envelope.payload_len as usize;
                if payload_len == 0
                    || payload_len > relmsg::MAX_MSG
                    || destination == local_mac
                    || peers
                        .iter()
                        .filter_map(|peer| peer.pending.as_ref())
                        .map(|pending| pending.payload_len)
                        .sum::<usize>()
                        .saturating_add(payload_len)
                        > MAX_OUTBOUND_BYTES
                {
                    memory_unmap(message.memory);
                    memory_close(message.memory);
                    ipc_reply(message.reply, relmsg::ERR_UNKNOWN);
                    continue;
                }
                let payload =
                    object[IPC_MESSAGE_HEADER_SIZE..IPC_MESSAGE_HEADER_SIZE + payload_len].to_vec();
                memory_unmap(message.memory);
                memory_close(message.memory);
                let Some(index) = peer_index(&mut peers, destination, local_session) else {
                    ipc_reply(message.reply, relmsg::ERR_PEER_UNREACHABLE);
                    continue;
                };
                if peers[index].pending.is_some() {
                    ipc_reply(message.reply, relmsg::ERR_BUSY);
                    continue;
                }
                if peers[index].tx_session == 0 {
                    ipc_reply(message.reply, relmsg::ERR_PEER_UNREACHABLE);
                    continue;
                }
                let seq = peers[index].next_tx_seq;

                // Split the message into one frame per fragment (a single
                // frame when it fits). All fragments share the message seq;
                // each carries its byte offset and FLAG_FRAG (+ FLAG_MORE
                // except the last).
                let mut frames: Vec<Vec<u8>> = Vec::new();
                let mut offset = 0usize;
                while offset < payload_len {
                    let chunk_len = (payload_len - offset).min(MAX_PAYLOAD_SIZE);
                    let chunk = &payload[offset..offset + chunk_len];
                    let last = offset + chunk_len >= payload_len;
                    let fragmented = payload_len > MAX_PAYLOAD_SIZE;
                    let flags = if !fragmented {
                        FLAG_SYN
                    } else if last {
                        FLAG_SYN | FLAG_FRAG
                    } else {
                        FLAG_SYN | FLAG_FRAG | FLAG_MORE
                    };
                    let frame = make_frame(
                        FrameHeader {
                            destination,
                            source: local_mac,
                            message: MessageHeader {
                                session: peers[index].tx_session,
                                sequence: seq,
                                acknowledgment: 0,
                                fragment_offset: offset as u32,
                                total_message_len: payload_len as u32,
                                payload_len: chunk.len() as u16,
                                flags,
                            },
                        },
                        chunk,
                    );
                    frames.push(frame);
                    offset += chunk_len;
                }

                let mut send_ok = true;
                for frame in &frames {
                    if !send_frame(net_conn, frame) {
                        send_ok = false;
                        break;
                    }
                }
                if !send_ok {
                    config::write::<u32>(status::RECEIVER_STAGE, 0xe001);
                    ipc_reply(message.reply, relmsg::ERR_PEER_UNREACHABLE);
                    continue;
                }
                config::write::<u32>(status::RECEIVER_STAGE, 2);
                peers[index].next_tx_seq = seq.wrapping_add(1).max(1);
                peers[index].pending = Some(Outbound {
                    seq,
                    reply: message.reply,
                    payload_len,
                    ticks_until_retry: (frames.len() as u32)
                        .clamp(1, relmsg::MAX_RETRY_DELAY_TICKS),
                    frames,
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
                config::write::<u32>(status::STAGE, 8);
                ipc_reply(message.reply, pack_mac(local_mac) as i64);
            }
            relmsg::OP_DIAG => {
                // Move a page with the packed RelmsgDiag snapshot so the
                // httpd keyhole can render live transport counters.
                let cap = memory_alloc(1);
                if cap == 0 {
                    ipc_reply(message.reply, relmsg::ERR_BAD_OPCODE);
                    continue;
                }
                let (diag_map_status, diag_vaddr) = memory_map_any(cap, true);
                if diag_map_status != 0 {
                    memory_close(cap);
                    ipc_reply(message.reply, relmsg::ERR_BAD_OPCODE);
                    continue;
                }
                let in_flight = peers.iter().filter(|peer| peer.pending.is_some()).count() as u32;
                let words = [
                    relmsg::DIAG_MAGIC,
                    peers.len() as u32,
                    DIAG_HANDLED.load(Ordering::Relaxed),
                    DIAG_RETRANSMITS.load(Ordering::Relaxed),
                    DIAG_SEND_FAILURES.load(Ordering::Relaxed),
                    DIAG_RECEIVED.load(Ordering::Relaxed),
                    in_flight,
                ];
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        words.as_ptr(),
                        diag_vaddr as *mut u32,
                        words.len(),
                    );
                }
                memory_unmap(cap);
                ipc_reply_move(message.reply, cap, (words.len() * 4) as i64);
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
                let (rx_scratch_map_status, rx_scratch_vaddr) =
                    memory_map_any(message.memory, false);
                if rx_scratch_map_status == 0 {
                    let frame = unsafe {
                        core::slice::from_raw_parts(rx_scratch_vaddr as *const u8, frame_len)
                    };
                    process_frame(
                        net_conn,
                        local_mac,
                        local_session,
                        frame,
                        &mut peers,
                        &mut received,
                    );
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

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
