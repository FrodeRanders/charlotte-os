//! Frame demultiplexer ("frouter") — the single owner of the NIC receive path.
//!
//! Every node needs exactly one service holding `net OP_RECV`, otherwise
//! protocol consumers (reliable messages, cluster discovery, TCP/IP) would
//! fight over the driver's single deferred-receive slot. The frouter holds
//! that slot permanently and dispatches each received frame to the service
//! registered for its EtherType. Consumers implement an `OP_FRAME` ingress
//! opcode that takes the moved frame buffer.
//!
//! The route table is EtherType → (service name, ingress opcode). Consumers
//! are optional: a route is installed once the named service is registered,
//! so the frouter works whether a node runs relmsg, disco, tcpip, both, or
//! neither. Each absent route owns one deferred name-service lookup; route
//! discovery therefore never blocks the NIC receive loop.
//!
//! Status-page diagnostics use the fields in
//! [`charlotte_launch::frouter_status`]: lifecycle stage, received, forwarded,
//! dropped and unrouted frame counters, plus the installed-route count.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use catten_rt::{
    Context,
    ManifestValue,
    ShutdownRequest,
    config,
    owned::{
        Completion,
        Connection,
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        PendingCall,
    },
};
use catten_services::{
    cluster_ingress::{
        BackendSnapshot,
        FORWARDED_ETHERTYPE,
        FlowEpochTable,
        ServiceId,
        SnapshotHistory,
        decapsulate_forwarded_frame,
        encapsulate_forwarded_frame,
        gratuitous_arp,
        is_arp_request_for_vip,
        local_advertises_vip,
        parse_service_flow,
        select_backend,
    },
    disco,
    dns,
    frouter,
    net,
    ns,
    raft,
    relmsg,
    sleep_ms,
    socket,
    wait_for_registered_name_owned,
};
use catten_syscall::{
    IpcRights,
    thread_exit,
};
use charlotte_launch::frouter_status as status;
use charlotte_protocol_disco::DISCO_ETHERTYPE;
use charlotte_protocol_msg::MSG_ETHERTYPE;

const IPV4_ETHERTYPE: u16 = 0x0800;
const ARP_ETHERTYPE: u16 = 0x0806;

const FRAME_MAX: usize = 4096;
const ETHERTYPE_OFFSET: usize = 12;
const ETHERNET_HEADER_MIN: usize = 14;
/// Poll interval for the NIC receive call and deferred route lookups. The
/// driver's `OP_RECV` reply only completes when a frame arrives, so while no
/// frames are flowing we poll it rather than blocking the router reactor.
const ROUTE_RETRY_MS: u64 = 50;
const MAX_PENDING_PER_ROUTE: usize = 8;
const MAX_PENDING_L2_SENDS: usize = 32;
const FLOW_EPOCH_CAPACITY: usize = 1024;
const SNAPSHOT_HISTORY_CAPACITY: usize = 4;
const MEMBERSHIP_REFRESH_MS: u64 = 1_000;
const VIP_KEY: u64 = charlotte_launch::manifest_key(b"vip");
const VIP_PORT_KEY: u64 = charlotte_launch::manifest_key(b"vipport");

/// Monotonic reactor-tick counter for periodic heartbeat logging.
static HEARTBEAT_TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

struct Route {
    ethertype: u16,
    conn: Connection,
    opcode: u32,
}

struct RouteLookup {
    ethertype: u16,
    name: u64,
    opcode: u32,
    call: Option<PendingCall<'static>>,
}

struct PendingForward {
    call: PendingCall<'static>,
    route_ethertype: u16,
}

struct MembershipClient {
    lookup: Option<PendingCall<'static>>,
    connection: Option<Connection>,
    request: Option<PendingCall<'static>>,
}

impl MembershipClient {
    fn new() -> Self {
        Self {
            lookup: None,
            connection: None,
            request: None,
        }
    }

