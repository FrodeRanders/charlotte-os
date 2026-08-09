//! The CharlotteOS TCP/IP service (smoltcp-powered).
//!
//! Bootstraps, looks up the NIC driver ("net0"), initialises a smoltcp
//! interface, registers a "tcpip" endpoint with the name service, and
//! enters a poll loop that handles both NIC frames and socket-API
//! client requests.
//!
//! The receive path is driven by the frouter: it owns the NIC's deferred
//! `OP_RECV` slot and forwards IPv4/ARP frames here through `OP_FRAME`.
//! Each forwarded frame is copied into the smoltcp device's receive queue;
//! `OP_SEND` is used directly for transmit (the driver's send path is
//! multi-consumer).
//!
//! ## Socket protocol
//!
//! Clients call `OP_SOCKET`, `OP_CONNECT`, `OP_BIND`/`OP_LISTEN`,
//! `OP_ACCEPT`, `OP_SEND`, `OP_RECV` (deferred reply), and `OP_CLOSE` on the
//! tcpip connection. Data payloads use memory-object transfer. See
//! [`catten_services::socket`].
//!
//! ## Launch manifest
//!
//! - `ip`: optional local IPv4 address as four bytes. Defaults to a MAC-derived `10.0.0.(100 +
//!   mac[5] % 100)`; override with `10.0.2.15` (plus `gateway`) when the guest sits on a SLIRP user
//!   network.
//! - `gateway`: optional IPv4 default-route gateway as four bytes. Omit on a raw two-node link:
//!   same-subnet peers are reached directly.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;

