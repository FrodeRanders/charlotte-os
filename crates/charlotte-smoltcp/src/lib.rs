//! smoltcp → CharlotteOS adapter for the NIC driver protocol.
//!
//! Implements `smoltcp::phy::Device` over a connection to a CharlotteOS NIC
//! driver endpoint (`net0` via the name service).  The adapter translates
//! smoltcp's poll-driven `receive()`/`transmit()` calls into the driver's
//! `OP_RECV` (deferred receive) and `OP_SEND` (moved-memory transmit).
//!
//! ## Usage
//!
//! ```ignore
//! let mut device = CharlotteEthDevice::new(net_conn, mac, 1500);
//! let mut iface = smoltcp::iface::Interface::new(config, &mut device);
//! loop {
//!     iface.poll(Instant::from_millis(ticks), &mut device, &mut sockets);
//!     ticks += 1;  // crude ~1 ms monotonic clock
//! }
//! ```
//!
//! Memory model: `OP_RECV` returns a *moved* memory object holding the
//! received frame — the RxToken maps it, copies the bytes to smoltcp, and
//! returns the page to the kernel.  `OP_SEND` moves a freshly allocated
//! page (filled by smoltcp) to the driver — the TxToken allocates, maps,
//! and sends it.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use charlotte_protocol_net::{OP_RECV, OP_SEND, OP_STATUS, decode_status};
use catten_syscall::{
    IpcMessage, ipc_recv, ipc_reply, ipc_reply_poll, ipc_scalar_call,
    ipc_scalar_call_move,
    memory_alloc, memory_close, memory_map, memory_unmap,
    ipc_status,
};

use smoltcp::phy::{Device, RxToken, TxToken};

/// Scratch virtual address for mapping received frames into.
const RX_SCRATCH: usize = 0x0000_0000_00c0_0000;
/// Scratch virtual address for building transmit frames.
const TX_SCRATCH: usize = 0x0000_0000_00c0_1000;

pub struct CharlotteEthDevice {
    /// Connection capability to the NIC driver endpoint.
    conn: u64,
    /// Pending-call cap for an outstanding `OP_RECV`, or 0.
    rx_pending: u64,
    /// The endpoint of the NIC driver, for draining notifications.
    endpoint: u64,
    /// Cached device properties.
    mac: [u8; 6],
    mtu: usize,
    /// Whether we're inside a poll — avoid re-entrant IPC.
    in_poll: bool,
}

pub struct CharlotteRx {
    /// The memory cap returned by the NIC driver for a received frame.
    frame_cap: u64,
}

pub struct CharlotteTx {
    /// The NIC driver connection for sending.
    conn: u64,
}

impl CharlotteEthDevice {
    /// Create a new adapter.  `conn` is a connection cap to the NIC driver
    /// endpoint.  `mac` and `mtu` come from `OP_STATUS`.  `endpoint` is the
    /// driver's endpoint cap for draining readiness notifications.
    pub fn new(conn: u64, mac: [u8; 6], mtu: usize, endpoint: u64) -> Self {
        Self {
            conn, endpoint, mac, mtu,
            rx_pending: 0,
            in_poll: false,
        }
    }

    /// smoltcp calls this repeatedly in a tight loop.  We poll the driver
    /// for received frames (non-blocking) and return them if available.
    pub fn poll_smoltcp(
        &mut self,
        iface: &mut smoltcp::iface::Interface,
        sockets: &mut smoltcp::iface::SocketSet,
        ticks: &mut u64,
    ) {
        // Drain any incoming frames or wakeup notifications from the
        // driver's completion queue.
        self.drain_notifications();

        *ticks += 1;
        let now = Instant::from_millis((*ticks) as i64);
        iface.poll(now, self, sockets);
    }

    /// Drain endpoint readiness notifications from the driver's CQ-bound
    /// endpoint.  The driver sends an empty wake for each completed receive
    /// or transmit — we just need to consume them so the ring doesn't fill.
    fn drain_notifications(&mut self) {
        loop {
            let m = unsafe { ipc_recv(self.endpoint) };
            if m.status == ipc_status::NO_MESSAGE {
                break;
            }
            // The driver doesn't send endpoint messages to us; it uses CQ
            // wakeup.  If we get a message, reply with stub and discard.
            if m.reply != 0 {
                unsafe { ipc_reply(m.reply, 0) };
            }
        }
    }
}

impl Device for CharlotteEthDevice {
    type RxToken<'a> = CharlotteRx where Self: 'a;
    type TxToken<'a> = CharlotteTx where Self: 'a;

    fn receive(&mut self, _now: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // 1. Issue a new OP_RECV if none is outstanding.
        if self.rx_pending == 0 {
            self.rx_pending =
                unsafe { ipc_scalar_call(self.conn, OP_RECV, 0) };
            if self.rx_pending == 0 {
                return None; // driver rejected the call
            }
            return None; // wait for the reply
        }

        // 2. Poll the pending OP_RECV.
        let (status, _result, _cap) = unsafe { ipc_reply_poll(self.rx_pending) };
        if status != 0 {
            return None; // still pending
        }

        // 3. OP_RECV completed.  The reply carries a moved memory cap with
        //    the received frame.  ipc_reply_poll doesn't return the memory
        //    cap directly; we need the full poll.  Let's use
        //    ipc_reply_poll_with_memory from catten-syscall.
        //    Actually, the wrapper returns (0, result, returned_connection,
        //    returned_memory).  Let's re-poll with memory.
        #[allow(unused_assignments)]
        {
            let (s2, _, _, mem) =
                unsafe { catten_syscall::ipc_reply_poll_with_memory(self.rx_pending) };
            if s2 != 0 || mem == 0 {
                self.rx_pending = 0;
                return None;
            }
            let rx = mem;
            let tx = self.conn;
            self.rx_pending = 0;
            // Issue the next receive immediately (pipelining).
            self.rx_pending =
                unsafe { ipc_scalar_call(self.conn, OP_RECV, 0) };
            Some((
                CharlotteRx { frame_cap: rx },
                CharlotteTx { conn: tx },
            ))
        }
    }

    fn transmit(&mut self, _now: Instant) -> Option<Self::TxToken<'_>> {
        Some(CharlotteTx { conn: self.conn })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = self.mtu;
        caps.medium = smoltcp::phy::Medium::Ethernet;
        caps
    }
}

impl RxToken for CharlotteRx {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let cap = self.frame_cap;
        if unsafe { memory_map(cap, RX_SCRATCH, false) } != 0 {
            return f(&[]);
        }
        let result = f(unsafe {
            core::slice::from_raw_parts(RX_SCRATCH as *const u8, 2048)
        });
        unsafe { memory_unmap(cap) };
        unsafe { memory_close(cap) };
        result
    }
}

impl TxToken for CharlotteTx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let cap = unsafe { memory_alloc(1) };
        if cap == 0 {
            let mut empty = [0u8; 0];
            return f(&mut empty[..]);
        }
        if unsafe { memory_map(cap, TX_SCRATCH, true) } != 0 {
            unsafe { memory_close(cap) };
            let mut empty = [0u8; 0];
            return f(&mut empty[..]);
        }
        let buf = unsafe {
            core::slice::from_raw_parts_mut(TX_SCRATCH as *mut u8, len)
        };
        let result = f(buf);
        unsafe { memory_unmap(cap) };
        unsafe { ipc_scalar_call_move(self.conn, OP_SEND, len as u64, cap) };
        result
    }
}
