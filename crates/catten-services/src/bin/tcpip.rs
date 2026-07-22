//! The CharlotteOS TCP/IP service (smoltcp-powered).
//!
//! Bootstraps, looks up the NIC driver ("net0"), initialises a smoltcp
//! interface, registers a "tcpip" endpoint with the name service, and
//! enters a poll loop that handles both NIC frames and socket-API
//! client requests.
//!
//! ## Socket protocol
//!
//! Clients call `OP_SOCKET`, `OP_CONNECT`, `OP_SEND`, `OP_RECV` (deferred
//! reply), and `OP_CLOSE` on the tcpip connection. Data payloads use
//! memory-object transfer. See [`catten_services::socket`].
#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    net,
    ns,
    socket,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    thread_exit,
};
use charlotte_protocol_net::decode_status;
use charlotte_smoltcp::CharlotteEthDevice;
use smoltcp::{
    iface::{
        Config,
        Interface,
        SocketSet,
    },
    socket::tcp::{
        self,
        Socket as TcpSocket,
        SocketBuffer as TcpSocketBuffer,
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
const LOOKUP_ATTEMPTS: u64 = 2_000_000;
const STAGE_OFFSET: usize = 0;
const SCRATCH_VADDR: usize = 0x0000_0000_0080_0000;

struct SocketEntry {
    handle: smoltcp::iface::SocketHandle,
    recv_pending: Option<u64>,
}

struct TcpipState {
    sockets: BTreeMap<u64, SocketEntry>,
    next_sock_id: u64,
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
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_connection = match ctx.bootstrap_cap() {
        Some(c) => c,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2);

    let net_conn = {
        let mut attempts = 0u64;
        loop {
            let l = ipc_scalar_call(ns_connection, ns::OP_LOOKUP, net::NAME);
            if l != 0 {
                let (r, cap) = unsafe { wait_reply(l, REPLY_SPINS) };
                if r >= 1 && cap != 0 {
                    break cap;
                }
            }
            attempts += 1;
            if attempts >= LOOKUP_ATTEMPTS {
                unsafe { thread_exit() };
            }
            core::hint::spin_loop();
        }
    };
    config::write::<u32>(STAGE_OFFSET, 3);

    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        unsafe { thread_exit() };
    }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    let (link, mac) = decode_status(status);
    let mtu: usize = 1500;
    config::write::<u32>(STAGE_OFFSET, 4);

    let ep = ipc_endpoint_create(socket::INTERFACE, socket::VERSION, 8);
    if ep == 0 {
        unsafe { thread_exit() };
    }
    let reg = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        socket::NAME,
        ep,
        IpcRights::SEND | IpcRights::CALL,
    );
    if reg == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { wait_reply(reg, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    if ipc_endpoint_bind_cq(ep, 0) != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 5);

    let mut device = CharlotteEthDevice::new(net_conn, mac, mtu, ep);
    let hw = HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress(mac));
    let mut cfg = Config::new(hw);
    cfg.random_seed = 0x0123_4567_89ab_cdef;
    let local_ip = Ipv4Address::new(10, 0, 2, 15);
    let gateway = Ipv4Address::new(10, 0, 2, 2);
    let mut iface = Interface::new(cfg, &mut device, Instant::from_millis(0));
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(local_ip, 24)));
    });
    iface.routes_mut().add_default_ipv4_route(gateway).ok();
    let mut sock_storage: [_; 16] = Default::default();
    let mut sockets = SocketSet::new(&mut sock_storage[..]);
    let mut state = TcpipState {
        sockets: BTreeMap::new(),
        next_sock_id: 1,
    };
    let mut ticks: u64 = 0;
    config::write::<u32>(STAGE_OFFSET, 6);

    loop {
        device.poll_smoltcp(&mut iface, &mut sockets, &mut ticks);

        // Complete any ready recv operations.
        let mut completed: [u64; 8] = [0; 8];
        let mut completed_n: usize = 0;
        for (id, entry) in state.sockets.iter() {
            if let Some(reply_token) = entry.recv_pending {
                let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                if sock.can_recv() {
                    let cap = memory_alloc(1);
                    if cap == 0 {
                        continue;
                    }
                    if memory_map(cap, SCRATCH_VADDR, true) != 0 {
                        memory_close(cap);
                        continue;
                    }
                    let buf =
                        unsafe { core::slice::from_raw_parts_mut(SCRATCH_VADDR as *mut u8, 4096) };
                    match sock.recv_slice(buf) {
                        Ok(0) => {
                            memory_unmap(cap);
                            memory_close(cap);
                        }
                        Ok(len) => {
                            memory_unmap(cap);
                            ipc_reply_move(reply_token, cap, len as i64);
                            if completed_n < 8 {
                                completed[completed_n] = *id;
                                completed_n += 1;
                            }
                        }
                        Err(_) => {
                            memory_unmap(cap);
                            memory_close(cap);
                        }
                    }
                }
            }
        }
        for i in 0..completed_n {
            if let Some(entry) = state.sockets.get_mut(&completed[i]) {
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
                    if msg.arg0 != socket::DOMAIN_TCP {
                        ipc_reply(msg.reply, socket::ERR_BAD_DOMAIN);
                        continue;
                    }
                    if state.sockets.len() >= socket::MAX_SOCKETS {
                        ipc_reply(msg.reply, socket::ERR_TOO_MANY_SOCKETS);
                        continue;
                    }
                    let rx = TcpSocketBuffer::new(alloc::vec![0u8; 4096]);
                    let tx = TcpSocketBuffer::new(alloc::vec![0u8; 4096]);
                    let tcp = TcpSocket::new(rx, tx);
                    let handle = sockets.add(tcp);
                    let id = state.alloc_sock_id();
                    state.sockets.insert(
                        id,
                        SocketEntry {
                            handle,
                            recv_pending: None,
                        },
                    );
                    ipc_reply(msg.reply, id as i64);
                }

                socket::OP_CONNECT => {
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
                    memory_map(msg.memory, SCRATCH_VADDR, false);
                    let a = unsafe { core::ptr::read_volatile(SCRATCH_VADDR as *const u8) };
                    let b = unsafe { core::ptr::read_volatile((SCRATCH_VADDR + 1) as *const u8) };
                    let c = unsafe { core::ptr::read_volatile((SCRATCH_VADDR + 2) as *const u8) };
                    let d = unsafe { core::ptr::read_volatile((SCRATCH_VADDR + 3) as *const u8) };
                    let port =
                        unsafe { core::ptr::read_unaligned((SCRATCH_VADDR + 4) as *const u16) };
                    memory_unmap(msg.memory);
                    memory_close(msg.memory);
                    let remote =
                        IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::new(a, b, c, d)), port);
                    let local = IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), 0);
                    let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                    match sock.connect(iface.context(), remote, local) {
                        Ok(()) => ipc_reply(msg.reply, 0),
                        Err(_) => ipc_reply(msg.reply, socket::ERR_CONNECTION_REFUSED),
                    };
                }

                socket::OP_SEND => {
                    let entry = match state.sockets.get_mut(&msg.arg0) {
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
                    memory_map(msg.memory, SCRATCH_VADDR, false);
                    let data =
                        unsafe { core::slice::from_raw_parts(SCRATCH_VADDR as *const u8, 4096) };
                    let sock = sockets.get_mut::<TcpSocket>(entry.handle);
                    let result = match sock.send_slice(data) {
                        Ok(len) => len as i64,
                        Err(_) => socket::ERR_WOULD_BLOCK,
                    };
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
                    if let Some(entry) = state.sockets.remove(&msg.arg0) {
                        if let Some(token) = entry.recv_pending {
                            ipc_reply(token, 0);
                        }
                        sockets.remove(entry.handle);
                    }
                    ipc_reply(msg.reply, 0);
                }

                _ => {
                    ipc_reply(msg.reply, socket::ERR_BAD_OPCODE);
                }
            }
        }

        cq_wait(1, 0);
    }
}

catten_rt::entry!(main);
