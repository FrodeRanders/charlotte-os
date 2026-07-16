//! `charlotte-protocol-msg` — the reliable message-layer protocol v1.
//!
//! This is the layer between raw Ethernet frames (§6 of the networking
//! architecture doc) and the RPC/Distributed Objects layer. It provides
//! sequenced, acknowledged message delivery with retransmission and flow
//! control — the "Reliable Message Layer" of the architecture.
//!
//! Messages carry:
//! - A 32-bit sequence number (monotonic, per-connection)
//! - A 32-bit acknowledgement number (cumulative)
//! - A 16-bit payload length
//! - A 16-bit flags field (bit 0 = SYN, bit 1 = ACK, bit 2 = FIN)
//! - The application payload
//!
//! ## Wire format (Ethertype 0x88B5, allocated to CharlotteOS)
//!
//! ```text
//!  0..2   EtherType = 0x88B5
//!  2..4   Reserved (0)
//!  4..8   Sequence number (u32, big-endian)
//!  8..12  Ack number (u32, big-endian)
//! 12..14  Payload length (u16, big-endian)
//! 14..16  Flags (u16, big-endian)
//! 16..   Payload
//! ```
//!
//! The header is 16 bytes. The maximum payload is (MTU - 14 - 16) bytes,
//! i.e. ~1468 bytes on standard Ethernet.

#![no_std]

/// CharlotteOS reliable-message EtherType (IANA unassigned, chosen for
/// internal use until registration).
pub const MSG_ETHERTYPE: u16 = 0x88B5;

/// Header size in bytes.
pub const HEADER_SIZE: usize = 16;

/// Flags
pub const FLAG_SYN: u16 = 1 << 0;
pub const FLAG_ACK: u16 = 1 << 1;
pub const FLAG_FIN: u16 = 1 << 2;

/// Build a message header into a 16-byte buffer.
pub fn build_header(
    buf: &mut [u8; HEADER_SIZE],
    seq: u32,
    ack: u32,
    payload_len: u16,
    flags: u16,
) {
    buf[0] = (MSG_ETHERTYPE >> 8) as u8;
    buf[1] = MSG_ETHERTYPE as u8;
    buf[2] = 0; buf[3] = 0; // reserved
    buf[4] = (seq >> 24) as u8;
    buf[5] = (seq >> 16) as u8;
    buf[6] = (seq >> 8) as u8;
    buf[7] = seq as u8;
    buf[8] = (ack >> 24) as u8;
    buf[9] = (ack >> 16) as u8;
    buf[10] = (ack >> 8) as u8;
    buf[11] = ack as u8;
    buf[12] = (payload_len >> 8) as u8;
    buf[13] = payload_len as u8;
    buf[14] = (flags >> 8) as u8;
    buf[15] = flags as u8;
}

/// Parse a received message header. Returns `(seq, ack, payload_len, flags)`.
pub fn parse_header(buf: &[u8; HEADER_SIZE]) -> (u32, u32, u16, u16) {
    let seq = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ack = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let len = u16::from_be_bytes([buf[12], buf[13]]);
    let flags = u16::from_be_bytes([buf[14], buf[15]]);
    (seq, ack, len, flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut hdr = [0u8; HEADER_SIZE];
        build_header(&mut hdr, 42, 17, 100, FLAG_SYN | FLAG_ACK);
        let (seq, ack, len, flags) = parse_header(&hdr);
        assert_eq!(seq, 42);
        assert_eq!(ack, 17);
        assert_eq!(len, 100);
        assert_eq!(flags, FLAG_SYN | FLAG_ACK);
    }
}
