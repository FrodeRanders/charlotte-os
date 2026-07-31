//! Minimal hardcoded HTTP server exposing a node's observable state.
//!
//! Listens on TCP port 80 through the tcpip service, and for each connection
//! reads the request, then replies with a small JSON document aggregating
//! state that the services already collect:
//!
//! ```json
//! {
//!   "node":  { "mac": "52:54:00:12:34:56", "link": 1 },
//!   "tcpip": { "ip": "10.0.2.15", "rx_frames": N, "tx_sends": N,
//!              "sockets": N, "listen_port": 80 },
//!   "http":  { "requests": N, "uptime": N }
//! }
//! ```
//!
//! This is a deliberate keyhole, not a web server: no routing, no keep-alive,
//! one connection at a time. It exists so a host (or another node) can peek
//! into a CharlotteOS guest over the NIC.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::fmt::Write as _;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    net,
    ns,
    socket,
    sleep_ms,
    wait_for_boot_done,
    wait_reply,
};
use catten_syscall::{
    ipc_close,
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

const SCRATCH: usize = 0x0000_0000_00e0_0000;
const HTTP_PORT: u16 = 80;
const ACCEPT_POLL_MS: u64 = 50;
const SENTINEL: u32 = 0x4854_5450; // "HTTP"

fn fail(code: u32) -> ! {
    config::write::<u32>(8, code);
    unsafe { thread_exit() };
}

/// Send a payload, retrying while the tcpip service reports `ERR_WOULD_BLOCK`.
fn send_all(tcp_conn: u64, sock_id: u64, data: &[u8]) -> bool {
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

/// Query the tcpip service for its packed `TcpipStatus` snapshot.
fn tcpip_status(tcp_conn: u64) -> [u32; 5] {
    let call = ipc_scalar_call(tcp_conn, socket::OP_STATUS, 0);
    if call == 0 {
        return [0; 5];
    }
    let (status, _len, _connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if status != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return [0; 5];
    }
    if memory_map(memory, SCRATCH, false) != 0 {
        memory_close(memory);
        return [0; 5];
    }
    let mut words = [0u32; 5];
    unsafe {
        core::ptr::copy_nonoverlapping(SCRATCH as *const u32, words.as_mut_ptr(), words.len());
    }
    memory_unmap(memory);
    memory_close(memory);
    words
}

fn build_json(mac: &[u8; 6], link: u8, tcp_conn: u64, requests: u32, uptime: u32) -> String {
    let status = tcpip_status(tcp_conn);
    let ip = status[socket::STATUS_OFFSET_IP as usize];
    let rx = status[socket::STATUS_OFFSET_RX_FRAMES as usize];
    let tx = status[socket::STATUS_OFFSET_TX_SENDS as usize];
    let socks = status[socket::STATUS_OFFSET_SOCKETS as usize];
    let mut s = String::new();
    let _ = write!(
        &mut s,
        "{{\"node\":{{\"mac\":\"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\",\"link\":{}}},\
         \"tcpip\":{{\"ip\":\"{}.{}.{}.{}\",\"rx_frames\":{},\"tx_sends\":{},\"sockets\":{},\
         \"listen_port\":{}}},\"http\":{{\"requests\":{},\"uptime\":{}}}}}",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5],
        link,
        (ip >> 24) & 0xff,
        (ip >> 16) & 0xff,
        (ip >> 8) & 0xff,
        ip & 0xff,
        rx,
        tx,
        socks,
        HTTP_PORT,
        requests,
        uptime
    );
    s
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
    let (link, mac) = decode_status(status);
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

    if !wait_for_boot_done(ns_conn) {
        fail(0xe004);
    }
    config::write::<u32>(0, 5);

    let mut requests: u32 = 0;
    let mut uptime: u32 = 0;
    loop {
        // Fresh socket + listener per connection (smoltcp's listening socket
        // becomes the established connection, then returns to Closed).
        let sock_call = ipc_scalar_call(tcp_conn, socket::OP_SOCKET, socket::DOMAIN_TCP);
        if sock_call == 0 {
            fail(0xe005);
        }
        let (sock_id, _) = unsafe { wait_reply(sock_call, 0) };
        if sock_id < 1 {
            fail(0xe006);
        }
        let port_cap = memory_alloc(1);
        if port_cap == 0 || memory_map(port_cap, SCRATCH, true) != 0 {
            if port_cap != 0 {
                memory_close(port_cap);
            }
            fail(0xe007);
        }
        unsafe { core::ptr::write_unaligned(SCRATCH as *mut u16, HTTP_PORT.to_le()) }
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
        config::write::<u32>(0, 6);

        let mut accepted = false;
        for _ in 0..3000 {
            let accept = ipc_scalar_call(tcp_conn, socket::OP_ACCEPT, sock_id as u64);
            if accept == 0 {
                fail(0xe00a);
            }
            let (result, _) = unsafe { wait_reply(accept, 0) };
            if result == 0 {
                accepted = true;
                break;
            }
            if result != socket::ERR_WOULD_BLOCK {
                fail(0xe00b);
            }
            sleep_ms(ACCEPT_POLL_MS);
        }
        if !accepted {
            fail(0xe00c);
        }

        // Read whatever request arrived (the response is hardcoded state, so
        // even a partial request is fine).
        let recv = ipc_scalar_call(tcp_conn, socket::OP_RECV, sock_id as u64);
        if recv == 0 {
            fail(0xe00d);
        }
        let (status, _len, _connection, memory) = ipc_reply_wait_with_memory(recv);
        ipc_close(recv);
        if status != 0 || memory == 0 {
            if memory != 0 {
                memory_close(memory);
            }
            fail(0xe00e);
        }
        if memory_map(memory, SCRATCH, false) != 0 {
            memory_close(memory);
            fail(0xe00f);
        }
        memory_unmap(memory);
        memory_close(memory);

        let body = build_json(&mac, link, tcp_conn, requests, uptime);
        let mut response = String::new();
        response.push_str("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ");
        let _ = write!(response, "{}", body.len());
        response.push_str("\r\nConnection: close\r\n\r\n");
        response.push_str(&body);
        if !send_all(tcp_conn, sock_id as u64, response.as_bytes()) {
            fail(0xe010);
        }

        let close = ipc_scalar_call(tcp_conn, socket::OP_CLOSE, sock_id as u64);
        if close != 0 {
            let _ = unsafe { wait_reply(close, 0) };
        }

        requests = requests.wrapping_add(1);
        uptime = uptime.wrapping_add(1);
        config::write::<u32>(4, requests);
        config::write::<u32>(0, SENTINEL);
    }
}

catten_rt::entry!(main);
