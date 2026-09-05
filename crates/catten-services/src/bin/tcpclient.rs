//! Two-node TCP/IP smoke client over the tcpip service (smoltcp).
//!
//! Self-configuring: derives its local IPv4 address and role from its NIC
//! MAC (read via `net OP_STATUS`) so both guests run identical code. The node
//! with an even last MAC octet is the server (listens, echoes), the odd one
//! is the client (connects, sends). Peer address is the local address with
//! the last octet toggled.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use catten_rt::{
    Context,
    ManifestValue,
    config,
    owned::{
        ConnectionRef,
        OwnedMemory,
    },
};
use catten_services::{
    frouter,
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
const VIP_KEY: u64 = charlotte_launch::manifest_key(b"vip");
const VIP_PORT_KEY: u64 = charlotte_launch::manifest_key(b"vipport");
const VIP_PROBE_REQUEST: &[u8] = b"GET /metrics HTTP/1.0\r\n\r\n";
const VIP_PROBE_FLOWS: usize = 8;
const VIP_PROBE_SETTLE_MS: u64 = 20_000;
const VIP_MEMBERSHIP_CONVERGE_MS: u64 = 5_000;
const VIP_ADVERTISER_FIXTURE_HOLD_MS: u64 = 120_000;
const EXPECTED_MEMBERS_KEY: u64 = charlotte_launch::manifest_key(b"members");

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
    send_payload_attempts(tcp_conn, socket, data, 300)
}

