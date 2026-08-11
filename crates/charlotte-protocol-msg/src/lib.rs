//! `charlotte-protocol-msg` — the reliable message-layer wire protocol v2.
//!
//! This is the layer between raw Ethernet frames (§6 of the networking
//! architecture doc) and the RPC/Distributed Objects layer. It provides
//! sequenced, acknowledged message delivery with retransmission and flow
//! control — the "Reliable Message Layer" of the architecture.
//!
//! Messages carry:
//! - A 64-bit ordered session identifier, packed as a 32-bit service generation plus a 32-bit retry
//!   epoch (the sender session on data frames; the acknowledged sender session on ACK-only frames)
//! - A 32-bit sequence number (monotonic, per-session; one per *message*, shared by all fragments)
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
//! `FLAG_SYN` asserts the session field. On data frames, receivers use a
//! strictly newer sender session to reset receive ordering after a sender
//! restart or uncertain retry epoch. ACK-only
//! frames echo the data frame's session, allowing a sender to reject delayed
//! ACKs from an abandoned session. ACK and payload are deliberately not
//! combined because the one session field cannot identify both directions.
//!
//! ## Wire format (Ethertype 0x88B5, allocated to CharlotteOS)
//!
//! ```text
//!  0..2   EtherType = 0x88B5
//!  2..4   Fragment offset (u16, big-endian; 0 unless FLAG_FRAG is set)
//!  4..12  Session (u64, big-endian; sender on data, acknowledged on ACK)
//! 12..16  Sequence number (u32, big-endian)
//! 16..20  Ack number (u32, big-endian)
//! 20..22  Payload length (u16, big-endian)
//! 22..24  Flags (u16, big-endian)
//! 24..   Payload
//! ```
//!
//! The header is 24 bytes. The maximum payload per frame is (MTU - 14 - 24)
//! bytes, i.e. ~1468 bytes on standard Ethernet; messages larger than that
//! are fragmented.

#![no_std]

/// CharlotteOS reliable-message EtherType (IANA unassigned, chosen for
/// internal use until registration).
pub const MSG_ETHERTYPE: u16 = 0x88b5;

/// Header size in bytes.
pub const HEADER_SIZE: usize = 24;
pub const MAX_PAYLOAD_SIZE: usize = 1460;
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

/// Number of low session-id bits reserved for retry epochs within one
/// name-service generation. The high half identifies the service instance;
/// the low half starts at one and advances whenever an uncertain send is
/// abandoned. This keeps restart and retry session namespaces disjoint.
pub const SESSION_ATTEMPT_BITS: u32 = 32;
const SESSION_ATTEMPT_MASK: u64 = (1u64 << SESSION_ATTEMPT_BITS) - 1;

/// Construct the first wire session for a monotonically allocated service
/// generation. Returns `None` rather than wrapping if either namespace is
/// exhausted.
pub fn initial_wire_session(generation: u64) -> Option<u64> {
    let generation = u32::try_from(generation).ok()?;
    (generation != 0).then_some((u64::from(generation) << SESSION_ATTEMPT_BITS) | 1)
}

/// Advance to a fresh retry epoch within the same service generation.
pub fn next_wire_session(session: u64) -> Option<u64> {
    let generation = session >> SESSION_ATTEMPT_BITS;
    let attempt = session & SESSION_ATTEMPT_MASK;
    if generation == 0 || attempt == 0 || attempt == SESSION_ATTEMPT_MASK {
        return None;
    }
    Some(session + 1)
}

/// Whether `candidate` is a well-formed session strictly newer than the
/// receiver's current session. Session ordering follows the packed
/// `(service generation, retry epoch)` identity, so arbitrarily delayed
/// frames can never roll receive state backwards.
pub fn wire_session_is_newer(candidate: u64, current: u64) -> bool {
    let generation = candidate >> SESSION_ATTEMPT_BITS;
    let attempt = candidate & SESSION_ATTEMPT_MASK;
    generation != 0 && attempt != 0 && candidate > current
}

/// Parsed frame header: (destination MAC, source MAC, session, seq, ack,
/// payload_len, flags, fragment_offset).
pub type ParsedFrameHeader = ([u8; 6], [u8; 6], u64, u32, u32, u16, u16, u16);