    /// Poll the DNS-owned materialized Raft membership without ever blocking
    /// the NIC owner. A malformed/partial reply leaves the previous immutable
    /// snapshot installed.
    fn poll(
        &mut self,
        ns_conn: ConnectionRef<'_>,
        refresh_due: bool,
        history: &mut SnapshotHistory,
    ) -> Option<BackendSnapshot> {
        if let Some(request) = self.request.as_mut() {
            match request.poll() {
                Ok(None) => return None,
                Ok(Some(result)) => {
                    self.request = None;
                    let length = usize::try_from(result.result).ok()?;
                    let memory = result.memory?;
                    let mapping = memory.map_read_only().ok()?;
                    let bytes = mapping.as_slice().get(..length)?;
                    let snapshot = BackendSnapshot::decode(bytes)?;
                    drop(mapping);
                    history.install(snapshot.clone());
                    return Some(snapshot);
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
            self.lookup = ns_conn.call(ns::OP_LOOKUP, dns::NAME).ok();
            return None;
        }
        if refresh_due {
            self.request = self
                .connection
                .as_ref()
                .and_then(|connection| connection.call(dns::OP_INGRESS_MEMBERSHIP, 0).ok());
        }
        None
    }
}

enum IngressDecision {
    Ordinary,
    Local,
    Remote {
        ingress: [u8; 6],
        destination: [u8; 6],
    },
    Drop,
}

fn classify_ingress(
    memory: OwnedMemory,
    frame_len: usize,
    service: ServiceId,
    history: &SnapshotHistory,
    flows: &mut FlowEpochTable,
) -> Option<(OwnedMemory, usize, u16, IngressDecision)> {
    let mapping = memory.map_read_only().ok()?;
    let frame = mapping.as_slice().get(..frame_len)?;
    let ethertype = u16::from_be_bytes(frame.get(12..14)?.try_into().ok()?);
    let current = history.current();
    if ethertype == FORWARDED_ETHERTYPE {
        let source: [u8; 6] = frame.get(6..12)?.try_into().ok()?;
        let trusted = current
            .is_some_and(|snapshot| snapshot.members().iter().any(|backend| backend.mac == source));
        let memory = mapping.unmap().ok()?;
        if !trusted {
            return Some((memory, frame_len, ethertype, IngressDecision::Drop));
        }
        let mut mapping = memory.map_writable().ok()?;
        let (restored_len, restored_ethertype) =
            decapsulate_forwarded_frame(mapping.as_mut_slice(), frame_len)?;
        let memory = mapping.unmap().ok()?;
        return Some((memory, restored_len, restored_ethertype, IngressDecision::Local));
    }
    let decision = if ethertype == ARP_ETHERTYPE
        && is_arp_request_for_vip(frame, service.address)
        && !local_advertises_vip(current)
    {
        IngressDecision::Drop
    } else if let Some(packet) = parse_service_flow(frame, &service) {
        let Some(current) = current else {
            let memory = mapping.unmap().ok()?;
            return Some((memory, frame_len, ethertype, IngressDecision::Drop));
        };
        let epoch = flows.observe(&packet, current.epoch);
        let snapshot = history.get(epoch).unwrap_or(current);
        match select_backend(&service, &packet.key, snapshot) {
            Some(backend) if backend.node_id == snapshot.self_node => IngressDecision::Local,
            Some(backend) => {
                let ingress = snapshot
                    .members()
                    .iter()
                    .find(|candidate| candidate.node_id == snapshot.self_node)
                    .map(|candidate| candidate.mac)?;
                IngressDecision::Remote {
                    ingress,
                    destination: backend.mac,
                }
            }
            // A complete committed snapshot with no eligible backend is an
            // explicit fail-closed policy, not ordinary local VIP traffic.
            None => IngressDecision::Drop,
        }
    } else {
        IngressDecision::Ordinary
    };
    let memory = mapping.unmap().ok()?;
    Some((memory, frame_len, ethertype, decision))
}

fn submit_gratuitous_arp(
    net_conn: &Connection,
    local_mac: [u8; 6],
    vip: [u8; 4],
) -> Option<PendingCall<'static>> {
    let memory = OwnedMemory::allocate(1).ok()?;
    let mut mapping = memory.map_writable().ok()?;
    let frame = gratuitous_arp(local_mac, vip);
    mapping.as_mut_slice().get_mut(..frame.len())?.copy_from_slice(&frame);
    let memory = mapping.unmap().ok()?;
    net_conn.call_move(net::OP_SEND, frame.len() as u64, memory).ok()
}

fn lookup(ns_conn: ConnectionRef<'_>, name: u64) -> Option<Connection> {
    wait_for_registered_name_owned(ns_conn, name).map(|(_, connection)| connection)
}

fn refresh_routes(
    routes: &mut Vec<Route>,
    lookups: &mut [RouteLookup],
    ns_conn: ConnectionRef<'_>,
) {
    for lookup in lookups {
        if routes.iter().any(|route| route.ethertype == lookup.ethertype) {
            continue;
        }

        if lookup.call.is_none() {
            // OP_LOOKUP intentionally remains pending while the service is
            // absent. This is the name service's synchronization mechanism;
            // keeping the call in this small fixed table makes it asynchronous
            // from the router's point of view and bounds retained authority.
            lookup.call = ns_conn.call(ns::OP_LOOKUP, lookup.name).ok();
            continue;
        }

        let result = lookup.call.as_mut().expect("route lookup exists").poll();
        match result {
            Ok(None) => continue,
            Ok(Some(result)) => {
                lookup.call = None;
                if result.result >= 1
                    && let Some(connection) = result.connection
                {
                    routes.push(Route {
                        ethertype: lookup.ethertype,
                        conn: connection,
                        opcode: lookup.opcode,
                    });
                }
            }
            Err(_) => lookup.call = None,
        }
    }
}

/// Peek the EtherType field (bytes 12..14, big-endian) of a frame held in a
/// moved memory object, without consuming the object.
fn read_ethertype(memory: OwnedMemory, frame_len: usize) -> Option<(OwnedMemory, u16)> {
    if frame_len < ETHERNET_HEADER_MIN {
        return None;
    }
    let Ok(mapping) = memory.map_read_only() else {
        return None;
    };
    let field = mapping.as_slice().get(ETHERTYPE_OFFSET..ETHERTYPE_OFFSET + 2)?;
    let ethertype = u16::from_be_bytes([field[0], field[1]]);
    let Ok(memory) = mapping.unmap() else {
        return None;
    };
    Some((memory, ethertype))
}

fn fail() -> ! {
    unsafe { thread_exit() }
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = ctx.bootstrap_connection().unwrap_or_else(|| fail());
    // The NIC driver is mandatory; discovery of the optional consumers may
    // lag behind their registration.
    let mut net_conn = lookup(ns_conn, net::NAME).unwrap_or_else(|| fail());
    let cluster_service = match (ctx.manifest_value(VIP_KEY), ctx.manifest_value(VIP_PORT_KEY)) {
        (Some(ManifestValue::Bytes(address)), Some(ManifestValue::Unsigned(port)))
            if address.len() == 4 && u16::try_from(port).is_ok() =>
        {
            let service =
                ServiceId::tcp_v4([address[0], address[1], address[2], address[3]], port as u16);
            service.is_valid().then_some(service)
        }
        _ => None,
    };
    config::write::<u32>(status::STAGE, 2);

    // Register so other services (notably the httpd report aggregator) can
    // look us up and query our live counters.
    let endpoint =
        Endpoint::create(frouter::INTERFACE, frouter::VERSION, 8).unwrap_or_else(|_| fail());
    let registration = ns_conn
        .call_connection(
            ns::OP_REGISTER,
            frouter::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .unwrap_or_else(|_| fail())
        .wait()
        .unwrap_or_else(|_| fail());
    if registration.result < 1 {
        fail();
    }
    endpoint.bind_completion_queue(0).unwrap_or_else(|_| fail());
    config::write::<u32>(status::STAGE, 3);

    let mut routes: Vec<Route> = Vec::new();
    let mut route_lookups = [
        RouteLookup {
            ethertype: MSG_ETHERTYPE,
            name: relmsg::NAME,
            opcode: relmsg::OP_FRAME,
            call: None,
        },
        RouteLookup {
            ethertype: DISCO_ETHERTYPE,
            name: disco::NAME,
            opcode: disco::OP_FRAME,
            call: None,
        },
        RouteLookup {
            ethertype: raft::ETHERTYPE,
            name: dns::NAME,
            opcode: dns::OP_RAFT_FRAME,
            call: None,
        },
        RouteLookup {
            ethertype: IPV4_ETHERTYPE,
            name: socket::NAME,
            opcode: socket::OP_FRAME,
            call: None,
        },
        RouteLookup {
            ethertype: ARP_ETHERTYPE,
            name: socket::NAME,
            opcode: socket::OP_FRAME,
            call: None,
        },
    ];
    let mut pending_forwards: Vec<PendingForward> = Vec::new();
    let mut pending_l2_sends: Vec<PendingCall<'static>> = Vec::new();
    let mut membership = MembershipClient::new();
    let mut snapshots = SnapshotHistory::new(SNAPSHOT_HISTORY_CAPACITY);
    let mut flows = FlowEpochTable::new(FLOW_EPOCH_CAPACITY);
    let mut membership_timer = Completion::timer(MEMBERSHIP_REFRESH_MS).ok();
    let mut membership_due = true;
    let mut vip_advertiser = None;
    let mut logged_membership_epoch = None;
    refresh_routes(&mut routes, &mut route_lookups, ns_conn);
    config::write::<u32>(status::ROUTES, routes.len() as u32);

    let mut rx_total: u32 = 0;
    let mut forwarded: u32 = 0;
    let mut dropped: u32 = 0;
    let mut unknown: u32 = 0;
    let mut ingress_local: u32 = 0;
    let mut ingress_forwarded: u32 = 0;
    let mut ingress_dropped: u32 = 0;
    let stage: u32 = 4;
    config::write::<u32>(status::STAGE, stage);

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            catten_rt::logln!(
                "[frouter] shutdown: cancelling NIC receive, {} lookup(s), {} protocol \
                 forward(s), and {} L2 send(s)",
                route_lookups.iter().filter(|lookup| lookup.call.is_some()).count(),
                pending_forwards.len(),
                pending_l2_sends.len()
            );
            return request;
        }
        let mut receive = net_conn.call(net::OP_RECV, 0).unwrap_or_else(|_| fail());
        loop {
            if let Some(request) = ctx.lifecycle().shutdown_requested() {
                catten_rt::logln!(
                    "[frouter] shutdown: cancelling NIC receive, {} lookup(s), {} protocol \
                     forward(s), and {} L2 send(s)",
                    route_lookups.iter().filter(|lookup| lookup.call.is_some()).count(),
                    pending_forwards.len(),
                    pending_l2_sends.len()
                );
                return request;
            }
            // Periodic heartbeat (~every 256 reactor iterations) so a stall can
            // be localized: if rx stops advancing here, frames are not reaching
            // the demultiplexer from the NIC driver.
            let tick = HEARTBEAT_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if tick & 0xff == 0 {
                catten_rt::logln!(
                    "[frouter] hb rx={} fwd={} dropped={} unknown={} routes={} ingress={}/{}/{} \
                     epoch={}",
                    rx_total,
                    forwarded,
                    dropped,
                    unknown,
                    routes.len(),
                    ingress_local,
                    ingress_forwarded,
                    ingress_dropped,
                    snapshots.current().map_or(0, |snapshot| snapshot.epoch)
                );
            }
            // Drain our own endpoint so status queries are served even while
            // no frames are flowing (non-blocking).
            loop {
                let Some(mut message) = endpoint.try_receive().unwrap_or_else(|_| fail()) else {
                    break;
                };
                let Some(reply) = message.reply.take() else {
                    continue;
                };
                if message.opcode == frouter::OP_STATUS {
                    let words = [
                        stage,
                        rx_total,
                        forwarded,
                        dropped,
                        unknown,
                        routes.len() as u32,
                        frouter::STATUS_MAGIC,
                        snapshots.current().map_or(0, |snapshot| snapshot.epoch as u32),
                        snapshots.current().map_or(0, |snapshot| (snapshot.epoch >> 32) as u32),
                        snapshots.current().map_or(0, |snapshot| snapshot.backends().len() as u32),
                        snapshots
                            .current()
                            .and_then(BackendSnapshot::vip_advertiser)
                            .map_or(0, |backend| backend.node_id as u32),
                        ingress_local,
                        ingress_forwarded,
                        ingress_dropped,
                        flows.len() as u32,
                        u32::from(vip_advertiser.is_some_and(|node| {
                            snapshots.current().is_some_and(|snapshot| node == snapshot.self_node)
                        })),
                        snapshots.current().map_or(0, |snapshot| snapshot.members().len() as u32),
                    ];
                    let memory = match OwnedMemory::allocate(1) {
                        Ok(memory) => memory,
                        Err(_) => {
                            let _ = reply.reply(frouter::ERR_BAD_OPCODE);
                            continue;
                        }
                    };
                    let mut mapping = match memory.map_writable() {
                        Ok(mapping) => mapping,
                        Err((_, _)) => {
                            let _ = reply.reply(frouter::ERR_BAD_OPCODE);
                            continue;
                        }
                    };
                    for (chunk, word) in mapping
                        .as_mut_slice()
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(words.iter().copied())
                    {
                        chunk.copy_from_slice(&word.to_le_bytes());
                    }
                    let memory = mapping.unmap().unwrap_or_else(|_| fail());
                    let _ = reply.reply_move(memory, (words.len() * 4) as i64);
                } else {
                    let _ = reply.reply(frouter::ERR_BAD_OPCODE);
                }
            }

            // Poll each bounded deferred name-service lookup. A newly
            // registered consumer becomes routable without ever parking this
            // single-owner NIC reactor.
            refresh_routes(&mut routes, &mut route_lookups, ns_conn);
            if let Some(timer) = membership_timer.as_mut() {
                match timer.poll() {
                    Ok(Some(_)) => {
                        membership_timer = Completion::timer(MEMBERSHIP_REFRESH_MS).ok();
                        membership_due = true;
                    }
                    Ok(None) => {}
                    Err(_) => membership_timer = None,
                }
            } else {
                membership_timer = Completion::timer(MEMBERSHIP_REFRESH_MS).ok();
            }
            if cluster_service.is_some()
                && let Some(snapshot) = membership.poll(ns_conn, membership_due, &mut snapshots)
            {
                if let Some(service) = cluster_service {
                    let _ = flows.remove_absent_backends(&service, &snapshots, &snapshot);
                }
                let new_advertiser = snapshot.vip_advertiser().map(|backend| backend.node_id);
                let epoch_changed = logged_membership_epoch != Some(snapshot.epoch);
                if epoch_changed && new_advertiser == Some(snapshot.self_node) {
                    catten_rt::logln!(
                        "[frouter] VIP SNAPSHOT OWNER node={:08x} epoch={} backends={}/{}",
                        snapshot.self_node,
                        snapshot.epoch,
                        snapshot.backends().len(),
                        snapshot.members().len()
                    );
                }
                if (vip_advertiser != new_advertiser || epoch_changed)
                    && new_advertiser == Some(snapshot.self_node)
                    && pending_l2_sends.len() < MAX_PENDING_L2_SENDS
                    && let (Some(service), Some(owner)) =
                        (cluster_service, snapshot.vip_advertiser())
                    && let Some(call) = submit_gratuitous_arp(&net_conn, owner.mac, service.address)
                {
                    catten_rt::logln!(
                        "[frouter] VIP ADVERTISER ACQUIRED node={:08x} epoch={} backends={}/{}",
                        snapshot.self_node,
                        snapshot.epoch,
                        snapshot.backends().len(),
                        snapshot.members().len()
                    );
                    pending_l2_sends.push(call);
                }
                vip_advertiser = new_advertiser;
                logged_membership_epoch = Some(snapshot.epoch);
            }
            if membership.request.is_some() {
                membership_due = false;
            }
            config::write::<u32>(status::RX_TOTAL, rx_total);
            config::write::<u32>(status::FORWARDED, forwarded);
            config::write::<u32>(status::DROPPED, dropped);
            config::write::<u32>(status::UNKNOWN, unknown);
            config::write::<u32>(status::ROUTES, routes.len() as u32);
            config::write::<u32>(
                status::EPOCH_LO,
                snapshots.current().map_or(0, |snapshot| snapshot.epoch as u32),
            );
            config::write::<u32>(
                status::EPOCH_HI,
                snapshots.current().map_or(0, |snapshot| (snapshot.epoch >> 32) as u32),
            );
            config::write::<u32>(
                status::BACKENDS,
                snapshots.current().map_or(0, |snapshot| snapshot.backends().len() as u32),
            );
            config::write::<u32>(
                status::VIP_ADVERTISER,
                snapshots
                    .current()
                    .and_then(BackendSnapshot::vip_advertiser)
                    .map_or(0, |backend| backend.node_id as u32),
            );
            config::write::<u32>(status::INGRESS_LOCAL, ingress_local);
            config::write::<u32>(status::INGRESS_FORWARDED, ingress_forwarded);
            config::write::<u32>(status::INGRESS_DROPPED, ingress_dropped);
            config::write::<u32>(status::FLOW_BINDINGS, flows.len() as u32);
            config::write::<u32>(
                status::IS_ADVERTISER,
                u32::from(vip_advertiser.is_some_and(|node| {
                    snapshots.current().is_some_and(|snapshot| node == snapshot.self_node)
                })),
            );
            config::write::<u32>(
                status::MEMBERS,
                snapshots.current().map_or(0, |snapshot| snapshot.members().len() as u32),
            );

            // Forward calls are asynchronous: one slow protocol consumer must
            // not stop the NIC owner from serving every other EtherType.
            let mut index = 0;
            while index < pending_forwards.len() {
                let result = pending_forwards[index].call.poll();
                let outcome = match result {
                    Ok(None) => {
                        index += 1;
                        continue;
                    }
                    Ok(Some(result)) => Some(result.result),
                    Err(_) => None,
                };
                let pending = pending_forwards.swap_remove(index);
                if outcome.is_some_and(|result| result != catten_syscall::IPC_REPLY_ENDPOINT_CLOSED)
                {
                    forwarded = forwarded.wrapping_add(1);
                } else {
                    dropped = dropped.wrapping_add(1);
                    if let Some(stale_index) =
                        routes.iter().position(|route| route.ethertype == pending.route_ethertype)
                    {
                        routes.remove(stale_index);
                    }
                }
            }

            let mut index = 0;
            while index < pending_l2_sends.len() {
                match pending_l2_sends[index].poll() {
                    Ok(None) => index += 1,
                    Ok(Some(result)) if result.result >= 0 => {
                        let _ = pending_l2_sends.swap_remove(index);
                    }
                    Ok(Some(_)) | Err(_) => {
                        let _ = pending_l2_sends.swap_remove(index);
                        ingress_dropped = ingress_dropped.wrapping_add(1);
                    }
                }
            }

            let received = match receive.poll() {
                Ok(None) => {
                    // No frame yet; yield briefly, then poll again.
                    sleep_ms(ROUTE_RETRY_MS);
                    continue;
                }
                Ok(Some(result)) => result,
                Err(_) => {
                    // A restarted NIC invalidates this connection. Synchronize on
                    // the next registered generation before issuing another
                    // receive instead of polling a terminal call forever.
                    net_conn = lookup(ns_conn, net::NAME).unwrap_or_else(|| fail());
                    break;
                }
            };
            let frame_len = match usize::try_from(received.result) {
                Ok(frame_len) if frame_len <= FRAME_MAX => frame_len,
                _ => break,
            };
            let Some(memory) = received.memory else {
                break;
            };

            rx_total = rx_total.wrapping_add(1);
            let (memory, frame_len, ethertype, decision) = match cluster_service {
                Some(service) => {
                    let Some(result) =
                        classify_ingress(memory, frame_len, service, &snapshots, &mut flows)
                    else {
                        dropped = dropped.wrapping_add(1);
                        break;
                    };
                    result
                }
                None => {
                    let Some((memory, ethertype)) = read_ethertype(memory, frame_len) else {
                        dropped = dropped.wrapping_add(1);
                        break;
                    };
                    (memory, frame_len, ethertype, IngressDecision::Ordinary)
                }
            };
            match decision {
                IngressDecision::Drop => {
                    ingress_dropped = ingress_dropped.wrapping_add(1);
                    break;
                }
                IngressDecision::Remote {
                    ingress,
                    destination,
                } => {
                    if pending_l2_sends.len() >= MAX_PENDING_L2_SENDS {
                        ingress_dropped = ingress_dropped.wrapping_add(1);
                        break;
                    }
                    let mut mapping = match memory.map_writable() {
                        Ok(mapping) => mapping,
                        Err(_) => {
                            ingress_dropped = ingress_dropped.wrapping_add(1);
                            break;
                        }
                    };
                    let Some(forwarded_len) = encapsulate_forwarded_frame(
                        mapping.as_mut_slice(),
                        frame_len,
                        ingress,
                        destination,
                    ) else {
                        ingress_dropped = ingress_dropped.wrapping_add(1);
                        break;
                    };
                    let memory = match mapping.unmap() {
                        Ok(memory) => memory,
                        Err(_) => {
                            ingress_dropped = ingress_dropped.wrapping_add(1);
                            break;
                        }
                    };
                    match net_conn.call_move(net::OP_SEND, forwarded_len as u64, memory) {
                        Ok(call) => {
                            pending_l2_sends.push(call);
                            if ingress_forwarded == 0 {
                                catten_rt::logln!(
                                    "[frouter] FIRST REMOTE VIP FRAME \
                                     backend={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                                    destination[0],
                                    destination[1],
                                    destination[2],
                                    destination[3],
                                    destination[4],
                                    destination[5]
                                );
                            }
                            ingress_forwarded = ingress_forwarded.wrapping_add(1);
                        }
                        Err(_) => ingress_dropped = ingress_dropped.wrapping_add(1),
                    }
                    break;
                }
                IngressDecision::Local => {
                    ingress_local = ingress_local.wrapping_add(1);
                }
                IngressDecision::Ordinary => {}
            }
            let Some(route_index) = routes.iter().position(|route| route.ethertype == ethertype)
            else {
                unknown = unknown.wrapping_add(1);
                break;
            };

            let route_ethertype = routes[route_index].ethertype;
            let route_opcode = routes[route_index].opcode;
            if pending_forwards
                .iter()
                .filter(|pending| pending.route_ethertype == route_ethertype)
                .count()
                >= MAX_PENDING_PER_ROUTE
            {
                // Bound authority and memory retained by a wedged consumer.
                // Closing pending calls cancels their moved frame objects; the
                // name-service retry can install a fresh service generation.
                let mut pending_index = 0;
                while pending_index < pending_forwards.len() {
                    if pending_forwards[pending_index].route_ethertype == route_ethertype {
                        pending_forwards.swap_remove(pending_index);
                        dropped = dropped.wrapping_add(1);
                    } else {
                        pending_index += 1;
                    }
                }
                dropped = dropped.wrapping_add(1);
                routes.remove(route_index);
                break;
            }
            match routes[route_index].conn.call_move(route_opcode, frame_len as u64, memory) {
                Ok(call) => pending_forwards.push(PendingForward {
                    call,
                    route_ethertype,
                }),
                Err((_memory, _)) => {
                    dropped = dropped.wrapping_add(1);
                    routes.remove(route_index);
                }
            }
            break;
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