use catten_rt::{
    Context,
    ManifestValue,
    config,
};
use catten_services::{
    net,
    ns,
    socket,
    wait_for_local_ready,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait_timeout,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_net::decode_status;
use charlotte_smoltcp::CharlotteEthDevice;
use smoltcp::{
    iface::{
        Config,
        Interface,
        SocketSet,
    },
    socket::tcp::{
        Socket as TcpSocket,
        SocketBuffer as TcpSocketBuffer,
    },
    time::Instant,
    wire::{
        HardwareAddress,
        IpAddress,
        IpCidr,
        IpEndpoint,
        Ipv4Address,
        Ipv4Cidr,
    },
};

const REPLY_SPINS: u64 = 50_000_000;
const STAGE_OFFSET: usize = 0;
const RX_TOTAL_OFFSET: usize = 4;
const TX_OK_OFFSET: usize = 8;
const SOCKETS_OFFSET: usize = 12;
const POLL_MS: u64 = 50;
const FRAME_MAX: usize = 4096;

struct SocketEntry {
    handle: smoltcp::iface::SocketHandle,
    recv_pending: Option<u64>,
    /// Set by `OP_CLOSE`: the socket was gracefully closed and may be swept
    /// from the set once it reaches a final state.
    closing: bool,
}

struct TcpipState {
    sockets: BTreeMap<u64, SocketEntry>,
    next_sock_id: u64,
    next_ephemeral: u16,
}

impl TcpipState {
    fn alloc_sock_id(&mut self) -> u64 {
        let id = self.next_sock_id;
        self.next_sock_id = id.wrapping_add(1);
        if self.next_sock_id == 0 {
            self.next_sock_id = 1;
        }
        id
    }

    /// Allocate a local ephemeral port for an outgoing connection. The
    /// default ephemeral range is 49152..=65535.
    fn alloc_ephemeral_port(&mut self) -> u16 {
        let port = self.next_ephemeral;
        self.next_ephemeral = if port == u16::MAX {
            49152
        } else {
            port + 1
        };
        port
    }
}

/// Read a little-endian u16 payload (e.g. a port) from a moved memory object.
fn read_port(memory: u64) -> u16 {
    let (scratch_vaddr_6_map_status, scratch_vaddr_6_vaddr) = memory_map_any(memory, false);
    if scratch_vaddr_6_map_status != 0 {
        return 0;
    }
    let port = unsafe { core::ptr::read_unaligned(scratch_vaddr_6_vaddr as *const u16) };
    memory_unmap(memory);
    port
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_connection = match ctx.bootstrap_cap() {
        Some(c) => c,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2);

    let lookup = ipc_scalar_call(ns_connection, ns::OP_LOOKUP, net::NAME);
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
    let (_link, mac) = decode_status(status);
    let mtu: usize = 1500;

    let default_ip = Ipv4Address::new(10, 0, 0, 100u8.wrapping_add(mac[5] % 100));
    let local_ip = match ctx.manifest_value(charlotte_launch::manifest_key(b"ip")) {
        Some(ManifestValue::Bytes(raw)) if raw.len() == 4 => {
            Ipv4Address::new(raw[0], raw[1], raw[2], raw[3])
        }
        _ => default_ip,
    };
    let gateway = match ctx.manifest_value(charlotte_launch::manifest_key(b"gateway")) {
        Some(ManifestValue::Bytes(raw)) if raw.len() == 4 => {
            Some(Ipv4Address::new(raw[0], raw[1], raw[2], raw[3]))
        }
        _ => None,
    };
    config::write::<u32>(STAGE_OFFSET, 3);

    let ep = ipc_endpoint_create(socket::INTERFACE, socket::VERSION, 8);
    if ep == 0 {
        unsafe { thread_exit() };
    }
    let reg = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        socket::NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
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
    config::write::<u32>(STAGE_OFFSET, 4);

    // Let the NIC and the link settle before ARP/IP traffic starts flowing.
    if !wait_for_local_ready(ns_connection) {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 5);

    let mut device = CharlotteEthDevice::new(net_conn, mtu);
    let hw = HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress(mac));
    let mut cfg = Config::new(hw);
    cfg.random_seed = 0x0123_4567_89ab_cdef;
    let mut iface = Interface::new(cfg, &mut device, Instant::from_millis(0));
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(local_ip, 24)));
    });
    if let Some(gw) = gateway {
        iface.routes_mut().add_default_ipv4_route(gw).ok();
    }
    let mut sock_storage: [_; 16] = Default::default();
    let mut sockets = SocketSet::new(&mut sock_storage[..]);
    let mut state = TcpipState {
        sockets: BTreeMap::new(),
        next_sock_id: 1,
        next_ephemeral: 49152,
    };
    let mut ticks: u64 = 0;
    let mut elapsed_ms: u64 = 1;
    let mut rx_total: u32 = 0;
    let mut tx_ok: u32 = 0;
    config::write::<u32>(STAGE_OFFSET, 6);

    loop {
        device.poll_smoltcp(&mut iface, &mut sockets, &mut ticks, elapsed_ms);

        // Sweep sockets that have fully closed (graceful close finished) so
        // their handles are recycled.
        let mut closing: [u64; 8] = [0; 8];
        let mut closing_n: usize = 0;
        for (id, entry) in state.sockets.iter() {
            if entry.closing && !sockets.get::<TcpSocket>(entry.handle).is_open() && closing_n < 8 {
                closing[closing_n] = *id;
                closing_n += 1;
            }
        }
        for id in closing.iter().take(closing_n) {
            if let Some(entry) = state.sockets.remove(id) {
                sockets.remove(entry.handle);
            }
        }
        config::write::<u32>(SOCKETS_OFFSET, state.sockets.len() as u32);

        // Complete any ready recv operations.
        let mut completed: [u64; 8] = [0; 8];
        let mut completed_n: usize = 0;
        for (id, entry) in state.sockets.iter() {
            if let Some(reply_token) = entry.recv_pending {
                let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                if sock.can_recv() {
                    let cap = memory_alloc(1);
                    if cap == 0 {
                        continue;
                    }
                    let (scratch_vaddr_5_map_status, scratch_vaddr_5_vaddr) =
                        memory_map_any(cap, true);
                    if scratch_vaddr_5_map_status != 0 {
                        memory_close(cap);
                        continue;
                    }
                    let buf = unsafe {
                        core::slice::from_raw_parts_mut(scratch_vaddr_5_vaddr as *mut u8, 4096)
                    };
                    match sock.recv_slice(buf) {
                        Ok(0) => {
                            memory_unmap(cap);
                            memory_close(cap);
                        }
                        Ok(len) => {
                            memory_unmap(cap);
                            ipc_reply_move(reply_token, cap, len as i64);
                            if completed_n < 8 {
                                completed[completed_n] = *id;
                                completed_n += 1;
                            }
                        }
                        Err(_) => {
                            memory_unmap(cap);
                            memory_close(cap);
                        }
                    }
                }
            }
        }
        for id in completed.iter().take(completed_n) {
            if let Some(entry) = state.sockets.get_mut(id) {
                entry.recv_pending = None;
            }
        }

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
                socket::OP_SOCKET => {
                    if msg.arg0 != socket::DOMAIN_TCP {
                        ipc_reply(msg.reply, socket::ERR_BAD_DOMAIN);
                        continue;
                    }
                    if state.sockets.len() >= socket::MAX_SOCKETS {
                        ipc_reply(msg.reply, socket::ERR_TOO_MANY_SOCKETS);
                        continue;
                    }
                    let rx = TcpSocketBuffer::new(alloc::vec![0u8; 4096]);
                    let tx = TcpSocketBuffer::new(alloc::vec![0u8; 4096]);
                    let tcp = TcpSocket::new(rx, tx);
                    let handle = sockets.add(tcp);
                    let id = state.alloc_sock_id();
                    state.sockets.insert(
                        id,
                        SocketEntry {
                            handle,
                            recv_pending: None,
                            closing: false,
                        },
                    );
                    config::write::<u32>(SOCKETS_OFFSET, state.sockets.len() as u32);
                    ipc_reply(msg.reply, id as i64);
                }

                socket::OP_CONNECT => {
                    if msg.memory == 0 {
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
                    let (_scratch_vaddr_4_map_status, scratch_vaddr_4_vaddr) =
                        memory_map_any(msg.memory, false);
                    let a = unsafe { core::ptr::read_volatile(scratch_vaddr_4_vaddr as *const u8) };
                    let b = unsafe {
                        core::ptr::read_volatile((scratch_vaddr_4_vaddr + 1) as *const u8)
                    };
                    let c = unsafe {
                        core::ptr::read_volatile((scratch_vaddr_4_vaddr + 2) as *const u8)
                    };
                    let d = unsafe {
                        core::ptr::read_volatile((scratch_vaddr_4_vaddr + 3) as *const u8)
                    };
                    let port = unsafe {
                        core::ptr::read_unaligned((scratch_vaddr_4_vaddr + 4) as *const u16)
                    };
                    memory_unmap(msg.memory);
                    memory_close(msg.memory);
                    let remote =
                        IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::new(a, b, c, d)), port);
                    let local_port = state.alloc_ephemeral_port();
                    let entry = match state.sockets.get_mut(&msg.arg0) {
                        Some(e) => e,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    let local = smoltcp::wire::IpListenEndpoint {
                        addr: None,
                        port: local_port,
                    };
                    let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                    match sock.connect(iface.context(), remote, local) {
                        Ok(()) => ipc_reply(msg.reply, 0),
                        Err(_) => ipc_reply(msg.reply, socket::ERR_CONNECTION_REFUSED),
                    };
                }

                socket::OP_BIND => {
                    let entry = match state.sockets.get_mut(&msg.arg0) {
                        Some(e) => e,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    if msg.memory == 0 {
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
                    let port = read_port(msg.memory);
                    memory_close(msg.memory);
                    let listen = smoltcp::wire::IpListenEndpoint {
                        addr: None,
                        port,
                    };
                    let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                    match sock.listen(listen) {
                        Ok(()) => ipc_reply(msg.reply, 0),
                        Err(_) => ipc_reply(msg.reply, socket::ERR_BAD_SOCKET),
                    };
                }

                socket::OP_LISTEN => {
                    let entry = match state.sockets.get_mut(&msg.arg0) {
                        Some(e) => e,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    if msg.memory == 0 {
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
                    let port = read_port(msg.memory);
                    memory_close(msg.memory);
                    let listen = smoltcp::wire::IpListenEndpoint {
                        addr: None,
                        port,
                    };
                    let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                    match sock.listen(listen) {
                        Ok(()) => ipc_reply(msg.reply, 0),
                        Err(_) => ipc_reply(msg.reply, socket::ERR_BAD_SOCKET),
                    };
                }

                socket::OP_ACCEPT => {
                    let entry = match state.sockets.get_mut(&msg.arg0) {
                        Some(e) => e,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    // smoltcp 0.13 transitions the listening socket itself
                    // into the established connection, so "accept" succeeds
                    // once the listener is no longer listening.
                    let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                    if sock.is_listening() {
                        ipc_reply(msg.reply, socket::ERR_WOULD_BLOCK);
                    } else if sock.is_open() {
                        ipc_reply(msg.reply, 0);
                    } else {
                        ipc_reply(msg.reply, socket::ERR_CONNECTION_REFUSED);
                    }
                }

                socket::OP_SEND => {
                    // arg0 packs the socket id (low 32 bits) and the payload
                    // length (high 32 bits); the memory object is one page.
                    let sock_id = (msg.arg0 & 0xffff_ffff) as u64;
                    let payload_len = (msg.arg0 >> 32) as usize;
                    let entry = match state.sockets.get_mut(&sock_id) {
                        Some(e) => e,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    if msg.memory == 0 {
                        ipc_reply(msg.reply, 0);
                        continue;
                    }
                    if !(1..=4096).contains(&payload_len) {
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
                    let (_scratch_vaddr_3_map_status, scratch_vaddr_3_vaddr) =
                        memory_map_any(msg.memory, false);
                    let data = unsafe {
                        core::slice::from_raw_parts(scratch_vaddr_3_vaddr as *const u8, payload_len)
                    };
                    let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                    let result = match sock.send_slice(data) {
                        Ok(len) => {
                            if len > 0 {
                                tx_ok = tx_ok.wrapping_add(1);
                                config::write::<u32>(TX_OK_OFFSET, tx_ok);
                            }
                            len as i64
                        }
                        Err(_) => socket::ERR_WOULD_BLOCK,
                    };
                    memory_unmap(msg.memory);
                    memory_close(msg.memory);
                    ipc_reply(msg.reply, result);
                }

                socket::OP_RECV => {
                    let entry = match state.sockets.get_mut(&msg.arg0) {
                        Some(e) => e,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    if entry.recv_pending.is_some() {
                        ipc_reply(msg.reply, socket::ERR_WOULD_BLOCK);
                    } else {
                        entry.recv_pending = Some(msg.reply);
                    }
                }

                socket::OP_CLOSE => {
                    // Graceful close: transition to FIN-WAIT so queued
                    // transmit data (e.g. an httpd response) drains before the
                    // FIN; the reactor sweeps the socket once fully closed.
                    if let Some(entry) = state.sockets.get_mut(&msg.arg0) {
                        if let Some(token) = entry.recv_pending.take() {
                            ipc_reply(token, 0);
                        }
                        entry.closing = true;
                        let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                        sock.close();
                    }
                    config::write::<u32>(SOCKETS_OFFSET, state.sockets.len() as u32);
                    ipc_reply(msg.reply, 0);
                }

                socket::OP_FRAME => {
                    let frame_len = msg.arg0 as usize;
                    if msg.memory == 0 || !(14..=FRAME_MAX).contains(&frame_len) {
                        if msg.memory != 0 {
                            memory_close(msg.memory);
                        }
                        ipc_reply(msg.reply, -1);
                        continue;
                    }
                    let (scratch_vaddr_2_map_status, scratch_vaddr_2_vaddr) =
                        memory_map_any(msg.memory, false);
                    if scratch_vaddr_2_map_status == 0 {
                        let frame = unsafe {
                            core::slice::from_raw_parts(
                                scratch_vaddr_2_vaddr as *const u8,
                                frame_len,
                            )
                        };
                        device.push_rx(frame.to_vec());
                        memory_unmap(msg.memory);
                    }
                    memory_close(msg.memory);
                    rx_total = rx_total.wrapping_add(1);
                    config::write::<u32>(RX_TOTAL_OFFSET, rx_total);
                    ipc_reply(msg.reply, 0);
                }

                socket::OP_STATUS => {
                    // Move a page with the packed TcpipStatus snapshot so the
                    // httpd keyhole can render live service counters.
                    let cap = memory_alloc(1);
                    if cap == 0 {
                        ipc_reply(msg.reply, socket::ERR_WOULD_BLOCK);
                        continue;
                    }
                    let (scratch_vaddr_map_status, scratch_vaddr_vaddr) = memory_map_any(cap, true);
                    if scratch_vaddr_map_status != 0 {
                        memory_close(cap);
                        ipc_reply(msg.reply, socket::ERR_WOULD_BLOCK);
                        continue;
                    }
                    let octets = local_ip.octets();
                    let words = [
                        u32::from_be_bytes([octets[0], octets[1], octets[2], octets[3]]),
                        rx_total,
                        tx_ok,
                        state.sockets.len() as u32,
                        socket::STATUS_MAGIC,
                    ];
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            words.as_ptr(),
                            scratch_vaddr_vaddr as *mut u32,
                            words.len(),
                        );
                    }
                    memory_unmap(cap);
                    ipc_reply_move(msg.reply, cap, (words.len() * 4) as i64);
                }

                _ => {
                    ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                }
            }
        }

        let (_, timed_out) = cq_wait_timeout(1, POLL_MS, 0);
        elapsed_ms = if timed_out != 0 {
            POLL_MS
        } else {
            1
        };
    }
}

catten_rt::entry!(main);
