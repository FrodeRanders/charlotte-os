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
    config,
};
use catten_services::{
    disco,
    frouter,
    net,
    ns,
    relmsg,
    sleep_ms,
    socket,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_reply_poll,
    ipc_reply_poll_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_scalar_call_move,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
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
    conn: u64,
    opcode: u32,
}

struct RouteLookup {
    ethertype: u16,
    name: u64,
    opcode: u32,
    call: u64,
}

struct PendingForward {
    call: u64,
    route_conn: u64,
}

fn lookup(ns_conn: u64, name: u64) -> u64 {
    let call = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, name);
    if call == 0 {
        return 0;
    }
    let (generation, connection) = unsafe { wait_reply(call, 0) };
    if generation < 1 || connection == 0 {
        if connection != 0 {
            ipc_close(connection);
        }
        0
    } else {
        connection
    }
}

fn refresh_routes(routes: &mut Vec<Route>, lookups: &mut [RouteLookup], ns_conn: u64) {
    for lookup in lookups {
        if routes.iter().any(|route| route.ethertype == lookup.ethertype) {
            continue;
        }

        if lookup.call == 0 {
            // OP_LOOKUP intentionally remains pending while the service is
            // absent. This is the name service's synchronization mechanism;
            // keeping the call in this small fixed table makes it asynchronous
            // from the router's point of view and bounds retained authority.
            lookup.call = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, lookup.name);
            continue;
        }

        let (status, generation, connection) = ipc_reply_poll(lookup.call);
        if status == 1 {
            continue;
        }

        ipc_close(lookup.call);
        lookup.call = 0;
        if status == 0 && generation >= 1 && connection != 0 {
            routes.push(Route {
                ethertype: lookup.ethertype,
                conn: connection,
                opcode: lookup.opcode,
            });
        } else if connection != 0 {
            ipc_close(connection);
        }
    }
}

