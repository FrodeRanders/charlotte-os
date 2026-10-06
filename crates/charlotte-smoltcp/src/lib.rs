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
//!     // ... on socket::OP_FRAME: device.push_rx(&frame) ...
//!     device.poll_smoltcp(&mut iface, &mut sockets, now_ms);
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
/// A request flood cannot postpone packet processing or owner reclamation
/// indefinitely by keeping the shared IPC endpoint nonempty.
pub const MAX_IPC_REQUESTS_PER_CYCLE: usize = 16;
pub const RX_QUEUE_MAX_FRAMES: usize = 32;
pub const RX_QUEUE_MAX_BYTES: usize = 64 * 1024;

/// Derive protocol time from the trusted counter, never from wake/timer counts.
pub struct ReactorClock {
    origin: u64,
    frequency: u64,
    last_ms: u64,
}

impl ReactorClock {
    pub fn new(origin: u64, frequency: u64) -> Option<Self> {
        (frequency != 0).then_some(Self {
            origin,
            frequency,
            last_ms: 0,
        })
    }

    pub fn sample(&mut self, counter: u64, frequency: u64) -> Option<u64> {
        if frequency != self.frequency {
            return None;
        }
        let delta = counter.checked_sub(self.origin)?;
        let ms = (u128::from(delta) * 1000 / u128::from(frequency)).min(i64::MAX as u128) as u64;
        if ms < self.last_ms {
            return None;
        }
        self.last_ms = ms;
        Some(ms)
    }
}

const TX_SCRATCH: usize = 0x0000_0000_00c0_1000;

pub struct CharlotteEthDevice {
    /// Connection capability to the NIC driver endpoint.
    conn: u64,
    mtu: usize,
    /// Frames delivered through the service's `OP_FRAME` ingress (from the
    /// frouter) awaiting consumption by smoltcp.
    rx: VecDeque<alloc::vec::Vec<u8>>,
    rx_bytes: usize,
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
            rx_bytes: 0,
        }
    }

    /// Push a received frame (delivered by the frouter through the service's
    /// `OP_FRAME` ingress) onto the receive queue for smoltcp to consume.
    /// Admission precedes copying: rejected traffic cannot allocate another
    /// frame or grow queue metadata. Allocation pressure drops one packet.
    pub fn push_rx(&mut self, frame: &[u8]) -> bool {
        if frame.is_empty()
            || frame.len() > 4096
            || self.rx.len() >= RX_QUEUE_MAX_FRAMES
            || self.rx_bytes.saturating_add(frame.len()) > RX_QUEUE_MAX_BYTES
        {
            return false;
        }
        let mut owned = alloc::vec::Vec::new();
        if self.rx.try_reserve(1).is_err() || owned.try_reserve_exact(frame.len()).is_err() {
            return false;
        }
        owned.extend_from_slice(frame);
        self.rx_bytes += owned.len();
        self.rx.push_back(owned);
        true
    }

    /// Number of frames queued and not yet consumed by smoltcp.
    pub fn rx_len(&self) -> usize {
        self.rx.len()
    }

    /// Poll a bounded frame backlog using the absolute sampled monotonic time.
    pub fn poll_smoltcp(
        &mut self,
        iface: &mut smoltcp::iface::Interface,
        sockets: &mut smoltcp::iface::SocketSet,
        now_ms: u64,
    ) {
        let now = Instant::from_millis(now_ms.min(i64::MAX as u64) as i64);
        iface.poll(now, self, sockets);
    }
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
        self.rx_bytes -= frame.len();
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
    use smoltcp::phy::{
        Device,
        RxToken,
    };

    use super::CharlotteEthDevice;

    #[test]
    fn rx_queue_bounds_and_order() {
        let mut device = CharlotteEthDevice::new(0, 1500);
        let now = smoltcp::time::Instant::from_millis(0);
        assert_eq!(device.rx_len(), 0);
        assert!(device.receive(now).is_none());

        device.push_rx(&alloc::vec![1u8; 64]);
        device.push_rx(&alloc::vec![2u8; 64]);
        device.push_rx(&alloc::vec![]);
        device.push_rx(&alloc::vec![3u8; 8192]);
        assert_eq!(device.rx_len(), 2);

        let (rx, _tx) = device.receive(now).unwrap();
        rx.consume(|frame| {
            assert_eq!(frame.len(), 64);
            assert_eq!(frame[0], 1);
        });
        assert_eq!(device.rx_len(), 1);
    }

    #[test]
    fn sustained_ingress_keeps_count_and_byte_bounds_and_recovers_capacity() {
        let now = smoltcp::time::Instant::from_millis(0);
        for size in [64, 4096] {
            let mut device = CharlotteEthDevice::new(0, 1500);
            let frame = alloc::vec![0x55; size];
            let capacity = super::RX_QUEUE_MAX_FRAMES.min(super::RX_QUEUE_MAX_BYTES / size);
            for _ in 0..capacity {
                assert!(device.push_rx(&frame));
            }
            for _ in 0..4096 {
                assert!(!device.push_rx(&frame));
            }
            assert_eq!(device.rx_len(), capacity);
            assert_eq!(device.rx_bytes, capacity * size);
            for _ in 0..4096 {
                device.receive(now).unwrap();
                assert!(device.push_rx(&frame));
                assert!(!device.push_rx(&frame));
            }
            for _ in 0..capacity {
                device.receive(now).unwrap();
            }
            assert_eq!(device.rx_bytes, 0);
            assert!(device.push_rx(&frame));
        }
    }

    #[test]
    fn late_and_frequent_wakes_use_actual_elapsed_time() {
        let mut clock = super::ReactorClock::new(100, 10_000).unwrap();
        assert_eq!(clock.sample(100, 10_000), Some(0));
        assert_eq!(clock.sample(101, 10_000), Some(0));
        assert_eq!(clock.sample(25_100, 10_000), Some(2500));
        assert_eq!(clock.sample(50_100, 10_000), Some(5000));
        assert!(clock.sample(50_101, 10_000).is_some()); // same millisecond
        assert!(clock.sample(100, 10_000).is_none());
        assert!(clock.sample(50_100, 1).is_none());
        assert!(super::ReactorClock::new(0, 0).is_none());
        let mut huge = super::ReactorClock::new(0, 1).unwrap();
        assert_eq!(huge.sample(u64::MAX, 1), Some(i64::MAX as u64));
    }
}
