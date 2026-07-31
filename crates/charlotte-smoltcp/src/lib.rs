//! smoltcp → CharlotteOS adapter for the NIC driver protocol.
//!
//! Implements `smoltcp::phy::Device` over a connection to a CharlotteOS NIC
//! driver endpoint (`net0` via the name service).  The adapter translates
//! smoltcp's poll-driven `receive()`/`transmit()` calls into the driver's
//! `OP_SEND` (moved-memory transmit) and a queue of received frames.
//!
//! The receive path does *not* issue the driver's `OP_RECV`: that deferred
//! receive slot is owned exclusively by the frouter, which demultiplexes
//! frames by EtherType and delivers IP/ARP frames to the TCP/IP service via
//! its `OP_FRAME` ingress.  The service copies each forwarded frame into
//! [`CharlotteEthDevice::push_rx`]; `receive()` then hands those bytes to
//! smoltcp.
//!
//! ## Usage
//!
//! ```ignore
//! let mut device = CharlotteEthDevice::new(net_conn, 1500);
//! let mut iface = smoltcp::iface::Interface::new(config, &mut device);
//! loop {
//!     // ... on socket::OP_FRAME: device.push_rx(frame) ...
//!     device.poll_smoltcp(&mut iface, &mut sockets, &mut ticks, elapsed_ms);
//! }
//! ```
//!
//! Memory model: `OP_SEND` moves a freshly allocated page (filled by
//! smoltcp) to the driver — the TxToken allocates, maps, and sends it.

#![no_std]

extern crate alloc;

use alloc::collections::VecDeque;

use catten_syscall::{
    ipc_close,
    ipc_reply_wait,
    ipc_scalar_call_move,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
};
use charlotte_protocol_net::OP_SEND;
use smoltcp::{
    phy::{
        Device,
        DeviceCapabilities,
        RxToken,
        TxToken,
    },
    time::Instant,
};

/// Scratch virtual address for building transmit frames.
const TX_SCRATCH: usize = 0x0000_0000_00c0_1000;

pub struct CharlotteEthDevice {
    /// Connection capability to the NIC driver endpoint.
    conn: u64,
    mtu: usize,
    /// Frames delivered through the service's `OP_FRAME` ingress (from the
    /// frouter) awaiting consumption by smoltcp.
    rx: VecDeque<alloc::vec::Vec<u8>>,
}

pub struct CharlotteRx {
    frame: alloc::vec::Vec<u8>,
}

pub struct CharlotteTx {
    /// The NIC driver connection for sending.
    conn: u64,
}

impl CharlotteEthDevice {
    /// Create a new adapter.  `conn` is a connection cap to the NIC driver
    /// endpoint; `mtu` comes from `OP_STATUS`.
    pub fn new(conn: u64, mtu: usize) -> Self {
        Self {
            conn,
            mtu,
            rx: VecDeque::new(),
        }
    }

    /// Push a received frame (delivered by the frouter through the service's
    /// `OP_FRAME` ingress) onto the receive queue for smoltcp to consume.
    pub fn push_rx(&mut self, frame: alloc::vec::Vec<u8>) {
        if !frame.is_empty() && frame.len() <= 4096 {
            self.rx.push_back(frame);
        }
    }

    /// Number of frames queued and not yet consumed by smoltcp.
    pub fn rx_len(&self) -> usize {
        self.rx.len()
    }

    /// smoltcp calls this repeatedly.  We poll the driver-independent frame
    /// queue and advance the monotonic clock by the elapsed milliseconds.
    pub fn poll_smoltcp(
        &mut self,
        iface: &mut smoltcp::iface::Interface,
        sockets: &mut smoltcp::iface::SocketSet,
        ticks: &mut u64,
        elapsed_ms: u64,
    ) {
        advance_ticks(ticks, elapsed_ms);
        let now = Instant::from_millis(*ticks as i64);
        iface.poll(now, self, sockets);
    }
}

/// Advance the crudely simulated monotonic clock by `elapsed_ms`.
fn advance_ticks(ticks: &mut u64, elapsed_ms: u64) {
    *ticks = ticks.saturating_add(elapsed_ms);
}

impl Device for CharlotteEthDevice {
    type RxToken<'a>
        = CharlotteRx
    where
        Self: 'a;
    type TxToken<'a>
        = CharlotteTx
    where
        Self: 'a;

    fn receive(&mut self, _now: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let frame = self.rx.pop_front()?;
        Some((
            CharlotteRx {
                frame,
            },
            CharlotteTx {
                conn: self.conn,
            },
        ))
    }

    fn transmit(&mut self, _now: Instant) -> Option<Self::TxToken<'_>> {
        Some(CharlotteTx {
            conn: self.conn,
        })
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
        f(&self.frame)
    }
}

impl TxToken for CharlotteTx {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        if len > 4096 {
            let mut empty = [0u8; 0];
            return f(&mut empty);
        }
        let cap = memory_alloc(1);
        if cap == 0 {
            let mut empty = [0u8; 0];
            return f(&mut empty[..]);
        }
        if memory_map(cap, TX_SCRATCH, true) != 0 {
            memory_close(cap);
            let mut empty = [0u8; 0];
            return f(&mut empty[..]);
        }
        let buf = unsafe { core::slice::from_raw_parts_mut(TX_SCRATCH as *mut u8, len) };
        let result = f(buf);
        memory_unmap(cap);
        let call = ipc_scalar_call_move(self.conn, OP_SEND, len as u64, cap);
        if call == 0 {
            memory_close(cap);
        } else {
            // Reap the driver's reply so the pending-call slot is recycled.
            let _ = ipc_reply_wait(call);
            ipc_close(call);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::CharlotteEthDevice;

    #[test]
    fn rx_queue_bounds_and_order() {
        let mut device = CharlotteEthDevice::new(0, 1500);
        let now = smoltcp::time::Instant::from_millis(0);
        assert_eq!(device.rx_len(), 0);
        assert!(device.receive(now).is_none());

        device.push_rx(alloc::vec![1u8; 64]);
        device.push_rx(alloc::vec![2u8; 64]);
        device.push_rx(alloc::vec![]);
        device.push_rx(alloc::vec![3u8; 8192]);
        assert_eq!(device.rx_len(), 2);

        let (rx, _tx) = device.receive(now).unwrap();
        rx.consume(|frame| {
            assert_eq!(frame.len(), 64);
            assert_eq!(frame[0], 1);
        });
        assert_eq!(device.rx_len(), 1);
    }

    #[test]
    fn ticks_advance_by_elapsed() {
        use super::advance_ticks;
        let mut ticks = 0u64;
        advance_ticks(&mut ticks, 50);
        assert_eq!(ticks, 50);
        advance_ticks(&mut ticks, 1);
        assert_eq!(ticks, 51);
    }
}