/// Peek the EtherType field (bytes 12..14, big-endian) of a frame held in a
/// moved memory object, without consuming the object.
fn read_ethertype(memory: u64, frame_len: usize) -> u16 {
    if frame_len < ETHERNET_HEADER_MIN {
        return 0;
    }
    let (scratch_2_map_status, scratch_2_vaddr) = memory_map_any(memory, false);
    if scratch_2_map_status != 0 {
        return 0;
    }
    let ethertype = unsafe {
        let base = scratch_2_vaddr as *const u8;
        u16::from_be_bytes([
            core::ptr::read_volatile(base.add(ETHERTYPE_OFFSET)),
            core::ptr::read_volatile(base.add(ETHERTYPE_OFFSET + 1)),
        ])
    };
    let _ = memory_unmap(memory);
    ethertype
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    // The NIC driver is mandatory; discovery of the optional consumers may
    // lag behind their registration.
    let mut net_conn = lookup(ns_conn, net::NAME);
    if net_conn == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 2);

    // Register so other services (notably the httpd report aggregator) can
    // look us up and query our live counters.
    let ep = ipc_endpoint_create(frouter::INTERFACE, frouter::VERSION, 8);
    if ep == 0 {
        unsafe { thread_exit() };
    }
    let registration = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        frouter::NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if registration == 0 {
        unsafe { thread_exit() };
    }
    let (generation, returned_connection) = unsafe { wait_reply(registration, 0) };
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    if generation < 1 {
        unsafe { thread_exit() };
    }
    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 3);

    let mut routes: Vec<Route> = Vec::new();
    let mut route_lookups = [
        RouteLookup {
            ethertype: MSG_ETHERTYPE,
            name: relmsg::NAME,
            opcode: relmsg::OP_FRAME,
            call: 0,
        },
        RouteLookup {
            ethertype: DISCO_ETHERTYPE,
            name: disco::NAME,
            opcode: disco::OP_FRAME,
            call: 0,
        },
        RouteLookup {
            ethertype: IPV4_ETHERTYPE,
            name: socket::NAME,
            opcode: socket::OP_FRAME,
            call: 0,
        },
        RouteLookup {
            ethertype: ARP_ETHERTYPE,
            name: socket::NAME,
            opcode: socket::OP_FRAME,
            call: 0,
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
        let receive = ipc_scalar_call(net_conn, net::OP_RECV, 0);
        if receive == 0 {
            unsafe { thread_exit() };
        }
        loop {
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
                let m = ipc_recv(ep);
                if m.status == ipc_status::NO_MESSAGE {
                    break;
                }
                if m.status == ipc_status::ENDPOINT_CLOSED {
                    unsafe { thread_exit() };
                }
                if !m.is_ok() || m.reply == 0 {
                    continue;
                }
                if m.opcode == frouter::OP_STATUS {
                    let cap = memory_alloc(1);
                    if cap == 0 {
                        ipc_reply(m.reply, frouter::ERR_BAD_OPCODE);
                        continue;
                    }
                    let (scratch_map_status, scratch_vaddr) = memory_map_any(cap, true);
                    if scratch_map_status != 0 {
                        memory_close(cap);
                        ipc_reply(m.reply, frouter::ERR_BAD_OPCODE);
                        continue;
                    }
                    let words = [
                        stage,
                        rx_total,
                        forwarded,
                        dropped,
                        unknown,
                        routes.len() as u32,
                        frouter::STATUS_MAGIC,
                    ];
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            words.as_ptr(),
                            scratch_vaddr as *mut u32,
                            words.len(),
                        );
                    }
                    memory_unmap(cap);
                    if ipc_reply_move(m.reply, cap, (words.len() * 4) as i64) != 0 {
                        memory_close(cap);
                    }
                } else {
                    ipc_reply(m.reply, frouter::ERR_BAD_OPCODE);
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
                let pending = &pending_forwards[index];
                let (status, result, returned_cap) = ipc_reply_poll(pending.call);
                if status == 1 {
                    index += 1;
                    continue;
                }
                let pending = pending_forwards.swap_remove(index);
                ipc_close(pending.call);
                if returned_cap != 0 {
                    ipc_close(returned_cap);
                }
                if status == 0 && result as i64 != catten_syscall::IPC_REPLY_ENDPOINT_CLOSED {
                    forwarded = forwarded.wrapping_add(1);
                } else {
                    dropped = dropped.wrapping_add(1);
                    if let Some(stale_index) =
                        routes.iter().position(|route| route.conn == pending.route_conn)
                    {
                        let stale = routes.remove(stale_index);
                        ipc_close(stale.conn);
                    }
                }
            }

            let (status, frame_len, connection, memory) = ipc_reply_poll_with_memory(receive);
            if status == 1 {
                // No frame yet; yield briefly, then poll again.
                sleep_ms(ROUTE_RETRY_MS);
                continue;
            }
            ipc_close(receive);
            if connection != 0 {
                ipc_close(connection);
            }
            if status != 0 {
                if memory != 0 {
                    memory_close(memory);
                }
                // A restarted NIC invalidates this connection. Synchronize on
                // the next registered generation before issuing another
                // receive instead of polling a terminal call forever.
                ipc_close(net_conn);
                net_conn = lookup(ns_conn, net::NAME);
                if net_conn == 0 {
                    unsafe { thread_exit() };
                }
                break;
            }
            if memory == 0 || frame_len > FRAME_MAX as u64 {
                if memory != 0 {
                    memory_close(memory);
                }
                break;
            }

            rx_total = rx_total.wrapping_add(1);
            let ethertype = read_ethertype(memory, frame_len as usize);
            let Some(route_index) = routes.iter().position(|route| route.ethertype == ethertype)
            else {
                unknown = unknown.wrapping_add(1);
                memory_close(memory);
                break;
            };

            let route_conn = routes[route_index].conn;
            let route_opcode = routes[route_index].opcode;
            if pending_forwards.iter().filter(|pending| pending.route_conn == route_conn).count()
                >= MAX_PENDING_PER_ROUTE
            {
                // Bound authority and memory retained by a wedged consumer.
                // Closing pending calls cancels their moved frame objects; the
                // name-service retry can install a fresh service generation.
                let mut pending_index = 0;
                while pending_index < pending_forwards.len() {
                    if pending_forwards[pending_index].route_conn == route_conn {
                        let pending = pending_forwards.swap_remove(pending_index);
                        ipc_close(pending.call);
                        dropped = dropped.wrapping_add(1);
                    } else {
                        pending_index += 1;
                    }
                }
                memory_close(memory);
                dropped = dropped.wrapping_add(1);
                let stale = routes.remove(route_index);
                ipc_close(stale.conn);
                break;
            }
            let forward = ipc_scalar_call_move(route_conn, route_opcode, frame_len, memory);
            if forward == 0 {
                memory_close(memory);
                dropped = dropped.wrapping_add(1);
                let stale = routes.remove(route_index);
                ipc_close(stale.conn);
            } else {
                pending_forwards.push(PendingForward {
                    call: forward,
                    route_conn,
                });
            }
            break;
        }
    }
}

catten_rt::entry!(main);