/// Fields written ahead of a reliable-message Ethernet payload.
///
/// Keeping these related values together makes construction self-documenting
/// and avoids an error-prone sequence of similarly typed positional arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub destination: [u8; 6],
    pub source: [u8; 6],
    pub session: u64,
    pub sequence: u32,
    pub acknowledgment: u32,
    pub payload_len: u16,
    pub flags: u16,
}

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

pub fn build_frame_header(buf: &mut [u8; FRAME_HEADER_SIZE], header: FrameHeader) {
    buf[..6].copy_from_slice(&header.destination);
    buf[6..12].copy_from_slice(&header.source);
    buf[12..14].copy_from_slice(&MSG_ETHERTYPE.to_be_bytes());
    let mut message = [0u8; HEADER_SIZE];
    build_header(
        &mut message,
        header.session,
        header.sequence,
        header.acknowledgment,
        header.payload_len,
        header.flags,
    );
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
    let (session, seq, ack, payload_len, flags, offset) = parse_header_checked(&header)?;
    if FRAME_HEADER_SIZE + payload_len as usize > frame.len() {
        return Err(HeaderError::FrameTooShort);
    }
    Ok((destination, source, session, seq, ack, payload_len, flags, offset))
}

/// Build a message header into a 16-byte buffer. Bytes 2..4 are left zero
/// (fragment offset); call [`set_fragment_offset`] for fragmented frames.
pub fn build_header(
    buf: &mut [u8; HEADER_SIZE],
    session: u64,
    seq: u32,
    ack: u32,
    payload_len: u16,
    flags: u16,
) {
    buf[0] = (MSG_ETHERTYPE >> 8) as u8;
    buf[1] = MSG_ETHERTYPE as u8;
    buf[2] = 0;
    buf[3] = 0; // fragment offset
    buf[4..12].copy_from_slice(&session.to_be_bytes());
    buf[12..16].copy_from_slice(&seq.to_be_bytes());
    buf[16..20].copy_from_slice(&ack.to_be_bytes());
    buf[20..22].copy_from_slice(&payload_len.to_be_bytes());
    buf[22..24].copy_from_slice(&flags.to_be_bytes());
}

/// Set the fragment byte offset in a message header (bytes 2..4).
pub fn set_fragment_offset(buf: &mut [u8; HEADER_SIZE], offset: u16) {
    buf[2] = (offset >> 8) as u8;
    buf[3] = offset as u8;
}

/// Replace the message sequence number in an already-built header.
pub fn set_sequence(buf: &mut [u8; HEADER_SIZE], sequence: u32) {
    buf[12..16].copy_from_slice(&sequence.to_be_bytes());
}

/// Read the fragment byte offset from a message header (bytes 2..4).
pub fn fragment_offset(buf: &[u8; HEADER_SIZE]) -> u16 {
    u16::from_be_bytes([buf[2], buf[3]])
}

/// Parse a received message header. Returns
/// `(seq, ack, payload_len, flags, fragment_offset)`.
pub fn parse_header(buf: &[u8; HEADER_SIZE]) -> (u64, u32, u32, u16, u16, u16) {
    let session = u64::from_be_bytes(buf[4..12].try_into().expect("session field"));
    let seq = u32::from_be_bytes(buf[12..16].try_into().expect("sequence field"));
    let ack = u32::from_be_bytes(buf[16..20].try_into().expect("ack field"));
    let len = u16::from_be_bytes(buf[20..22].try_into().expect("length field"));
    let flags = u16::from_be_bytes(buf[22..24].try_into().expect("flags field"));
    (session, seq, ack, len, flags, fragment_offset(buf))
}

