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
//! - `vips`: optional canonical table of cluster-service IPv4/TCP identities accepted by every
//!   backend. The frame router independently restricts ARP advertisement and distributes each
//!   service's flows from its committed placement/readiness projection.
//! - `sockslot`: bounded SocketSet capacity (default 64, policy ceiling 1024), clamped to the tcpip
//!   domain's heap at startup. One slot is reserved for DHCP while address acquisition is active.
//! - `sockquot`: maximum number of sockets owned by one authenticated domain (default 16).
//! - `bufquot`: maximum TCP/UDP buffer bytes charged to one authenticated domain (default 2 MiB).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    vec,
    vec::Vec,
};

use catten_rt::{
    Context,
    ManifestValue,
    ShutdownRequest,
    config,
    owned::{
        Connection,
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        PendingCall,
        ReplyToken,
    },
};
use catten_services::{
    dns,
    net,
    ns,
    socket,
    wait_for_local_ready_or_shutdown,
    wait_for_registered_name_owned,
};
use catten_syscall::{
    cq_read,
    cq_wait_timeout,
    ipc_recv_authenticated,
    ipc_reply,
    ipc_reply_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    thread_exit,
};
use charlotte_launch::tcpip_status as status;
use charlotte_protocol_net::decode_status;
use charlotte_smoltcp::{
    CharlotteEthDevice,
    MAX_IPC_REQUESTS_PER_CYCLE,
    ReactorClock,
};
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
            State as TcpState,
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

const FRAME_MAX: usize = 4096;
/// Upper bound on idle protocol polling; endpoint readiness can wake sooner.
const CLOCK_TICK_MS: u64 = 50;
const ASSIGNMENT_REFRESH_MS: u64 = 1_000;
/// Per-socket buffer size. The httpd report exceeds one 4096-byte page, so a
/// single-page buffer forces the sender to stall mid-stream while the peer
/// drains it; a larger buffer lets a full report be accepted without blocking.
const SOCKET_BUF: usize = 16 * 1024;
const UDP_PACKET_SLOTS: usize = 4;
const UDP_BUF: usize = 4096;
/// Leave room in the tcpip heap for smoltcp metadata, request handling, and
/// ordinary service allocations when deriving socket-table capacity.
const SOCKET_HEAP_RESERVE_BYTES: usize = 512 * 1024;
/// Graceful TCP close can remain in FIN-WAIT/TIME-WAIT when a peer has
/// disappeared.  Do not let such sockets consume a tenant's quota forever:
/// after this interval the service aborts the protocol state and reclaims the
/// socket slot and buffers.
const SOCKET_CLOSE_GRACE_MS: u64 = 5_000;

/// Socket ownership is tied to the authenticated sender identity, not to the
/// numeric socket id. The address-space id prevents two live instances of the
/// same artifact principal from sharing sockets; the generation makes an id
/// recyclable without reviving the previous owner's authority.
use catten_services::{
    network_random::NetworkRandom,
    socket_lifetime::{
        self,
        SocketLifetime,
        SocketOwner,
    },
    socket_state::{
        SocketEntry,
        SocketKind,
        TcpipState,
    },
};

fn owner_status(owner: SocketOwner) -> u64 {
    catten_syscall::socket_owner_status(owner.address_space, owner.generation, owner.principal)
}

fn tcpip_policy_value(ctx: &Context, key: u64, default: usize, maximum: usize) -> usize {
    match ctx.manifest_value(key) {
        None => default,
        Some(ManifestValue::Unsigned(value)) => usize::try_from(value)
            .ok()
            .filter(|value| (1..=maximum).contains(value))
            .unwrap_or_else(|| fail(0xe009)),
        Some(_) => fail(0xe009),
    }
}

fn socket_buffer_bytes(kind: SocketKind) -> usize {
    match kind {
        SocketKind::Tcp => SOCKET_BUF * 2,
        SocketKind::Udp => {
            UDP_BUF * 2 + UDP_PACKET_SLOTS * core::mem::size_of::<UdpPacketMetadata>() * 2
        }
    }
}

/// Clamp the launch ceiling to the memory granted to this tcpip domain. TCP
/// is the largest supported socket allocation, so its footprint is the safe
/// bound for either protocol. Retain enough slots for reserved protocol
/// sockets; application opens then fail normally if no capacity remains.
fn resource_socket_capacity(heap_bytes: usize, requested: usize, reserved: usize) -> usize {
    let heap_slots =
        heap_bytes.saturating_sub(SOCKET_HEAP_RESERVE_BYTES) / socket_buffer_bytes(SocketKind::Tcp);
    requested.min(heap_slots.max(reserved.max(1)))
}