fn send_payload_attempts(
    tcp_conn: ConnectionRef<'_>,
    socket: &socket::OwnedSocket<'_>,
    data: &[u8],
    attempts: usize,
) -> bool {
    for _ in 0..attempts {
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

fn wait_for_ingress_members(ns_conn: ConnectionRef<'_>, expected: u32) -> Option<bool> {
    let Some((_, connection)) = wait_for_registered_name_owned(ns_conn, frouter::NAME) else {
        return None;
    };
    for _ in 0..1_200 {
        let Ok(call) = connection.call(frouter::OP_STATUS, 0) else {
            return None;
        };
        if let Ok(result) = call.wait()
            && result.result >= (frouter::STATUS_WORDS * 4) as i64
            && let Some(memory) = result.memory
            && let Ok(mapping) = memory.map_read_only()
        {
            let backends_offset = frouter::STATUS_OFFSET_BACKENDS as usize * 4;
            let advertiser_offset = frouter::STATUS_OFFSET_IS_ADVERTISER as usize * 4;
            if let (Some(backends), Some(advertiser)) = (
                mapping.as_slice().get(backends_offset..backends_offset + 4),
                mapping.as_slice().get(advertiser_offset..advertiser_offset + 4),
            ) && u32::from_le_bytes(backends.try_into().unwrap_or([0; 4])) >= expected
            {
                return Some(u32::from_le_bytes(advertiser.try_into().unwrap_or([0; 4])) != 0);
            }
        }
        sleep_ms(ACCEPT_POLL_MS);
    }
    None
}

fn receive_http(socket: &socket::OwnedSocket<'_>) -> bool {
    let Ok(mut receive) = socket.call(socket::OP_RECV, socket.id()) else {
        return false;
    };
    for _ in 0..50 {
        match receive.poll() {
            Ok(None) => sleep_ms(ACCEPT_POLL_MS),
            Ok(Some(received)) => {
                let Some(memory) = received.memory else {
                    return false;
                };
                let Ok(mapping) = memory.map_read_only() else {
                    return false;
                };
                let length =
                    usize::try_from(received.result).unwrap_or(0).min(mapping.as_slice().len());
                return mapping.as_slice()[..length].starts_with(b"HTTP/1.1 200");
            }
            Err(_) => return false,
        }
    }
    false
}

fn probe_cluster_vip(tcp_conn: ConnectionRef<'_>, address: [u8; 4], port: u16) -> bool {
    let mut sockets = Vec::with_capacity(VIP_PROBE_FLOWS);
    for _ in 0..VIP_PROBE_FLOWS {
        let Ok(socket) = socket::OwnedSocket::open(tcp_conn, socket::DOMAIN_TCP) else {
            return false;
        };
        let Ok(address_memory) = OwnedMemory::allocate(1) else {
            return false;
        };
        let Ok(mut mapping) = address_memory.map_writable() else {
            return false;
        };
        mapping.as_mut_slice()[..4].copy_from_slice(&address);
        mapping.as_mut_slice()[4..6].copy_from_slice(&port.to_le_bytes());
        let Ok(address_memory) = mapping.unmap() else {
            return false;
        };
        let Ok(connect) = tcp_conn.call_move(socket::OP_CONNECT, socket.id(), address_memory)
        else {
            return false;
        };
        if connect.wait().ok().map(|result| result.result) != Some(0) {
            return false;
        }
        sockets.push(socket);
    }

    // tcpip accepts OP_CONNECT once the nonblocking handshake starts. Poll all
    // candidates together so a backend's deliberately small HTTP listener can
    // accept whichever rendezvous winners reach it; refused candidates do not
    // invalidate connections that are already established on other backends.
    let mut established = Vec::with_capacity(VIP_PROBE_FLOWS);
    for _ in 0..300 {
        let mut index = 0;
        while index < sockets.len() {
            match sockets[index].connection_state() {
                Ok(socket::CONNECTION_STATE_ESTABLISHED) => {
                    established.push(sockets.swap_remove(index));
                }
                Ok(socket::CONNECTION_STATE_CONNECTING) => index += 1,
                Ok(_) | Err(_) => {
                    let socket = sockets.swap_remove(index);
                    let _ = socket.close();
                }
            }
        }
        if sockets.is_empty() {
            break;
        }
        sleep_ms(ACCEPT_POLL_MS);
    }
    for socket in sockets {
        let _ = socket.close();
    }
    catten_rt::logln!(
        "[ingress-probe] FAILOVER WINDOW OPEN with {} established flow(s)",
        established.len()
    );
    sleep_ms(VIP_PROBE_SETTLE_MS);

    let mut successful = 0usize;
    for socket in established {
        if send_payload_attempts(tcp_conn, &socket, VIP_PROBE_REQUEST, 20) && receive_http(&socket)
        {
            successful += 1;
        }
        let _ = socket.close();
    }
    catten_rt::logln!("[ingress-probe] {} flow(s) survived the failover window", successful);
    true
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

    if let (Some(ManifestValue::Bytes(address)), Some(ManifestValue::Unsigned(port))) =
        (ctx.manifest_value(VIP_KEY), ctx.manifest_value(VIP_PORT_KEY))
        && address.len() == 4
        && let Ok(port) = u16::try_from(port)
    {
        let expected = match ctx.manifest_value(EXPECTED_MEMBERS_KEY) {
            Some(ManifestValue::Unsigned(value)) => u32::try_from(value).unwrap_or(u32::MAX),
            _ => 1,
        };
        let is_advertiser =
            wait_for_ingress_members(ns_conn, expected).unwrap_or_else(|| fail(0xe01f));
        // A leader can observe a committed configuration just before the
        // commit notification reaches every follower. The forwarding envelope
        // prevents a transient second selection, while this short pause keeps
        // the failover fixture focused on stable-state distribution.
        sleep_ms(VIP_MEMBERSHIP_CONVERGE_MS);
        if is_advertiser {
            // A NIC does not normally loop a locally transmitted frame back
            // into its own receive queue. The VIP owner therefore cannot act
            // as a faithful external client of the L2 address it advertises.
            // Keep its kernel alive until the external fixture removes it;
            // otherwise successful completion would stop QEMU before the
            // non-advertiser guests have established their test flows.
            catten_rt::logln!("[ingress-probe] PROBE SKIPPED ON VIP ADVERTISER");
            sleep_ms(VIP_ADVERTISER_FIXTURE_HOLD_MS);
        } else if !probe_cluster_vip(
            tcp_conn.as_ref(),
            [address[0], address[1], address[2], address[3]],
            port,
        ) {
            fail(0xe020);
        }
        config::write::<u32>(status::LOCAL_IP, u32::from_be_bytes([10, 0, 0, local_octet]));
        config::write::<u32>(status::STAGE, SENTINEL);
        unsafe { thread_exit() };
    }

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
