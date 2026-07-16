//! `charlotte-protocol-net` — the NIC driver protocol v1.
//!
//! This crate defines the interface between a client (application or
//! higher-level service) and a NIC driver. It is deliberately frame-level
//! only: the driver knows nothing about IP, TCP, or sockets (S6 of the
//! networking-architecture doc).
#![no_std]

/// Interface id: "NET " packed as a u64 (little-endian ASCII).
pub const INTERFACE: u64 = 0x0000_2054_454e; // "NET "
pub const VERSION: u32 = 1;
/// Default short service name for the first NIC.
pub const NAME: u64 = 0x306_4656e; // "net0" little-endian

pub const OP_STATUS: u32 = 1;
pub const OP_SEND: u32 = 2;
pub const OP_RECV: u32 = 3;
pub const OP_SHUTDOWN: u32 = 4;

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
