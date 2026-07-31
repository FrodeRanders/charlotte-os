//! `charlotte-protocol-msg` — the reliable message-layer protocol v1.
//!
//! This is the layer between raw Ethernet frames (§6 of the networking
//! architecture doc) and the RPC/Distributed Objects layer. It provides
//! sequenced, acknowledged message delivery with retransmission and flow
//! control — the "Reliable Message Layer" of the architecture.
//!
//! Messages carry:
//! - A 32-bit sequence number (monotonic, per-connection; one per *message*, shared by all
//!   fragments)
//! - A 32-bit acknowledgement number (cumulative)
//! - A 16-bit payload length
//! - A 16-bit flags field (bit 0 = SYN, bit 1 = ACK, bit 2 = FIN, bit 3 = FRAG, bit 4 = MORE)
//! - The application payload
//!
//! ## Fragmentation
//!
//! Messages larger than one frame's payload are split across multiple
//! frames that share the message's sequence number. A fragmented frame sets
//! `FLAG_FRAG` and carries its byte offset in header bytes 2..4 (otherwise
//! reserved). Every fragment except the last also sets `FLAG_MORE`. The
//! receiver reassembles contiguous fragments of the expected message before
//! delivering one application message.
//!
//! ## Wire format (Ethertype 0x88B5, allocated to CharlotteOS)
//!
//! ```text
//!  0..2   EtherType = 0x88B5
//!  2..4   Fragment offset (u16, big-endian; 0 unless FLAG_FRAG is set)
//!  4..8   Sequence number (u32, big-endian)
//!  8..12  Ack number (u32, big-endian)
//! 12..14  Payload length (u16, big-endian)
//! 14..16  Flags (u16, big-endian)
//! 16..   Payload
//! ```
//!
//! The header is 16 bytes. The maximum payload per frame is (MTU - 14 - 16)
//! bytes, i.e. ~1468 bytes on standard Ethernet; messages larger than that
//! are fragmented.

#![no_std]

/// CharlotteOS reliable-message EtherType (IANA unassigned, chosen for
/// internal use until registration).
pub const MSG_ETHERTYPE: u16 = 0x88b5;

/// Header size in bytes.
pub const HEADER_SIZE: usize = 16;
pub const MAX_PAYLOAD_SIZE: usize = 1468;
pub const ETHERNET_HEADER_SIZE: usize = 14;
pub const FRAME_HEADER_SIZE: usize = ETHERNET_HEADER_SIZE + HEADER_SIZE;
pub const BROADCAST_MAC: [u8; 6] = [0xff; 6];

/// Flags
pub const FLAG_SYN: u16 = 1 << 0;
pub const FLAG_ACK: u16 = 1 << 1;
pub const FLAG_FIN: u16 = 1 << 2;
/// This frame is a fragment of a multi-frame message; header bytes 2..4
/// carry the byte offset within the message payload.
pub const FLAG_FRAG: u16 = 1 << 3;
/// More fragments of this message follow (this is not the last one).
pub const FLAG_MORE: u16 = 1 << 4;
pub const VALID_FLAGS: u16 = FLAG_SYN | FLAG_ACK | FLAG_FIN | FLAG_FRAG | FLAG_MORE;

/// Parsed frame header: (destination MAC, source MAC, seq, ack, payload_len,
/// flags, fragment_offset).
pub type ParsedFrameHeader = ([u8; 6], [u8; 6], u32, u32, u16, u16, u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    FrameTooShort,
    WrongEtherType,
    ReservedBits,
    InvalidFlags,
    PayloadTooLarge,
    InvalidControlPayload,
}

pub fn pack_address_and_len(mac: [u8; 6], len: u16) -> u64 {
    let mut packed = len as u64;
    for (index, byte) in mac.iter().enumerate() {
        packed |= (*byte as u64) << (16 + index * 8);
    }
    packed
}

pub fn unpack_address_and_len(packed: u64) -> ([u8; 6], u16) {
    let mut mac = [0u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = (packed >> (16 + index * 8)) as u8;
    }
    (mac, packed as u16)
}

pub fn build_frame_header(
    buf: &mut [u8; FRAME_HEADER_SIZE],
    destination: [u8; 6],
    source: [u8; 6],
    seq: u32,
    ack: u32,
    payload_len: u16,
    flags: u16,
) {
    buf[..6].copy_from_slice(&destination);
    buf[6..12].copy_from_slice(&source);
    buf[12..14].copy_from_slice(&MSG_ETHERTYPE.to_be_bytes());
    let mut message = [0u8; HEADER_SIZE];
    build_header(&mut message, seq, ack, payload_len, flags);
    buf[ETHERNET_HEADER_SIZE..].copy_from_slice(&message);
}

pub fn parse_frame_header_checked(frame: &[u8]) -> Result<ParsedFrameHeader, HeaderError> {
    if frame.len() < FRAME_HEADER_SIZE {
        return Err(HeaderError::FrameTooShort);
    }
    if u16::from_be_bytes([frame[12], frame[13]]) != MSG_ETHERTYPE {
        return Err(HeaderError::WrongEtherType);
    }
    let destination = frame[..6].try_into().map_err(|_| HeaderError::FrameTooShort)?;
    let source = frame[6..12].try_into().map_err(|_| HeaderError::FrameTooShort)?;
    let header: [u8; HEADER_SIZE] = frame[ETHERNET_HEADER_SIZE..FRAME_HEADER_SIZE]
        .try_into()
        .map_err(|_| HeaderError::FrameTooShort)?;
    let (seq, ack, payload_len, flags, offset) = parse_header_checked(&header)?;
    if FRAME_HEADER_SIZE + payload_len as usize > frame.len() {
        return Err(HeaderError::FrameTooShort);
    }
    Ok((destination, source, seq, ack, payload_len, flags, offset))
}

