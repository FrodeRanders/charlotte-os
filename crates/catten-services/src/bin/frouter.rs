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
//! are optional: a route is only installed once the named service is
//! registered (retried each loop), so the frouter works whether a node runs
//! relmsg, disco, tcpip, both, or neither.
//!
//! Status page layout:
//! - word 0: stage
//! - word 1: rx_total (frames received from the NIC driver)
//! - word 2: forwarded (frames delivered to a consumer)
//! - word 3: dropped (delivery failed)
//! - word 4: unknown (no route for the EtherType)
//! - word 5: routes (installed consumer routes)
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
    raft,
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
use charlotte_protocol_disco::DISCO_ETHERTYPE;
use charlotte_protocol_msg::MSG_ETHERTYPE;

const IPV4_ETHERTYPE: u16 = 0x0800;
const ARP_ETHERTYPE: u16 = 0x0806;

const STAGE_OFFSET: usize = 0;
const RX_TOTAL_OFFSET: usize = 4;
const FORWARDED_OFFSET: usize = 8;
const DROPPED_OFFSET: usize = 12;
const UNKNOWN_OFFSET: usize = 16;
const ROUTES_OFFSET: usize = 20;

const FRAME_MAX: usize = 4096;
const ETHERTYPE_OFFSET: usize = 12;
const ETHERNET_HEADER_MIN: usize = 14;
/// Poll interval for the NIC receive call. The driver's `OP_RECV` reply only
/// completes when a frame arrives, so while no frames are flowing we poll it
/// (instead of blocking) and re-run the optional-consumer route lookups each
/// cycle — otherwise a service that registers after us would never be routed.
const ROUTE_RETRY_MS: u64 = 50;

struct Route {
    ethertype: u16,
    conn: u64,
    opcode: u32,
}

fn lookup(ns_conn: u64, name: u64) -> u64 {
    let call = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, name);
    if call == 0 {
        return 0;
    }
    let (generation, connection) = unsafe { wait_reply(call, 0) };
    if generation < 1 {
        0
    } else {
        connection
    }
}

fn try_lookup(ns_conn: u64, name: u64) -> Option<(i64, u64)> {
    let call = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, name);
    if call == 0 {
        return None;
    }
    let (generation, connection) = unsafe { wait_reply(call, 0) };
    if generation < 1 || connection == 0 {
        None
    } else {
        Some((generation, connection))
    }
}

fn add_route_if_missing(
    routes: &mut Vec<Route>,
    ns_conn: u64,
    ethertype: u16,
    name: u64,
    opcode: u32,
) {
    if routes.iter().any(|route| route.ethertype == ethertype) {
        return;
    }
    if let Some((_generation, conn)) = try_lookup(ns_conn, name) {
        routes.push(Route {
            ethertype,
            conn,
            opcode,
        });
    }
}

/// Peek the EtherType field (bytes 12..14, big-endian) of a frame held in a
/// moved memory object, without consuming the object.
fn read_ethertype(memory: u64, frame_len: usize) -> u16 {
        let (scratch_2_map_status, scratch_2_vaddr) = memory_map_any(memory, false);
    if frame_len < ETHERNET_HEADER_MIN || scratch_2_map_status != 0 {
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
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    // The NIC driver is mandatory; discovery of the optional consumers may
    // lag behind their registration.
    let net_conn = lookup(ns_conn, net::NAME);
    if net_conn == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 2);

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
    let (generation, _) = unsafe { wait_reply(registration, 0) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 3);

    let mut routes: Vec<Route> = Vec::new();
    add_route_if_missing(&mut routes, ns_conn, MSG_ETHERTYPE, relmsg::NAME, relmsg::OP_FRAME);
    add_route_if_missing(&mut routes, ns_conn, DISCO_ETHERTYPE, disco::NAME, disco::OP_FRAME);
    add_route_if_missing(&mut routes, ns_conn, raft::ETHERTYPE, raft::FRAME_NAME, raft::OP_FRAME);
    add_route_if_missing(&mut routes, ns_conn, IPV4_ETHERTYPE, socket::NAME, socket::OP_FRAME);
    add_route_if_missing(&mut routes, ns_conn, ARP_ETHERTYPE, socket::NAME, socket::OP_FRAME);
    config::write::<u32>(ROUTES_OFFSET, routes.len() as u32);

    let mut rx_total: u32 = 0;
    let mut forwarded: u32 = 0;
    let mut dropped: u32 = 0;
    let mut unknown: u32 = 0;
    let stage: u32 = 4;

    loop {
        let receive = ipc_scalar_call(net_conn, net::OP_RECV, 0);
        if receive == 0 {
            unsafe { thread_exit() };
        }
        loop {
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
                    ipc_reply_move(m.reply, cap, (words.len() * 4) as i64);
                } else {
                    ipc_reply(m.reply, frouter::ERR_BAD_OPCODE);
                }
            }

            // Consumers may register after us; refresh the optional routes on
            // every poll so a late-registering service is picked up even when
            // no frames are arriving.
            add_route_if_missing(
                &mut routes,
                ns_conn,
                MSG_ETHERTYPE,
                relmsg::NAME,
                relmsg::OP_FRAME,
            );
            add_route_if_missing(
                &mut routes,
                ns_conn,
                DISCO_ETHERTYPE,
                disco::NAME,
                disco::OP_FRAME,
            );
            add_route_if_missing(
                &mut routes,
                ns_conn,
                IPV4_ETHERTYPE,
                socket::NAME,
                socket::OP_FRAME,
            );
            add_route_if_missing(
                &mut routes,
                ns_conn,
                ARP_ETHERTYPE,
                socket::NAME,
                socket::OP_FRAME,
            );
            config::write::<u32>(RX_TOTAL_OFFSET, rx_total);
            config::write::<u32>(FORWARDED_OFFSET, forwarded);
            config::write::<u32>(DROPPED_OFFSET, dropped);
            config::write::<u32>(UNKNOWN_OFFSET, unknown);
            config::write::<u32>(ROUTES_OFFSET, routes.len() as u32);

            let (status, frame_len, _connection, memory) = ipc_reply_poll_with_memory(receive);
            if status != 0 {
                // No frame yet; yield briefly, then poll again.
                sleep_ms(ROUTE_RETRY_MS);
                continue;
            }
            ipc_close(receive);
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
            let forward = ipc_scalar_call_move(route_conn, route_opcode, frame_len, memory);
            if forward == 0 {
                memory_close(memory);
                dropped = dropped.wrapping_add(1);
                let stale = routes.remove(route_index);
                ipc_close(stale.conn);
            } else {
                let (result, _) = unsafe { wait_reply(forward, 0) };
                if result == catten_syscall::IPC_REPLY_ENDPOINT_CLOSED {
                    dropped = dropped.wrapping_add(1);
                    let stale = routes.remove(route_index);
                    ipc_close(stale.conn);
                } else {
                    forwarded = forwarded.wrapping_add(1);
                }
            }
            break;
        }
    }
}

catten_rt::entry!(main);
