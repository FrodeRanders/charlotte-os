//! Two-node TCP/IP smoke client over the tcpip service (smoltcp).
//!
//! Self-configuring: derives its local IPv4 address and role from its NIC
//! MAC (read via `net OP_STATUS`) so both guests run identical code. The node
//! with an even last MAC octet is the server (listens, echoes), the odd one
//! is the client (connects, sends). Peer address is the local address with
//! the last octet toggled.
#![no_std]
#![no_main]

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    net,
    ns,
    sleep_ms,
    socket,
    wait_for_local_ready,
    wait_reply,
};
use catten_syscall::{
    ipc_reply_wait_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_move,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_net::decode_status;

const SCRATCH: usize = 0x0000_0000_00b0_0000;
const SENTINEL: u32 = 0x5345_4e54;
const PAYLOAD: &[u8] = b"CharlotteOS tcpip cross-node";
const PORT: u16 = 8080;
const ACCEPT_POLL_MS: u64 = 100;

fn fail(code: u32) -> ! {
    config::write::<u32>(8, code);
    unsafe { thread_exit() };
}

fn ip_from_mac(mac: &[u8; 6]) -> (u8, u8) {
    let last = mac[5];
    let local = 100u8.wrapping_add(last % 100);
    let peer = 100u8.wrapping_add((last ^ 1) % 100);
    (local, peer)
}

/// Send a payload, retrying while the tcpip service reports `ERR_WOULD_BLOCK`
/// (the connection is not established yet or the transmit buffer is full).
fn send_payload(tcp_conn: u64, sock_id: u64, data: &[u8]) -> bool {
    for _ in 0..300 {
        let cap = memory_alloc(1);
        if cap == 0 || memory_map(cap, SCRATCH, true) != 0 {
            if cap != 0 {
                memory_close(cap);
            }
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), SCRATCH as *mut u8, data.len());
        }
        memory_unmap(cap);
        let send = ipc_scalar_call_move(
            tcp_conn,
            socket::OP_SEND,
            ((data.len() as u64) << 32) | (sock_id & 0xffff_ffff),
            cap,
        );
        if send == 0 {
            memory_close(cap);
            return false;
        }
        let (sent, _) = unsafe { wait_reply(send, 0) };
        if sent == data.len() as i64 {
            return true;
        }
        if sent != socket::ERR_WOULD_BLOCK {
            return false;
        }
        sleep_ms(ACCEPT_POLL_MS);
    }
    false
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(0, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (generation, net_conn) = unsafe { wait_reply(lookup, 0) };
    if generation < 1 || net_conn == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 2);

    let status = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status == 0 {
        fail(0xe001);
    }
    let (status, _) = unsafe { wait_reply(status, 0) };
    let (_link, mac) = decode_status(status);
    let (local_octet, peer_octet) = ip_from_mac(&mac);
    let is_server = local_octet % 2 == 0;
    config::write::<u32>(0, 3);

    let tcp_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, socket::NAME);
    if tcp_lookup == 0 {
        fail(0xe002);
    }
    let (generation, tcp_conn) = unsafe { wait_reply(tcp_lookup, 0) };
    if generation < 1 || tcp_conn == 0 {
        fail(0xe003);
    }
    config::write::<u32>(0, 4);

    if !wait_for_local_ready(ns_conn) {
        fail(0xe004);
    }
    config::write::<u32>(0, 5);

    let sock_call = ipc_scalar_call(tcp_conn, socket::OP_SOCKET, socket::DOMAIN_TCP);
    if sock_call == 0 {
        fail(0xe005);
    }
    let (sock_id, _) = unsafe { wait_reply(sock_call, 0) };
    if sock_id < 1 {
        fail(0xe006);
    }
    config::write::<u32>(0, 6);

    if is_server {
        // ---- Server: listen, accept, receive, echo. ----
        let port_cap = memory_alloc(1);
        if port_cap == 0 || memory_map(port_cap, SCRATCH, true) != 0 {
            if port_cap != 0 {
                memory_close(port_cap);
            }
            fail(0xe007);
        }
        unsafe { core::ptr::write_unaligned(SCRATCH as *mut u16, PORT.to_le()) }
        memory_unmap(port_cap);
        let listen = ipc_scalar_call_move(tcp_conn, socket::OP_LISTEN, sock_id as u64, port_cap);
        if listen == 0 {
            memory_close(port_cap);
            fail(0xe008);
        }
        let (result, _) = unsafe { wait_reply(listen, 0) };
        if result != 0 {
            fail(0xe009);
        }
        config::write::<u32>(0, 7);

        let mut accepted = false;
        for _ in 0..600 {
            let accept = ipc_scalar_call(tcp_conn, socket::OP_ACCEPT, sock_id as u64);
            if accept == 0 {
                fail(0xe00a);
            }
            let (result, _) = unsafe { wait_reply(accept, 0) };
            if result != 0 && result != socket::ERR_WOULD_BLOCK {
                fail(0xe00b);
            }
            if result == 0 {
                accepted = true;
                break;
            }
            sleep_ms(ACCEPT_POLL_MS);
        }
        if !accepted {
            fail(0xe00c);
        }
        config::write::<u32>(0, 8);

        let recv = ipc_scalar_call(tcp_conn, socket::OP_RECV, sock_id as u64);
        if recv == 0 {
            fail(0xe00d);
        }
        let (status, len, _connection, memory) = ipc_reply_wait_with_memory(recv);
        if status != 0 || memory == 0 || len as usize != PAYLOAD.len() {
            if memory != 0 {
                memory_close(memory);
            }
            fail(0xe00e);
        }
        if memory_map(memory, SCRATCH, false) != 0 {
            memory_close(memory);
            fail(0xe00f);
        }
        let received = unsafe { core::slice::from_raw_parts(SCRATCH as *const u8, PAYLOAD.len()) };
        let matches = received == PAYLOAD;
        memory_unmap(memory);
        memory_close(memory);
        if !matches {
            fail(0xe010);
        }
        config::write::<u32>(0, 9);

        if !send_payload(tcp_conn, sock_id as u64, PAYLOAD) {
            fail(0xe013);
        }
        config::write::<u32>(0, 10);
    } else {
        // ---- Client: connect, send, receive echo. ----
        let addr_cap = memory_alloc(1);
        if addr_cap == 0 || memory_map(addr_cap, SCRATCH, true) != 0 {
            if addr_cap != 0 {
                memory_close(addr_cap);
            }
            fail(0xe014);
        }
        let peer_ip = [10, 0, 0, peer_octet];
        unsafe {
            core::ptr::copy_nonoverlapping(peer_ip.as_ptr(), SCRATCH as *mut u8, 4);
            core::ptr::write_unaligned((SCRATCH + 4) as *mut u16, PORT.to_le());
        }
        memory_unmap(addr_cap);
        let connect = ipc_scalar_call_move(tcp_conn, socket::OP_CONNECT, sock_id as u64, addr_cap);
        if connect == 0 {
            memory_close(addr_cap);
            fail(0xe015);
        }
        let (result, _) = unsafe { wait_reply(connect, 0) };
        if result != 0 {
            fail(0xe016);
        }
        config::write::<u32>(0, 7);

        if !send_payload(tcp_conn, sock_id as u64, PAYLOAD) {
            fail(0xe018);
        }
        config::write::<u32>(0, 8);

        let recv = ipc_scalar_call(tcp_conn, socket::OP_RECV, sock_id as u64);
        if recv == 0 {
            fail(0xe01a);
        }
        let (status, len, _connection, memory) = ipc_reply_wait_with_memory(recv);
        if status != 0 || memory == 0 || len as usize != PAYLOAD.len() {
            if memory != 0 {
                memory_close(memory);
            }
            fail(0xe01b);
        }
        if memory_map(memory, SCRATCH, false) != 0 {
            memory_close(memory);
            fail(0xe01c);
        }
        let echoed = unsafe { core::slice::from_raw_parts(SCRATCH as *const u8, PAYLOAD.len()) };
        let matches = echoed == PAYLOAD;
        memory_unmap(memory);
        memory_close(memory);
        if !matches {
            fail(0xe01d);
        }
        config::write::<u32>(0, 9);
    }

    let close = ipc_scalar_call(tcp_conn, socket::OP_CLOSE, sock_id as u64);
    if close != 0 {
        let _ = unsafe { wait_reply(close, 0) };
    }

    config::write::<u32>(4, u32::from_be_bytes([10, 0, 0, local_octet]));
    config::write::<u32>(0, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
