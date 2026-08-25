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
//! - `dhcp`: when present, skip the static address and acquire the interface configuration
//!   (address, prefix, gateway, DNS servers) from a DHCP server. Use this on a network with a DHCP
//!   server (e.g. the QEMU SLIRP user network).
//! - `ip`: optional local IPv4 address as four bytes. Defaults to a MAC-derived `10.0.0.(100 +
//!   mac[5] % 100)`; override with `10.0.2.15` (plus `gateway`) when the guest sits on a SLIRP user
//!   network.
//! - `gateway`: optional IPv4 default-route gateway as four bytes. Omit on a raw two-node link:
//!   same-subnet peers are reached directly.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    vec,
};

use catten_rt::{
    Context,
    ManifestValue,
    config,
};
use catten_services::{
    net,
    ns,
    scalar_call_with_backpressure,
    socket,
    wait_for_local_ready,
    wait_for_registered_name,
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
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    submit_detached_timer,
    thread_exit,
};
use charlotte_launch::tcpip_status as status;
use charlotte_protocol_net::decode_status;
use charlotte_smoltcp::CharlotteEthDevice;
use smoltcp::{
    iface::{
        Config,
        Interface,
        SocketSet,
    },
    socket::{
        dhcpv4,
        tcp::{
            Socket as TcpSocket,
            SocketBuffer as TcpSocketBuffer,
        },
        udp::{
            PacketBuffer as UdpPacketBuffer,
            PacketMetadata as UdpPacketMetadata,
            Socket as UdpSocket,
        },
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
const FRAME_MAX: usize = 4096;
/// Detached-timer cadence for the smoltcp clock. A continuously IPC-woken
/// reactor must not collapse the timebase to a fixed 1 ms per iteration; the
/// timer fires independently of endpoint traffic and re-arms each cycle.
/// Kept at 100 ms (matching the discovery service) rather than 10 ms: smoltcp's
/// timers (delayed ACK, RTO) tolerate the coarser granularity, and the lower
/// re-arm rate avoids interacting with the 10 ms scheduler quantum on LP 0.
const CLOCK_TICK_MS: u64 = 100;
const CLOCK_TIMER_COOKIE: u64 = 0x5443_5049_434c_4b31;
/// Per-socket buffer size. The httpd report exceeds one 4096-byte page, so a
/// single-page buffer forces the sender to stall mid-stream while the peer
/// drains it; a larger buffer lets a full report be accepted without blocking.
const SOCKET_BUF: usize = 16 * 1024;

/// Monotonic reactor-tick counter for periodic heartbeat logging.
static HEARTBEAT_TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

struct SocketEntry {
    handle: smoltcp::iface::SocketHandle,
    kind: SocketKind,
    /// UDP has no connected state in smoltcp. The service remembers the peer
    /// selected by `OP_CONNECT` and filters received datagrams to it.
    udp_remote: Option<IpEndpoint>,
    recv_pending: Option<u64>,
    /// Set by `OP_CLOSE`: the socket was gracefully closed and may be swept
    /// from the set once it reaches a final state.
    closing: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SocketKind {
    Tcp,
    Udp,
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

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    catten_syscall::el0_log(0x5443_5049, code as u64); // "TCPI"
    unsafe { thread_exit() }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns_connection = match ctx.bootstrap_cap() {
        Some(c) => c,
        None => fail(0xe001),
    };
    config::write::<u32>(status::STAGE, 2);

    config::write::<u32>(status::DETAIL, 1);
    let (_, net_conn) =
        wait_for_registered_name(ns_connection, net::NAME).unwrap_or_else(|| fail(0xe002));
    config::write::<u32>(status::DETAIL, 2);

    let status_call = scalar_call_with_backpressure(net_conn, net::OP_STATUS, 0);
    config::write::<u32>(status::DETAIL, 3);
    let (nic_status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    config::write::<u32>(status::DETAIL, 4);
    if nic_status < 0 {
        fail(0xe003);
    }
    let (_link, mac) = decode_status(nic_status);
    let mtu: usize = 1500;

    catten_rt::logln!(
        "[tcpip] NIC MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    );

    // DHCP mode: when the `dhcp` manifest key is present, skip the static (or
    // MAC-derived) address and acquire the interface configuration from a DHCP
    // server instead. The static path remains the default for raw two-node
    // links and SLIRP runs with an explicit address.
    let dhcp = ctx.manifest_value(charlotte_launch::manifest_key(b"dhcp")).is_some();

    let default_ip = Ipv4Address::new(10, 0, 0, 100u8.wrapping_add(mac[5] % 100));
    let mut local_ip = if dhcp {
        Ipv4Address::new(0, 0, 0, 0)
    } else {
        match ctx.manifest_value(charlotte_launch::manifest_key(b"ip")) {
            Some(ManifestValue::Bytes(raw)) if raw.len() == 4 => {
                Ipv4Address::new(raw[0], raw[1], raw[2], raw[3])
            }
            _ => default_ip,
        }
    };
    let gateway = match ctx.manifest_value(charlotte_launch::manifest_key(b"gateway")) {
        Some(ManifestValue::Bytes(raw)) if raw.len() == 4 => {
            Some(Ipv4Address::new(raw[0], raw[1], raw[2], raw[3]))
        }
        _ => None,
    };
    config::write::<u32>(status::STAGE, 3);

    let ep = ipc_endpoint_create(socket::INTERFACE, socket::VERSION, 8);
    if ep == 0 {
        fail(0xe004);
    }
    let reg = loop {
        let call = ipc_scalar_call_connection(
            ns_connection,
            ns::OP_REGISTER,
            socket::NAME,
            ep,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        );
        if call != 0 {
            break call;
        }
        // The name-service queue is shared by the booting service set. Yield
        // through a timer when it is temporarily full; the endpoint and
        // delegated connection remain owned by this process and are safe to
        // submit again.
        catten_services::sleep_ms(1);
    };
    let (generation, _) = unsafe { wait_reply(reg, REPLY_SPINS) };
    if generation < 1 {
        fail(0xe005);
    }
    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        fail(0xe006);
    }
    config::write::<u32>(status::STAGE, 4);

    // Let the NIC and the link settle before ARP/IP traffic starts flowing.
    if !wait_for_local_ready(ns_connection) {
        fail(0xe007);
    }
    config::write::<u32>(status::STAGE, 5);

    let mut device = CharlotteEthDevice::new(net_conn, mtu);
    let hw = HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress(mac));
    let mut cfg = Config::new(hw);
    cfg.random_seed = 0x0123_4567_89ab_cdef;
    let mut iface = Interface::new(cfg, &mut device, Instant::from_millis(0));
    if !dhcp {
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(local_ip, 24)));
        });
        if let Some(gw) = gateway {
            iface.routes_mut().add_default_ipv4_route(gw).ok();
        }
    }
    let mut sock_storage: [_; 16] = Default::default();
    let mut sockets = SocketSet::new(&mut sock_storage[..]);
    let dhcp_handle = if dhcp {
        Some(sockets.add(dhcpv4::Socket::new()))
    } else {
        None
    };
    let mut state = TcpipState {
        sockets: BTreeMap::new(),
        next_sock_id: 1,
        next_ephemeral: 49152,
    };
    let mut ticks: u64 = 0;
    let mut elapsed_ms: u64 = 1;
    let mut rx_total: u32 = 0;
    let mut tx_ok: u32 = 0;
    let mut tx_err: u32 = 0;
    let dhcp_mode: u32 = if dhcp {
        1
    } else {
        0
    };
    let gateway_ip: u32 = gateway.map_or(0, |gw| {
        let octets = gw.octets();
        u32::from_be_bytes([octets[0], octets[1], octets[2], octets[3]])
    });
    config::write::<u32>(status::STAGE, 6);

    // Arm a detached timer as the smoltcp timebase. Reading its cookie from
    // the completion queue gives a real elapsed time that is independent of
    // how often endpoint traffic wakes the bounded CQ wait.
    let cq = ctx.completion_queue_layout();
    let mut clock_armed = submit_detached_timer(CLOCK_TICK_MS, 0, CLOCK_TIMER_COOKIE) != u64::MAX;

    loop {
        device.poll_smoltcp(&mut iface, &mut sockets, &mut ticks, elapsed_ms);

        // Periodic heartbeat (~every 1024 reactor iterations) so a stall can be
        // localized: if rx_total stops advancing here, forwarded frames (e.g.
        // ACKs) are not reaching the stack from the frouter.
        let tick = HEARTBEAT_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if tick & 0x3ff == 0 {
            catten_rt::logln!(
                "[tcpip] hb rx={} tx_ok={} tx_err={} sockets={}",
                rx_total,
                tx_ok,
                tx_err,
                state.sockets.len()
            );
        }

        // Apply DHCP configuration changes to the interface. The DHCP socket
        // reports `Configured` on a fresh/renewed lease and `Deconfigured` on
        // lease expiry; copy the Copy-able fields out before touching `iface`.
        if let Some(handle) = dhcp_handle {
            enum DhcpUpdate {
                None,
                Configured {
                    cidr: Ipv4Cidr,
                    router: Option<Ipv4Address>,
                },
                Deconfigured,
            }
            let update = match sockets.get_mut::<dhcpv4::Socket>(handle).poll() {
                None => DhcpUpdate::None,
                Some(dhcpv4::Event::Configured(config)) => DhcpUpdate::Configured {
                    cidr: config.address,
                    router: config.router,
                },
                Some(dhcpv4::Event::Deconfigured) => DhcpUpdate::Deconfigured,
            };
            match update {
                DhcpUpdate::None => {}
                DhcpUpdate::Configured {
                    cidr,
                    router,
                } => {
                    local_ip = cidr.address();
                    let octets = local_ip.octets();
                    catten_rt::logln!(
                        "[tcpip] DHCP assigned {}.{}.{}.{}/{}",
                        octets[0],
                        octets[1],
                        octets[2],
                        octets[3],
                        cidr.prefix_len()
                    );
                    iface.update_ip_addrs(|addrs| {
                        addrs.clear();
                        let _ = addrs.push(IpCidr::Ipv4(cidr));
                    });
                    match router {
                        Some(r) => {
                            let _ = iface.routes_mut().add_default_ipv4_route(r);
                        }
                        None => {
                            iface.routes_mut().remove_default_ipv4_route();
                        }
                    }
                }
                DhcpUpdate::Deconfigured => {
                    local_ip = Ipv4Address::new(0, 0, 0, 0);
                    iface.update_ip_addrs(|addrs| addrs.clear());
                    iface.routes_mut().remove_default_ipv4_route();
                }
            }
        }

        // Sweep sockets that have fully closed (graceful close finished) so
        // their handles are recycled.
        let mut closing: [u64; 8] = [0; 8];
        let mut closing_n: usize = 0;
        for (id, entry) in state.sockets.iter() {
            let closed = match entry.kind {
                SocketKind::Tcp => !sockets.get::<TcpSocket>(entry.handle).is_open(),
                SocketKind::Udp => !sockets.get::<UdpSocket>(entry.handle).is_open(),
            };
            if entry.closing && closed && closing_n < 8 {
                closing[closing_n] = *id;
                closing_n += 1;
            }
        }
        for id in closing.iter().take(closing_n) {
            if let Some(entry) = state.sockets.remove(id) {
                sockets.remove(entry.handle);
            }
        }
        config::write::<u32>(status::SOCKETS, state.sockets.len() as u32);

        // Complete any ready recv operations.
        let mut completed: [u64; 8] = [0; 8];
        let mut completed_n: usize = 0;
        for (id, entry) in state.sockets.iter() {
            if let Some(reply_token) = entry.recv_pending {
                let can_recv = match entry.kind {
                    SocketKind::Tcp => sockets.get::<TcpSocket>(entry.handle).can_recv(),
                    SocketKind::Udp => sockets.get::<UdpSocket>(entry.handle).can_recv(),
                };
                if can_recv {
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
                    let received = match entry.kind {
                        SocketKind::Tcp => sockets
                            .get_mut::<TcpSocket>(entry.handle)
                            .recv_slice(buf)
                            .ok()
                            .map(|len| (len, true)),
                        SocketKind::Udp => {
                            sockets.get_mut::<UdpSocket>(entry.handle).recv_slice(buf).ok().map(
                                |(len, metadata)| {
                                    (len, entry.udp_remote == Some(metadata.endpoint))
                                },
                            )
                        }
                    };
                    match received {
                        Some((len, true)) if len > 0 => {
                            memory_unmap(cap);
                            ipc_reply_move(reply_token, cap, len as i64);
                            if completed_n < 8 {
                                completed[completed_n] = *id;
                                completed_n += 1;
                            }
                        }
                        _ => {
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
                    if msg.arg0 != socket::DOMAIN_TCP && msg.arg0 != socket::DOMAIN_UDP {
                        ipc_reply(msg.reply, socket::ERR_BAD_DOMAIN);
                        continue;
                    }
                    if state.sockets.len() >= socket::MAX_SOCKETS {
                        ipc_reply(msg.reply, socket::ERR_TOO_MANY_SOCKETS);
                        continue;
                    }
                    let (handle, kind) = if msg.arg0 == socket::DOMAIN_TCP {
                        let rx = TcpSocketBuffer::new(vec![0u8; SOCKET_BUF]);
                        let tx = TcpSocketBuffer::new(vec![0u8; SOCKET_BUF]);
                        (sockets.add(TcpSocket::new(rx, tx)), SocketKind::Tcp)
                    } else {
                        let rx = UdpPacketBuffer::new(
                            vec![UdpPacketMetadata::EMPTY; 4],
                            vec![0u8; 4096],
                        );
                        let tx = UdpPacketBuffer::new(
                            vec![UdpPacketMetadata::EMPTY; 4],
                            vec![0u8; 4096],
                        );
                        (sockets.add(UdpSocket::new(rx, tx)), SocketKind::Udp)
                    };
                    let id = state.alloc_sock_id();
                    state.sockets.insert(
                        id,
                        SocketEntry {
                            handle,
                            kind,
                            udp_remote: None,
                            recv_pending: None,
                            closing: false,
                        },
                    );
                    config::write::<u32>(status::SOCKETS, state.sockets.len() as u32);
                    ipc_reply(msg.reply, id as i64);
                }

                socket::OP_CONNECT => {
                    if msg.memory == 0 {
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
                    let (scratch_vaddr_4_map_status, scratch_vaddr_4_vaddr) =
                        memory_map_any(msg.memory, false);
                    if scratch_vaddr_4_map_status != 0 {
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
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
                    let connected = match entry.kind {
                        SocketKind::Tcp => sockets
                            .get_mut::<TcpSocket>(entry.handle)
                            .connect(iface.context(), remote, local)
                            .is_ok(),
                        SocketKind::Udp => {
                            let sock = sockets.get_mut::<UdpSocket>(entry.handle);
                            let bound = sock.is_open() || sock.bind(local).is_ok();
                            if bound {
                                entry.udp_remote = Some(remote);
                            }
                            bound
                        }
                    };
                    if connected {
                        ipc_reply(msg.reply, 0);
                    } else {
                        ipc_reply(msg.reply, socket::ERR_CONNECTION_REFUSED);
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
                    let bound = match entry.kind {
                        SocketKind::Tcp => {
                            sockets.get_mut::<TcpSocket>(entry.handle).listen(listen).is_ok()
                        }
                        SocketKind::Udp => {
                            sockets.get_mut::<UdpSocket>(entry.handle).bind(listen).is_ok()
                        }
                    };
                    if bound {
                        ipc_reply(msg.reply, 0);
                    } else {
                        ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
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
                    if entry.kind != SocketKind::Tcp {
                        if msg.memory != 0 {
                            memory_close(msg.memory);
                        }
                        ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                        continue;
                    }
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
                    if entry.kind != SocketKind::Tcp {
                        ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                        continue;
                    }
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
                    let (scratch_vaddr_3_map_status, scratch_vaddr_3_vaddr) =
                        memory_map_any(msg.memory, false);
                    if scratch_vaddr_3_map_status != 0 {
                        memory_close(msg.memory);
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
                    let data = unsafe {
                        core::slice::from_raw_parts(scratch_vaddr_3_vaddr as *const u8, payload_len)
                    };
                    let result = match entry.kind {
                        SocketKind::Tcp => sockets
                            .get_mut::<TcpSocket>(entry.handle)
                            .send_slice(data)
                            .map(|len| len as i64)
                            .unwrap_or(socket::ERR_WOULD_BLOCK),
                        SocketKind::Udp => entry
                            .udp_remote
                            .and_then(|remote| {
                                sockets
                                    .get_mut::<UdpSocket>(entry.handle)
                                    .send_slice(data, remote)
                                    .ok()
                            })
                            .map(|()| payload_len as i64)
                            .unwrap_or(socket::ERR_NOT_CONNECTED),
                    };
                    if result > 0 {
                        tx_ok = tx_ok.wrapping_add(1);
                        config::write::<u32>(status::TX_OK, tx_ok);
                    } else {
                        tx_err = tx_err.wrapping_add(1);
                    }
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
                        match entry.kind {
                            SocketKind::Tcp => {
                                sockets.get_mut::<TcpSocket>(entry.handle).close();
                            }
                            SocketKind::Udp => {
                                sockets.get_mut::<UdpSocket>(entry.handle).close();
                            }
                        }
                    }
                    config::write::<u32>(status::SOCKETS, state.sockets.len() as u32);
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
                    config::write::<u32>(status::RX_TOTAL, rx_total);
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
                        tx_err,
                        dhcp_mode,
                        gateway_ip,
                        mtu as u32,
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

        let (_, timed_out) = cq_wait_timeout(1, CLOCK_TICK_MS, 0);
        let mut clock_fired = false;
        while let Some(completion) = unsafe { cq_read(cq.base, cq.entries) } {
            if completion.cookie == CLOCK_TIMER_COOKIE {
                clock_fired = true;
                clock_armed = false;
            }
        }
        // The detached timer advances the clock independently of IPC traffic;
        // a continuously-woken cq_wait must not collapse smoltcp's timebase
        // to a fixed 1 ms and stall retransmit/ACK timers under load.
        if clock_fired || timed_out != 0 {
            if !clock_armed {
                clock_armed =
                    submit_detached_timer(CLOCK_TICK_MS, 0, CLOCK_TIMER_COOKIE) != u64::MAX;
            }
            elapsed_ms = CLOCK_TICK_MS;
        } else {
            elapsed_ms = 0;
        }
    }
}

catten_rt::entry!(main);