/// Build a message header into a 16-byte buffer. Bytes 2..4 are left zero
/// (fragment offset); call [`set_fragment_offset`] for fragmented frames.
pub fn build_header(buf: &mut [u8; HEADER_SIZE], seq: u32, ack: u32, payload_len: u16, flags: u16) {
    buf[0] = (MSG_ETHERTYPE >> 8) as u8;
    buf[1] = MSG_ETHERTYPE as u8;
    buf[2] = 0;
    buf[3] = 0; // fragment offset
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

/// Set the fragment byte offset in a message header (bytes 2..4).
pub fn set_fragment_offset(buf: &mut [u8; HEADER_SIZE], offset: u16) {
    buf[2] = (offset >> 8) as u8;
    buf[3] = offset as u8;
}

/// Read the fragment byte offset from a message header (bytes 2..4).
pub fn fragment_offset(buf: &[u8; HEADER_SIZE]) -> u16 {
    u16::from_be_bytes([buf[2], buf[3]])
}

/// Parse a received message header. Returns
/// `(seq, ack, payload_len, flags, fragment_offset)`.
pub fn parse_header(buf: &[u8; HEADER_SIZE]) -> (u32, u32, u16, u16, u16) {
    let seq = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ack = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let len = u16::from_be_bytes([buf[12], buf[13]]);
    let flags = u16::from_be_bytes([buf[14], buf[15]]);
    (seq, ack, len, flags, fragment_offset(buf))
}

pub fn parse_header_checked(
    buf: &[u8; HEADER_SIZE],
) -> Result<(u32, u32, u16, u16, u16), HeaderError> {
    if u16::from_be_bytes([buf[0], buf[1]]) != MSG_ETHERTYPE {
        return Err(HeaderError::WrongEtherType);
    }
    let parsed = parse_header(buf);
    let payload_len = parsed.2 as usize;
    let flags = parsed.3;
    let offset = parsed.4;
    if offset != 0 && flags & FLAG_FRAG == 0 {
        return Err(HeaderError::ReservedBits);
    }
    if flags & !VALID_FLAGS != 0 {
        return Err(HeaderError::InvalidFlags);
    }
    if payload_len > MAX_PAYLOAD_SIZE || HEADER_SIZE + payload_len > 4096 {
        return Err(HeaderError::PayloadTooLarge);
    }
    if flags & (FLAG_SYN | FLAG_FIN) != 0 && payload_len != 0 {
        return Err(HeaderError::InvalidControlPayload);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut hdr = [0u8; HEADER_SIZE];
        build_header(&mut hdr, 42, 17, 100, FLAG_ACK);
        let (seq, ack, len, flags, offset) = parse_header(&hdr);
        assert_eq!(seq, 42);
        assert_eq!(ack, 17);
        assert_eq!(len, 100);
        assert_eq!(flags, FLAG_ACK);
        assert_eq!(offset, 0);
        assert_eq!(parse_header_checked(&hdr), Ok((42, 17, 100, FLAG_ACK, 0)));
    }

    #[test]
    fn fragment_offset_round_trip() {
        let mut hdr = [0u8; HEADER_SIZE];
        build_header(&mut hdr, 7, 3, 100, FLAG_FRAG | FLAG_MORE);
        set_fragment_offset(&mut hdr, 1468);
        assert_eq!(fragment_offset(&hdr), 1468);
        let (seq, _ack, len, flags, offset) = parse_header(&hdr);
        assert_eq!(seq, 7);
        assert_eq!(len, 100);
        assert_eq!(flags, FLAG_FRAG | FLAG_MORE);
        assert_eq!(offset, 1468);
        assert_eq!(parse_header_checked(&hdr), Ok((7, 3, 100, FLAG_FRAG | FLAG_MORE, 1468)));
    }

    #[test]
    fn checked_parser_rejects_malformed_headers() {
        let mut hdr = [0u8; HEADER_SIZE];
        build_header(&mut hdr, 1, 0, 0, 0);

        hdr[0] = 0;
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::WrongEtherType));
        build_header(&mut hdr, 1, 0, 0, 0);
        hdr[2] = 1;
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::ReservedBits));
        build_header(&mut hdr, 1, 0, 0, 0);
        // A nonzero offset requires FLAG_FRAG.
        set_fragment_offset(&mut hdr, 5);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::ReservedBits));
        build_header(&mut hdr, 1, 0, 0, 1 << 15);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::InvalidFlags));
        build_header(&mut hdr, 1, 0, (MAX_PAYLOAD_SIZE + 1) as u16, 0);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::PayloadTooLarge));
        build_header(&mut hdr, 1, 0, 1, FLAG_SYN);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::InvalidControlPayload));
    }

    #[test]
    fn ethernet_frame_header_round_trip() {
        let destination = [0x52, 0x54, 0, 0x12, 0x34, 2];
        let source = [0x52, 0x54, 0, 0x12, 0x34, 1];
        let mut frame = [0u8; FRAME_HEADER_SIZE + 3];
        let mut header = [0u8; FRAME_HEADER_SIZE];
        build_frame_header(&mut header, destination, source, 7, 6, 3, FLAG_ACK);
        frame[..FRAME_HEADER_SIZE].copy_from_slice(&header);
        frame[FRAME_HEADER_SIZE..].copy_from_slice(b"hey");
        assert_eq!(
            parse_frame_header_checked(&frame),
            Ok((destination, source, 7, 6, 3, FLAG_ACK, 0))
        );
        let packed = pack_address_and_len(destination, 3);
        assert_eq!(unpack_address_and_len(packed), (destination, 3));
    }
}
