//! The CharlotteOS TCP/IP compatibility service (smoltcp-powered).
//!
//! Bootstraps, looks up the NIC driver ("net0"), reads its MAC + MTU,
//! initialises a smoltcp interface on the adapter, registers a "tcpip"
//! endpoint, and enters a poll loop.  The interface configuration (IP
//! address, gateway) is hardcoded for the QEMU SLIRP default network
//! (10.0.2.0/24, host 10.0.2.2).
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;

use catten_rt::{Args, Input, config};
use catten_services::{net, ns, wait_reply};
use catten_syscall::{
    IpcRights,
    cq_wait,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    thread_exit,
};
use charlotte_smoltcp::CharlotteEthDevice;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::Device;
use smoltcp::wire::{HardwareAddress, IpCidr, Ipv4Address, Ipv4Cidr};

const REPLY_SPINS: u64 = 50_000_000;
const LOOKUP_ATTEMPTS: u64 = 2_000_000;
const STAGE_OFFSET: usize = 0;

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1);
    let ns_connection = match config::bootstrap_cap() {
        Some(c) => c, None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2);

    // Look up the NIC driver.
    let net_conn = {
        let mut attempts = 0u64;
        loop {
            let l = ipc_scalar_call(ns_connection, ns::OP_LOOKUP, net::NAME);
            if l != 0 {
                let (r, cap) = unsafe { wait_reply(l, REPLY_SPINS) };
                if r >= 1 && cap != 0 { break cap; }
            }
            attempts += 1;
            if attempts >= LOOKUP_ATTEMPTS { unsafe { thread_exit() }; }
            core::hint::spin_loop();
        }
    };
    config::write::<u32>(STAGE_OFFSET, 3);

    // Query driver status.
    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 { unsafe { thread_exit() }; }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    let (link, mac) = charlotte_protocol_net::decode_status(status);
    config::write::<u32>(4, link as u32);
    for i in 0..6 { config::write::<u8>(8 + i, mac[i]); }
    let mtu: usize = 1500;
    config::write::<u32>(STAGE_OFFSET, 4);

    // Create our own endpoint for future socket-API clients.
    let ep = ipc_endpoint_create(0x54435021, 1, 8);
    // Register with the name service.
    let tcpip_name: u64 = {
        let mut packed = [0u8; 8];
        packed[0] = b't'; packed[1] = b'c'; packed[2] = b'p'; packed[3] = b'i'; packed[4] = b'p';
        u64::from_le_bytes(packed)
    };
    let reg = ipc_scalar_call_connection(ns_connection, ns::OP_REGISTER, tcpip_name, ep,
            IpcRights::SEND | IpcRights::CALL);
    if reg == 0 { unsafe { thread_exit() }; }
    let (generation, _) = unsafe { wait_reply(reg, REPLY_SPINS) };
    if generation < 1 { unsafe { thread_exit() }; }

    if ipc_endpoint_bind_cq(ep, 0) != 0 { unsafe { thread_exit() }; }
    config::write::<u32>(STAGE_OFFSET, 5); // registered, serving

    // --- smoltcp setup ---
    let mut device = CharlotteEthDevice::new(net_conn, mac, mtu, ep);
    let hw = HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress(mac));
    let mut config = Config::new(hw);
    config.random_seed = 0x0123_4567_89ab_cdef; // fixed for determinism

    // QEMU SLIRP default: 10.0.2.0/24, host at 10.0.2.2, DNS at 10.0.2.3
    let local_ip = Ipv4Address::new(10, 0, 2, 15);
    let gateway = Ipv4Address::new(10, 0, 2, 2);

    let mut iface = Interface::new(config, &mut device, Instant::from_millis(0));
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(local_ip, 24)));
    });
    iface.routes_mut().add_default_ipv4_route(gateway).ok();
    let mut sock_storage: [_; 4] = Default::default();
    let mut sockets = SocketSet::new(&mut sock_storage[..]);
    let mut ticks: u64 = 0;

    config::write::<u32>(STAGE_OFFSET, 6); // smoltcp initialised

    // --- poll loop ---
    loop {
        device.poll_smoltcp(&mut iface, &mut sockets, &mut ticks);

        // Yield: block until the driver wakes us (endpoint readiness or
        // a frame arrives).  A short timeout ensures we poll sockets
        // even when no traffic arrives.
        cq_wait(1, 0);
    }
}

catten_rt::entry!(cmain);
