//! `charlotte-protocol-net` — the NIC driver protocol v1.
//!
//! This crate defines the interface between a client (application or
//! higher-level service) and a NIC driver. It is deliberately frame-level
//! only: the driver knows nothing about IP, TCP, or sockets (§6 of the
//! networking architecture doc).
//!
//! ## Operations
//!
//! | Opcode | Name      | Semantics |
//! |--------|-----------|-----------|
//! | 1      | OP_STATUS | Query driver status; reply carries MAC + link state |
//! | 2      | OP_SEND   | Transmit a raw Ethernet frame (call with moved memory) |
//! | 3      | OP_RECV   | Deferred receive (retained reply token, completed on RX) |
//! | 4      | OP_SHUTDOWN | Release device and exit |
//!
//! ## Usage
//!
//! A NIC driver creates an endpoint with `INTERFACE` and `VERSION` and
//! registers it under its service name (e.g. `net0`). A client looks up
//! that name through the userspace name service.
#![no_std]

/// Interface id: "NET " packed as a u64.
pub const INTERFACE: u64 = crate::name(b"NET ");
pub const VERSION: u32 = 1;
/// Default short service name for the first NIC.
pub const NAME: u64 = crate::name(b"net0");

pub const OP_STATUS: u32 = 1;
pub const OP_SEND: u32 = 2;
pub const OP_RECV: u32 = 3;
pub const OP_SHUTDOWN: u32 = 4;

/// Pack up to 8 ASCII bytes into a u64 service name (little-endian).
pub const fn name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

/// Decode a link status + MAC from the OP_STATUS reply scalar.
#[inline]
pub fn decode_status(reply: i64) -> (u8, [u8; 6]) {
    let v = reply as u64;
    let link = (v & 0xff) as u8;
    let mac = [
        ((v >> 48) & 0xff) as u8,
        ((v >> 40) & 0xff) as u8,
        ((v >> 32) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
    ];
    (link, mac)
}
