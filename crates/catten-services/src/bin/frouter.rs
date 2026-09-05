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
    ShutdownRequest,
    config,
    owned::{
        Connection,
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        PendingCall,
    },
};
use catten_services::{
    disco,
    frouter,
    net,
    ns,
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
    refresh_routes(&mut routes, &mut route_lookups, ns_conn);
    config::write::<u32>(status::ROUTES, routes.len() as u32);

    let mut rx_total: u32 = 0;
    let mut forwarded: u32 = 0;
    let mut dropped: u32 = 0;
    let mut unknown: u32 = 0;
    let stage: u32 = 4;
    config::write::<u32>(status::STAGE, stage);

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return request;
        }
        let mut receive = net_conn.call(net::OP_RECV, 0).unwrap_or_else(|_| fail());
        loop {
            if let Some(request) = ctx.lifecycle().shutdown_requested() {
                return request;
            }
            // Periodic heartbeat (~every 256 reactor iterations) so a stall can
            // be localized: if rx stops advancing here, frames are not reaching
            // the demultiplexer from the NIC driver.
            let tick = HEARTBEAT_TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if tick & 0xff == 0 {
                catten_rt::logln!(
                    "[frouter] hb rx={} fwd={} dropped={} unknown={} routes={}",
                    rx_total,
                    forwarded,
                    dropped,
                    unknown,
                    routes.len()
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
            config::write::<u32>(status::RX_TOTAL, rx_total);
            config::write::<u32>(status::FORWARDED, forwarded);
            config::write::<u32>(status::DROPPED, dropped);
            config::write::<u32>(status::UNKNOWN, unknown);
            config::write::<u32>(status::ROUTES, routes.len() as u32);

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
            let Some((memory, ethertype)) = read_ethertype(memory, frame_len) else {
                dropped = dropped.wrapping_add(1);
                break;
            };
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