/// Monotonic reactor-tick counter for periodic heartbeat logging.
static HEARTBEAT_TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[derive(Default)]
struct SocketSummary {
    tcp_listen: usize,
    tcp_connecting: usize,
    tcp_established: usize,
    tcp_closing: usize,
    tcp_closed: usize,
    udp: usize,
    recv_pending: usize,
    recv_ready: usize,
    send_ready: usize,
}

fn summarize_sockets(state: &TcpipState, sockets: &SocketSet<'_>) -> SocketSummary {
    let mut summary = SocketSummary::default();
    for entry in state.sockets.values() {
        if entry.recv_pending.is_some() {
            summary.recv_pending += 1;
        }
        match entry.kind {
            SocketKind::Tcp => {
                let socket = sockets.get::<TcpSocket>(entry.handle);
                match socket.state() {
                    TcpState::Closed => summary.tcp_closed += 1,
                    TcpState::Listen => summary.tcp_listen += 1,
                    TcpState::SynSent | TcpState::SynReceived => summary.tcp_connecting += 1,
                    TcpState::Established => summary.tcp_established += 1,
                    TcpState::FinWait1
                    | TcpState::FinWait2
                    | TcpState::CloseWait
                    | TcpState::Closing
                    | TcpState::LastAck
                    | TcpState::TimeWait => summary.tcp_closing += 1,
                }
                summary.recv_ready += usize::from(socket.can_recv());
                summary.send_ready += usize::from(socket.can_send());
            }
            SocketKind::Udp => {
                let socket = sockets.get::<UdpSocket>(entry.handle);
                summary.udp += 1;
                summary.recv_ready += usize::from(socket.can_recv());
                summary.send_ready += usize::from(socket.can_send());
            }
        }
    }
    summary
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

/// Decode the address-specific bind/listen payload at this protocol adapter
/// boundary. The caller retains responsibility for closing the moved object.
fn read_ipv4_listen_endpoint(memory: u64) -> Option<smoltcp::wire::IpListenEndpoint> {
    let (status, vaddr) = memory_map_any(memory, false);
    if status != 0 {
        return None;
    }
    let bytes = unsafe { core::slice::from_raw_parts(vaddr as *const u8, 6) };
    let address = Ipv4Address::new(bytes[0], bytes[1], bytes[2], bytes[3]);
    let port = u16::from_le_bytes([bytes[4], bytes[5]]);
    memory_unmap(memory);
    (address != Ipv4Address::UNSPECIFIED && port != 0).then_some(smoltcp::wire::IpListenEndpoint {
        addr: Some(IpAddress::Ipv4(address)),
        port,
    })
}

struct AssignmentClient {
    lookup: Option<PendingCall<'static>>,
    connection: Option<Connection>,
    request: Option<PendingCall<'static>>,
}

impl AssignmentClient {
    fn new() -> Self {
        Self {
            lookup: None,
            connection: None,
            request: None,
        }
    }

    fn poll(&mut self, names: ConnectionRef<'_>, refresh_due: bool) -> Option<Vec<Ipv4Address>> {
        if let Some(request) = self.request.as_mut() {
            match request.poll() {
                Ok(None) => return None,
                Ok(Some(result)) => {
                    self.request = None;
                    let length = usize::try_from(result.result).ok()?;
                    let memory = result.memory?;
                    let mapping = memory.map_read_only().ok()?;
                    let bytes = mapping.as_slice().get(..length)?;
                    return Some(charlotte_launch::ingress::decode(bytes)?.fold(
                        Vec::new(),
                        |mut addresses, binding| {
                            let [a, b, c, d] = binding.service.address;
                            let address = Ipv4Address::new(a, b, c, d);
                            if !addresses.contains(&address) {
                                addresses.push(address);
                            }
                            addresses
                        },
                    ));
                }
                Err(_) => {
                    self.request = None;
                    self.connection = None;
                }
            }
        }
        if let Some(lookup) = self.lookup.as_mut() {
            match lookup.poll() {
                Ok(None) => return None,
                Ok(Some(result)) => {
                    self.lookup = None;
                    if result.result >= 1 {
                        self.connection = result.connection;
                    }
                }
                Err(_) => self.lookup = None,
            }
        }
        if self.connection.is_none() {
            self.lookup = names.call(ns::OP_LOOKUP, dns::NAME).ok();
            return None;
        }
        if refresh_due {
            self.request = self
                .connection
                .as_ref()
                .and_then(|connection| connection.call(dns::OP_INGRESS_ASSIGNMENTS, 0).ok());
        }
        None
    }
}

fn install_interface_addresses(
    iface: &mut Interface,
    local: Option<Ipv4Cidr>,
    service_vips: &[Ipv4Address],
) {
    iface.update_ip_addrs(|addrs| {
        addrs.clear();
        if let Some(local) = local {
            let _ = addrs.push(IpCidr::Ipv4(local));
        }
        for vip in service_vips
            .iter()
            .copied()
            .filter(|vip| local.is_none_or(|cidr| *vip != cidr.address()))
        {
            addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(vip, 32))).unwrap_or_else(|_| fail(0xe008));
        }
    });
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    catten_syscall::el0_log(0x5443_5049, code as u64); // "TCPI"
    unsafe { thread_exit() }
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| fail(0xe001));
    config::write::<u32>(status::STAGE, 2);

    config::write::<u32>(status::DETAIL, 1);
    let (_, net_conn) =
        wait_for_registered_name_owned(ns_connection, net::NAME).unwrap_or_else(|| fail(0xe002));
    config::write::<u32>(status::DETAIL, 2);

    let status_call = net_conn.call(net::OP_STATUS, 0).unwrap_or_else(|_| fail(0xe003));
    config::write::<u32>(status::DETAIL, 3);
    let nic_status = status_call.wait().unwrap_or_else(|_| fail(0xe003)).result;
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
    let mut local_cidr = (!dhcp).then(|| Ipv4Cidr::new(local_ip, 24));
    let gateway = match ctx.manifest_value(charlotte_launch::manifest_key(b"gateway")) {
        Some(ManifestValue::Bytes(raw)) if raw.len() == 4 => {
            Some(Ipv4Address::new(raw[0], raw[1], raw[2], raw[3]))
        }
        _ => None,
    };
    let mut service_vips = match ctx.manifest_value(charlotte_launch::manifest_key(b"vips")) {
        Some(ManifestValue::Bytes(raw)) => charlotte_launch::ingress::decode(raw)
            .unwrap_or_else(|| fail(0xe007))
            .map(|binding| {
                let [a, b, c, d] = binding.service.address;
                Ipv4Address::new(a, b, c, d)
            })
            .fold(Vec::new(), |mut addresses, address| {
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
                addresses
            }),
        Some(_) => fail(0xe007),
        None => Vec::new(),
    };
    config::write::<u32>(status::STAGE, 3);

    let endpoint =
        Endpoint::create(socket::INTERFACE, socket::VERSION, 8).unwrap_or_else(|_| fail(0xe004));
    let reg = loop {
        if let Ok(call) = ns_connection.call_connection(
            ns::OP_REGISTER,
            socket::NAME,
            &endpoint,
            catten_syscall::IpcRights::SEND
                | catten_syscall::IpcRights::CALL
                | catten_syscall::IpcRights::MINT_CONNECTION,
        ) {
            break call;
        }
        // The name-service queue is shared by the booting service set. Yield
        // through a timer when it is temporarily full; the endpoint and
        // delegated connection remain owned by this process and are safe to
        // submit again.
        catten_services::sleep_ms(1);
    };
    let registration = reg.wait().unwrap_or_else(|_| fail(0xe005));
    if registration.result < 1 {
        fail(0xe005);
    }
    endpoint.bind_completion_queue(0).unwrap_or_else(|_| fail(0xe006));
    config::write::<u32>(status::STAGE, 4);

    // Let the NIC and the link settle before ARP/IP traffic starts flowing.
    if let Err(request) = wait_for_local_ready_or_shutdown(ctx, ns_connection) {
        return request;
    }
    config::write::<u32>(status::STAGE, 5);

    // CharlotteEthDevice is the low-level smoltcp/protocol adapter. It borrows
    // this owned connection handle for the lifetime of the serving scope.
    let mut device = CharlotteEthDevice::new(net_conn.as_raw(), mtu);
    let hw = HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress(mac));
    let mut cfg = Config::new(hw);
    // Entropy is obtained before the interface emits any protocol state.
    let random = NetworkRandom::initialize(|bytes| {
        if catten_services::tls_client::fill_entropy(None, bytes).is_ok() {
            return Ok(());
        }
        // RNG registration precedes network startup and needs no IP traffic.
        let (_, rng) =
            wait_for_registered_name_owned(ns_connection, catten_services::entropy::NAME)
                .ok_or(())?;
        catten_services::tls_client::fill_entropy(Some(rng.as_ref()), bytes).map_err(|_| ())
    })
    .unwrap_or_else(|_| fail(0xe00a));
    cfg.random_seed = random.interface_seed();
    let mut iface = Interface::new(cfg, &mut device, Instant::from_millis(0));
    if !dhcp {
        install_interface_addresses(&mut iface, local_cidr, &service_vips);
        if let Some(gw) = gateway {
            iface.routes_mut().add_default_ipv4_route(gw).ok();
        }
    }
    let requested_socket_slots = tcpip_policy_value(
        ctx,
        charlotte_launch::tcpip_config::SOCKET_SLOTS_KEY,
        charlotte_launch::tcpip_config::DEFAULT_SOCKET_SLOTS,
        charlotte_launch::tcpip_config::MAX_SOCKET_SLOTS,
    );
    let sockets_per_principal = tcpip_policy_value(
        ctx,
        charlotte_launch::tcpip_config::SOCKET_QUOTA_KEY,
        charlotte_launch::tcpip_config::DEFAULT_SOCKETS_PER_PRINCIPAL,
        charlotte_launch::tcpip_config::MAX_SOCKETS_PER_PRINCIPAL,
    );
    let buffer_bytes_per_principal = tcpip_policy_value(
        ctx,
        charlotte_launch::tcpip_config::BUFFER_QUOTA_KEY,
        charlotte_launch::tcpip_config::DEFAULT_BUFFER_BYTES_PER_PRINCIPAL,
        charlotte_launch::tcpip_config::MAX_BUFFER_BYTES_PER_PRINCIPAL,
    );
    let reserved_slots = usize::from(dhcp);
    let socket_slots =
        resource_socket_capacity(ctx.heap_layout().size, requested_socket_slots, reserved_slots);
    catten_rt::logln!(
        "[tcpip] socket policy requested_slots={} effective_slots={} per_owner={} buffer_quota={} \
         KiB heap={} KiB",
        requested_socket_slots,
        socket_slots,
        sockets_per_principal,
        buffer_bytes_per_principal / 1024,
        ctx.heap_layout().size / 1024,
    );
    if socket_slots < requested_socket_slots {
        catten_rt::logln!(
            "[tcpip] socket capacity clamped by heap: requested={} effective={}",
            requested_socket_slots,
            socket_slots
        );
    }
    let mut sock_storage: Vec<smoltcp::iface::SocketStorage<'_>> =
        core::iter::repeat_with(Default::default).take(socket_slots).collect();
    let mut sockets = SocketSet::new(&mut sock_storage[..]);
    let dhcp_handle = if dhcp {
        Some(sockets.add(dhcpv4::Socket::new()))
    } else {
        None
    };
    let mut state = TcpipState {
        sockets: alloc::collections::BTreeMap::new(),
        next_sock_id: 1,
        random,
    };
    let (counter, frequency) = catten_syscall::monotonic_clock();
    let mut clock = ReactorClock::new(counter, frequency).unwrap_or_else(|| fail(0xe00b));
    let mut rx_queue_drops = 0u64;
    let mut rx_total: u32 = 0;
    let mut rx_map_errors: u32 = 0;
    let mut rx_last_map_status: u32 = 0;
    let mut tx_ok: u32 = 0;
    let mut tx_err: u32 = 0;
    let mut quota_rejections: u64 = 0;
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

    let cq = ctx.completion_queue_layout();
    let mut assignment_client = AssignmentClient::new();
    let mut next_assignment_refresh_ms = 0u64;

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            // All higher-level socket consumers have already drained. Reject
            // new endpoint work by returning from this scope, complete any
            // retained receive calls, and stop residual protocol sockets
            // before the frame router and NIC are asked to quiesce.
            let socket_count = state.sockets.len();
            for entry in state.sockets.values_mut() {
                if let Some(token) = entry.recv_pending.take() {
                    let _ = token.reply(0);
                }
                match entry.kind {
                    SocketKind::Tcp => sockets.get_mut::<TcpSocket>(entry.handle).abort(),
                    SocketKind::Udp => sockets.get_mut::<UdpSocket>(entry.handle).close(),
                }
            }
            state.sockets.clear();
            config::write::<u32>(status::SOCKETS, 0);
            catten_rt::logln!("[tcpip] shutdown: released {} socket(s)", socket_count);
            return request;
        }
        let (counter, frequency) = catten_syscall::monotonic_clock();
        let ticks = clock.sample(counter, frequency).unwrap_or_else(|| fail(0xe00b));
        config::write::<u64>(status::MONOTONIC_MS, ticks);
        device.poll_smoltcp(&mut iface, &mut sockets, ticks);
        let assignments_due = ticks >= next_assignment_refresh_ms;
        if assignments_due {
            next_assignment_refresh_ms = ticks.saturating_add(ASSIGNMENT_REFRESH_MS);
        }
        if let Some(vips) = assignment_client.poll(ns_connection, assignments_due)
            && vips != service_vips
        {
            install_interface_addresses(&mut iface, local_cidr, &vips);
            service_vips = vips;
            catten_rt::logln!("[tcpip] installed {} committed cluster VIP(s)", service_vips.len());
        }

        // Periodic heartbeat (~every 1024 reactor iterations) with enough
        // protocol state to distinguish a dead listener, a stranded deferred
        // receive, socket backpressure, and loss before the stack. `tx_ok` and
        // `tx_err` count client OP_SEND calls, not emitted Ethernet frames.
        let tick = HEARTBEAT_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        config::write::<u64>(status::REACTOR_CYCLES, tick.saturating_add(1));
        if tick & 0x3ff == 0 {
            let summary = summarize_sockets(&state, &sockets);
            catten_rt::logln!(
                "[tcpip] hb rx={} rxq={} rx_drop={} rx_map_err={}:{} tx_ok={} tx_err={} \
                 sockets={} tcp=l{}/c{}/e{}/x{}/z{} udp={} recv={}/{} send_ready={}",
                rx_total,
                device.rx_len(),
                rx_queue_drops,
                rx_map_errors,
                rx_last_map_status,
                tx_ok,
                tx_err,
                state.sockets.len(),
                summary.tcp_listen,
                summary.tcp_connecting,
                summary.tcp_established,
                summary.tcp_closing,
                summary.tcp_closed,
                summary.udp,
                summary.recv_pending,
                summary.recv_ready,
                summary.send_ready
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
                    local_cidr = Some(cidr);
                    let octets = local_ip.octets();
                    catten_rt::logln!(
                        "[tcpip] DHCP assigned {}.{}.{}.{}/{}",
                        octets[0],
                        octets[1],
                        octets[2],
                        octets[3],
                        cidr.prefix_len()
                    );
                    install_interface_addresses(&mut iface, local_cidr, &service_vips);
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
                    local_cidr = None;
                    install_interface_addresses(&mut iface, local_cidr, &service_vips);
                    iface.routes_mut().remove_default_ipv4_route();
                }
            }
        }

        // Sweep sockets that have fully closed (graceful close finished) so
        // their handles are recycled. A peer may never complete the FIN
        // handshake, or the owner's best-effort OP_CLOSE may be lost when the
        // request queue is full. Track terminal TCP states as well and
        // force-abort them after a bounded interval. This keeps reconnect
        // churn from exhausting a principal's quota with unusable sockets.
        state.reap_abandoned(&mut sockets, ticks, |owner| matches!(owner_status(owner), 1 | 2));
        let mut closing: [u64; 8] = [0; 8];
        let mut closing_n: usize = 0;
        for (id, entry) in state.sockets.iter_mut() {
            let (is_open, tcp_terminal) = match entry.kind {
                SocketKind::Tcp => {
                    let socket = sockets.get::<TcpSocket>(entry.handle);
                    (
                        socket.is_open(),
                        matches!(
                            socket.state(),
                            TcpState::FinWait1
                                | TcpState::FinWait2
                                | TcpState::CloseWait
                                | TcpState::Closing
                                | TcpState::LastAck
                                | TcpState::TimeWait
                        ),
                    )
                }
                SocketKind::Udp => (sockets.get::<UdpSocket>(entry.handle).is_open(), false),
            };
            if entry.activated && tcp_terminal && entry.close_started_ms.is_none() {
                // The peer may have initiated the close, leaving no client
                // OP_CLOSE request for us to observe.
                entry.close_started_ms = Some(ticks);
            }
            if entry
                .close_started_ms
                .is_some_and(|started| ticks.saturating_sub(started) >= SOCKET_CLOSE_GRACE_MS)
                && is_open
            {
                match entry.kind {
                    SocketKind::Tcp => sockets.get_mut::<TcpSocket>(entry.handle).abort(),
                    SocketKind::Udp => sockets.get_mut::<UdpSocket>(entry.handle).close(),
                }
            }
            // A freshly opened socket is also reported as closed by smoltcp
            // until the client binds/listens/connects it.  Do not reap such
            // an entry between OP_SOCKET and the follow-up operation. An
            // activated socket is eligible once smoltcp has reached a terminal
            // state, and an explicit OP_CLOSE makes even an unactivated socket
            // eligible.
            let closed = match entry.kind {
                SocketKind::Tcp => !sockets.get::<TcpSocket>(entry.handle).is_open(),
                SocketKind::Udp => !sockets.get::<UdpSocket>(entry.handle).is_open(),
            };
            if (entry.closing || entry.activated) && closed && closing_n < 8 {
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

        // Reply authority and receive backing remain owned on every branch.
        for entry in state.sockets.values_mut() {
            if entry.recv_pending.is_none() {
                continue;
            }
            let can_recv = match entry.kind {
                SocketKind::Tcp => sockets.get::<TcpSocket>(entry.handle).can_recv(),
                SocketKind::Udp => sockets.get::<UdpSocket>(entry.handle).can_recv(),
            };
            if !can_recv {
                continue;
            }
            let Ok(memory) = OwnedMemory::allocate(1) else {
                continue;
            };
            let Ok(mut mapping) = memory.map_writable() else {
                continue;
            };
            let buf = mapping.as_mut_slice();
            let received = match entry.kind {
                SocketKind::Tcp => sockets
                    .get_mut::<TcpSocket>(entry.handle)
                    .recv_slice(buf)
                    .ok()
                    .map(|len| (len, true)),
                SocketKind::Udp => sockets
                    .get_mut::<UdpSocket>(entry.handle)
                    .recv_slice(buf)
                    .ok()
                    .map(|(len, metadata)| (len, entry.udp_remote == Some(metadata.endpoint))),
            };
            if let Some((len, true)) = received {
                let Ok(memory) = mapping.unmap() else {
                    continue;
                };
                let reply = entry.recv_pending.take().expect("checked pending receive");
                if len > 0 {
                    let _ = reply.reply_move(memory, len as i64);
                } else {
                    let _ = reply.reply(0);
                }
            }
        }

        for _ in 0..MAX_IPC_REQUESTS_PER_CYCLE {
            let msg = ipc_recv_authenticated(endpoint.as_raw());
            let _attachments = catten_services::RequestAttachments::new(msg.memory, msg.connection);
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

            let owner = SocketOwner::from_message(&msg);
            match msg.opcode {
                socket::OP_SOCKET => {
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    if msg.arg0 != socket::DOMAIN_TCP && msg.arg0 != socket::DOMAIN_UDP {
                        ipc_reply(msg.reply, socket::ERR_BAD_DOMAIN);
                        continue;
                    }
                    let kind = if msg.arg0 == socket::DOMAIN_TCP {
                        SocketKind::Tcp
                    } else {
                        SocketKind::Udp
                    };
                    let application_capacity = socket_slots.saturating_sub(reserved_slots);
                    let buffer_bytes = socket_buffer_bytes(kind);
                    let owner_state = owner_status(owner);
                    let platform = owner_state == 2;
                    let ordinary =
                        state.sockets.values().filter(|entry| !entry.lifetime.platform).count();
                    if !matches!(owner_state, 1 | 2)
                        || !socket_lifetime::can_admit(
                            application_capacity,
                            state.sockets.len(),
                            ordinary,
                            platform,
                        )
                    {
                        catten_rt::logln!(
                            "[tcpip] socket admission rejected: total={} capacity={} owner={:?}",
                            state.sockets.len(),
                            application_capacity,
                            owner
                        );
                        ipc_reply(msg.reply, socket::ERR_TOO_MANY_SOCKETS);
                        continue;
                    }
                    let owner_sockets = state.owner_socket_count(owner);
                    let owner_buffers = state.owner_buffer_bytes(owner);
                    if owner_sockets >= sockets_per_principal
                        || owner_buffers.saturating_add(buffer_bytes) > buffer_bytes_per_principal
                    {
                        quota_rejections = quota_rejections.saturating_add(1);
                        if quota_rejections <= 3 || quota_rejections.is_power_of_two() {
                            catten_rt::logln!(
                                "[tcpip] socket admission rejected (count={}): \
                                 owner_sockets={}/{} owner_buffers={}/{} total={} owner={:?}",
                                quota_rejections,
                                owner_sockets,
                                sockets_per_principal,
                                owner_buffers,
                                buffer_bytes_per_principal,
                                state.sockets.len(),
                                owner
                            );
                        }
                        ipc_reply(msg.reply, socket::ERR_QUOTA_EXCEEDED);
                        continue;
                    }
                    let handle = if kind == SocketKind::Tcp {
                        let rx = TcpSocketBuffer::new(vec![0u8; SOCKET_BUF]);
                        let tx = TcpSocketBuffer::new(vec![0u8; SOCKET_BUF]);
                        sockets.add(TcpSocket::new(rx, tx))
                    } else {
                        let rx = UdpPacketBuffer::new(
                            vec![UdpPacketMetadata::EMPTY; UDP_PACKET_SLOTS],
                            vec![0u8; UDP_BUF],
                        );
                        let tx = UdpPacketBuffer::new(
                            vec![UdpPacketMetadata::EMPTY; UDP_PACKET_SLOTS],
                            vec![0u8; UDP_BUF],
                        );
                        sockets.add(UdpSocket::new(rx, tx))
                    };
                    let entry = SocketEntry {
                        handle,
                        kind,
                        owner,
                        buffer_bytes,
                        lifetime: SocketLifetime::new(platform, ticks),
                        activated: false,
                        udp_remote: None,
                        recv_pending: None,
                        closing: false,
                        close_started_ms: None,
                    };
                    // ABI boundary: receive transferred this authority exactly once.
                    let reply =
                        unsafe { ReplyToken::from_raw(msg.reply) }.expect("nonzero received reply");
                    let _ = state.publish(&mut sockets, entry, |id| reply.reply(id as i64));
                    config::write::<u32>(status::SOCKETS, state.sockets.len() as u32);
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
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
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
                        entry.activated = true;
                        ipc_reply(msg.reply, 0);
                    } else {
                        ipc_reply(msg.reply, socket::ERR_CONNECTION_REFUSED);
                    };
                }

                socket::OP_BIND => {
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
                        Some(e) => e,
                        None => {
                            if msg.memory != 0 {
                                memory_close(msg.memory);
                            }
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
                        entry.activated = true;
                        ipc_reply(msg.reply, 0);
                    } else {
                        ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                    };
                }

                socket::OP_BIND_IPV4 => {
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
                        Some(entry) => entry,
                        None => {
                            if msg.memory != 0 {
                                memory_close(msg.memory);
                            }
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    let listen =
                        (msg.memory != 0).then(|| read_ipv4_listen_endpoint(msg.memory)).flatten();
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    let Some(listen) = listen else {
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
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
                        entry.activated = true;
                    }
                    ipc_reply(
                        msg.reply,
                        if bound {
                            0
                        } else {
                            socket::ERR_BAD_SOCKET
                        },
                    );
                }

                socket::OP_LISTEN => {
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
                        Some(e) => e,
                        None => {
                            if msg.memory != 0 {
                                memory_close(msg.memory);
                            }
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
                        Ok(()) => {
                            entry.activated = true;
                            ipc_reply(msg.reply, 0)
                        }
                        Err(_) => ipc_reply(msg.reply, socket::ERR_BAD_SOCKET),
                    };
                }

                socket::OP_LISTEN_IPV4 => {
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
                        Some(entry) if entry.kind == SocketKind::Tcp => entry,
                        _ => {
                            if msg.memory != 0 {
                                memory_close(msg.memory);
                            }
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    let listen =
                        (msg.memory != 0).then(|| read_ipv4_listen_endpoint(msg.memory)).flatten();
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    let Some(listen) = listen else {
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    };
                    let result = sockets
                        .get_mut::<TcpSocket>(entry.handle)
                        .listen(listen)
                        .map_or(socket::ERR_BAD_SOCKET, |()| 0);
                    if result == 0 {
                        entry.activated = true;
                    }
                    ipc_reply(msg.reply, result);
                }

                socket::OP_ACCEPT => {
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
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
                    let entry = match state.owned_entry_mut(sock_id, owner) {
                        Some(e) => e,
                        None => {
                            if msg.memory != 0 {
                                memory_close(msg.memory);
                            }
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
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
                        Some(e) => e,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    if entry.recv_pending.is_some() {
                        ipc_reply(msg.reply, socket::ERR_WOULD_BLOCK);
                    } else {
                        // ABI boundary: deferred reply moves into the socket operation owner.
                        entry.recv_pending = unsafe { ReplyToken::from_raw(msg.reply) }.ok();
                    }
                }

                socket::OP_CANCEL_RECV => {
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    let entry = match state.owned_entry_mut(msg.arg0, owner) {
                        Some(entry) => entry,
                        None => {
                            ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                            continue;
                        }
                    };
                    if let Some(token) = entry.recv_pending.take() {
                        let _ = token.reply(socket::ERR_WOULD_BLOCK);
                    }
                    ipc_reply(msg.reply, 0);
                }

                socket::OP_CLOSE => {
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    // Graceful close: transition to FIN-WAIT so queued
                    // transmit data (e.g. an httpd response) drains before the
                    // FIN; the reactor sweeps the socket once fully closed.
                    let Some(entry) = state.owned_entry_mut(msg.arg0, owner) else {
                        ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                        continue;
                    };
                    if let Some(token) = entry.recv_pending.take() {
                        let _ = token.reply(0);
                    }
                    entry.closing = true;
                    entry.close_started_ms = Some(ticks);
                    match entry.kind {
                        SocketKind::Tcp => {
                            sockets.get_mut::<TcpSocket>(entry.handle).close();
                        }
                        SocketKind::Udp => {
                            sockets.get_mut::<UdpSocket>(entry.handle).close();
                        }
                    }
                    config::write::<u32>(status::SOCKETS, state.sockets.len() as u32);
                    ipc_reply(msg.reply, 0);
                }

                socket::OP_FRAME => {
                    if !catten_syscall::is_frame_router(msg.sender, msg.sender_generation) {
                        if msg.memory != 0 {
                            memory_close(msg.memory);
                        }
                        ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                        continue;
                    }
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
                    if scratch_vaddr_2_map_status != 0 {
                        memory_close(msg.memory);
                        rx_map_errors = rx_map_errors.wrapping_add(1);
                        rx_last_map_status = scratch_vaddr_2_map_status as u32;
                        config::write::<u32>(status::RX_MAP_ERRORS, rx_map_errors);
                        config::write::<u32>(status::RX_LAST_MAP_STATUS, rx_last_map_status);
                        ipc_reply(msg.reply, socket::ERR_WOULD_BLOCK);
                        continue;
                    }
                    let frame = unsafe {
                        core::slice::from_raw_parts(scratch_vaddr_2_vaddr as *const u8, frame_len)
                    };
                    let admitted = device.push_rx(frame);
                    if !admitted {
                        rx_queue_drops = rx_queue_drops.saturating_add(1);
                        config::write::<u32>(
                            status::RX_QUEUE_DROPS,
                            rx_queue_drops.min(u32::MAX as u64) as u32,
                        );
                    }
                    memory_unmap(msg.memory);
                    memory_close(msg.memory);
                    rx_total = rx_total.wrapping_add(1);
                    config::write::<u32>(status::RX_TOTAL, rx_total);
                    ipc_reply(
                        msg.reply,
                        if admitted {
                            0
                        } else {
                            socket::ERR_WOULD_BLOCK
                        },
                    );
                }

                socket::OP_STATUS => {
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
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
                        socket_slots as u32,
                        sockets_per_principal as u32,
                        buffer_bytes_per_principal as u32,
                        requested_socket_slots as u32,
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

                socket::OP_CONNECTION_STATE => {
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    let Some(entry) = state.owned_entry(msg.arg0, owner) else {
                        ipc_reply(msg.reply, socket::ERR_BAD_SOCKET);
                        continue;
                    };
                    let connection_state = match entry.kind {
                        SocketKind::Tcp => match sockets.get::<TcpSocket>(entry.handle).state() {
                            TcpState::Closed | TcpState::Listen => socket::CONNECTION_STATE_CLOSED,
                            TcpState::SynSent | TcpState::SynReceived => {
                                socket::CONNECTION_STATE_CONNECTING
                            }
                            TcpState::Established => socket::CONNECTION_STATE_ESTABLISHED,
                            TcpState::FinWait1
                            | TcpState::FinWait2
                            | TcpState::CloseWait
                            | TcpState::Closing
                            | TcpState::LastAck
                            | TcpState::TimeWait => socket::CONNECTION_STATE_CLOSING,
                        },
                        SocketKind::Udp if entry.udp_remote.is_some() => {
                            socket::CONNECTION_STATE_ESTABLISHED
                        }
                        SocketKind::Udp => socket::CONNECTION_STATE_CLOSED,
                    };
                    ipc_reply(msg.reply, connection_state);
                }

                _ => {
                    if msg.memory != 0 {
                        memory_close(msg.memory);
                    }
                    ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                }
            }
        }

        let _ = cq_wait_timeout(1, CLOCK_TICK_MS, 0);
        // Bound draining even if another source can replenish this CQ. Time
        // comes from the scalar clock, not the number of cookies consumed.
        for _ in 0..cq.entries {
            if unsafe { cq_read(cq.base, cq.entries) }.is_none() {
                break;
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