pub fn parse_header_checked(
    buf: &[u8; HEADER_SIZE],
) -> Result<(u64, u32, u32, u16, u16, u16), HeaderError> {
    if u16::from_be_bytes([buf[0], buf[1]]) != MSG_ETHERTYPE {
        return Err(HeaderError::WrongEtherType);
    }
    let parsed = parse_header(buf);
    let payload_len = parsed.3 as usize;
    let flags = parsed.4;
    let offset = parsed.5;
    if offset != 0 && flags & FLAG_FRAG == 0 {
        return Err(HeaderError::ReservedBits);
    }
    if flags & !VALID_FLAGS != 0 {
        return Err(HeaderError::InvalidFlags);
    }
    if payload_len > MAX_PAYLOAD_SIZE || HEADER_SIZE + payload_len > 4096 {
        return Err(HeaderError::PayloadTooLarge);
    }
    if flags & FLAG_FIN != 0 && payload_len != 0 {
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
        build_header(&mut hdr, 99, 42, 17, 100, FLAG_ACK);
        let (session, seq, ack, len, flags, offset) = parse_header(&hdr);
        assert_eq!(session, 99);
        assert_eq!(seq, 42);
        assert_eq!(ack, 17);
        assert_eq!(len, 100);
        assert_eq!(flags, FLAG_ACK);
        assert_eq!(offset, 0);
        assert_eq!(parse_header_checked(&hdr), Ok((99, 42, 17, 100, FLAG_ACK, 0)));
    }

    #[test]
    fn fragment_offset_round_trip() {
        let mut hdr = [0u8; HEADER_SIZE];
        build_header(&mut hdr, 99, 7, 3, 100, FLAG_FRAG | FLAG_MORE);
        set_fragment_offset(&mut hdr, 1468);
        assert_eq!(fragment_offset(&hdr), 1468);
        let (_session, seq, _ack, len, flags, offset) = parse_header(&hdr);
        assert_eq!(seq, 7);
        assert_eq!(len, 100);
        assert_eq!(flags, FLAG_FRAG | FLAG_MORE);
        assert_eq!(offset, 1468);
        assert_eq!(parse_header_checked(&hdr), Ok((99, 7, 3, 100, FLAG_FRAG | FLAG_MORE, 1468)));
    }

    #[test]
    fn checked_parser_rejects_malformed_headers() {
        let mut hdr = [0u8; HEADER_SIZE];
        build_header(&mut hdr, 99, 1, 0, 0, 0);

        hdr[0] = 0;
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::WrongEtherType));
        build_header(&mut hdr, 99, 1, 0, 0, 0);
        hdr[2] = 1;
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::ReservedBits));
        build_header(&mut hdr, 99, 1, 0, 0, 0);
        // A nonzero offset requires FLAG_FRAG.
        set_fragment_offset(&mut hdr, 5);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::ReservedBits));
        build_header(&mut hdr, 99, 1, 0, 0, 1 << 15);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::InvalidFlags));
        build_header(&mut hdr, 99, 1, 0, (MAX_PAYLOAD_SIZE + 1) as u16, 0);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::PayloadTooLarge));
        build_header(&mut hdr, 99, 1, 0, 1, FLAG_FIN);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::InvalidControlPayload));
    }

    #[test]
    fn ethernet_frame_header_round_trip() {
        let destination = [0x52, 0x54, 0, 0x12, 0x34, 2];
        let source = [0x52, 0x54, 0, 0x12, 0x34, 1];
        let mut frame = [0u8; FRAME_HEADER_SIZE + 3];
        let mut header = [0u8; FRAME_HEADER_SIZE];
        build_frame_header(
            &mut header,
            FrameHeader {
                destination,
                source,
                session: 99,
                sequence: 7,
                acknowledgment: 6,
                payload_len: 3,
                flags: FLAG_ACK,
            },
        );
        frame[..FRAME_HEADER_SIZE].copy_from_slice(&header);
        frame[FRAME_HEADER_SIZE..].copy_from_slice(b"hey");
        assert_eq!(
            parse_frame_header_checked(&frame),
            Ok((destination, source, 99, 7, 6, 3, FLAG_ACK, 0))
        );
        let packed = pack_address_and_len(destination, 3);
        assert_eq!(unpack_address_and_len(packed), (destination, 3));
    }

    #[test]
    fn retry_and_restart_sessions_are_disjoint_and_ordered() {
        let generation_one = initial_wire_session(1).expect("generation one");
        let retry = next_wire_session(generation_one).expect("retry epoch");
        let generation_two = initial_wire_session(2).expect("generation two");

        assert_ne!(retry, generation_two);
        assert!(wire_session_is_newer(retry, generation_one));
        assert!(wire_session_is_newer(generation_two, retry));
        assert!(!wire_session_is_newer(generation_one, generation_two));
    }

    #[test]
    fn session_namespace_exhaustion_fails_closed() {
        assert_eq!(initial_wire_session(0), None);
        assert_eq!(initial_wire_session(u64::from(u32::MAX) + 1), None);
        let exhausted = (1u64 << SESSION_ATTEMPT_BITS) | SESSION_ATTEMPT_MASK;
        assert_eq!(next_wire_session(exhausted), None);
        assert!(!wire_session_is_newer(0, exhausted));
    }
}
