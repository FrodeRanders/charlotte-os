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
    owned::{
        ConnectionRef,
        OwnedMemory,
    },
};
use catten_services::{
    net,
    owned_call_with_backpressure,
    sleep_ms,
    socket,
    wait_for_local_ready_owned,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::tcpclient_status as status;
use charlotte_protocol_net::decode_status;

const SENTINEL: u32 = 0x5345_4e54;
const PAYLOAD: &[u8] = b"CharlotteOS tcpip cross-node";
const PORT: u16 = 8080;
const ACCEPT_POLL_MS: u64 = 100;

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
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
fn send_payload(
    tcp_conn: ConnectionRef<'_>,
    socket: &socket::OwnedSocket<'_>,
    data: &[u8],
) -> bool {
    for _ in 0..300 {
        let Ok(memory) = OwnedMemory::allocate(1) else {
            return false;
        };
        let Ok(mut mapping) = memory.map_writable() else {
            return false;
        };
        mapping.as_mut_slice()[..data.len()].copy_from_slice(data);
        let Ok(memory) = mapping.unmap() else {
            return false;
        };
        let packed = ((data.len() as u64) << 32) | (socket.id() & 0xffff_ffff);
        let Ok(send) = tcp_conn.call_move(socket::OP_SEND, packed, memory) else {
            return false;
        };
        let Ok(sent) = send.wait().map(|result| result.result) else {
            return false;
        };
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
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    let (_, net_conn) = wait_for_registered_name_owned(ns_conn, net::NAME)
        .unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 2);

    let net_status = owned_call_with_backpressure(net_conn.as_ref(), net::OP_STATUS, 0)
        .wait()
        .unwrap_or_else(|_| unsafe { thread_exit() })
        .result;
    let (_link, mac) = decode_status(net_status);
    let (local_octet, peer_octet) = ip_from_mac(&mac);
    let is_server = local_octet % 2 == 0;
    config::write::<u32>(status::STAGE, 3);

    let (_, tcp_conn) =
        wait_for_registered_name_owned(ns_conn, socket::NAME).unwrap_or_else(|| fail(0xe003));
    config::write::<u32>(status::STAGE, 4);

    if !wait_for_local_ready_owned(ns_conn) {
        fail(0xe004);
    }
    config::write::<u32>(status::STAGE, 5);

    let socket = socket::OwnedSocket::open(tcp_conn.as_ref(), socket::DOMAIN_TCP)
        .unwrap_or_else(|_| fail(0xe005));
    config::write::<u32>(status::STAGE, 6);

    if is_server {
        // ---- Server: listen, accept, receive, echo. ----
        let port_memory = OwnedMemory::allocate(1).unwrap_or_else(|_| fail(0xe007));
        let mut port_mapping = port_memory.map_writable().unwrap_or_else(|_| fail(0xe007));
        port_mapping.as_mut_slice()[..2].copy_from_slice(&PORT.to_le_bytes());
        let port_memory = port_mapping.unmap().unwrap_or_else(|_| fail(0xe007));
        let result = tcp_conn
            .call_move(socket::OP_LISTEN, socket.id(), port_memory)
            .unwrap_or_else(|_| fail(0xe008))
            .wait()
            .unwrap_or_else(|_| fail(0xe008))
            .result;
        if result != 0 {
            fail(0xe009);
        }
        config::write::<u32>(status::STAGE, 7);

        let mut accepted = false;
        for _ in 0..600 {
            let result = socket
                .call(socket::OP_ACCEPT, socket.id())
                .unwrap_or_else(|_| fail(0xe00a))
                .wait()
                .unwrap_or_else(|_| fail(0xe00a))
                .result;
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
        config::write::<u32>(status::STAGE, 8);

        let recv = socket
            .call(socket::OP_RECV, socket.id())
            .unwrap_or_else(|_| fail(0xe00d))
            .wait()
            .unwrap_or_else(|_| fail(0xe00e));
        if recv.result as usize != PAYLOAD.len() {
            fail(0xe00e);
        }
        let memory = recv.memory.unwrap_or_else(|| fail(0xe00e));
        let mapping = memory.map_read_only().unwrap_or_else(|_| fail(0xe00f));
        let matches = &mapping.as_slice()[..PAYLOAD.len()] == PAYLOAD;
        if !matches {
            fail(0xe010);
        }
        config::write::<u32>(status::STAGE, 9);

        if !send_payload(tcp_conn.as_ref(), &socket, PAYLOAD) {
            fail(0xe013);
        }
        config::write::<u32>(status::STAGE, 10);
    } else {
        // ---- Client: connect, send, receive echo. ----
        let address_memory = OwnedMemory::allocate(1).unwrap_or_else(|_| fail(0xe014));
        let mut address_mapping = address_memory.map_writable().unwrap_or_else(|_| fail(0xe014));
        let peer_ip = [10, 0, 0, peer_octet];
        address_mapping.as_mut_slice()[..4].copy_from_slice(&peer_ip);
        address_mapping.as_mut_slice()[4..6].copy_from_slice(&PORT.to_le_bytes());
        let address_memory = address_mapping.unmap().unwrap_or_else(|_| fail(0xe014));
        let result = tcp_conn
            .call_move(socket::OP_CONNECT, socket.id(), address_memory)
            .unwrap_or_else(|_| fail(0xe015))
            .wait()
            .unwrap_or_else(|_| fail(0xe015))
            .result;
        if result != 0 {
            fail(0xe016);
        }
        config::write::<u32>(status::STAGE, 7);

        if !send_payload(tcp_conn.as_ref(), &socket, PAYLOAD) {
            fail(0xe018);
        }
        config::write::<u32>(status::STAGE, 8);

        let recv = socket
            .call(socket::OP_RECV, socket.id())
            .unwrap_or_else(|_| fail(0xe01a))
            .wait()
            .unwrap_or_else(|_| fail(0xe01b));
        if recv.result as usize != PAYLOAD.len() {
            fail(0xe01b);
        }
        let memory = recv.memory.unwrap_or_else(|| fail(0xe01b));
        let mapping = memory.map_read_only().unwrap_or_else(|_| fail(0xe01c));
        let matches = &mapping.as_slice()[..PAYLOAD.len()] == PAYLOAD;
        if !matches {
            fail(0xe01d);
        }
        config::write::<u32>(status::STAGE, 9);
    }

    let _ = socket.close();

    config::write::<u32>(status::LOCAL_IP, u32::from_be_bytes([10, 0, 0, local_octet]));
    config::write::<u32>(status::STAGE, SENTINEL);
    unsafe { thread_exit() };
}

catten_rt::entry!(main);
