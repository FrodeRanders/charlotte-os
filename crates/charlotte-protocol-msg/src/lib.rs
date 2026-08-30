//! `charlotte-protocol-msg` — the reliable message-layer wire protocol v3.
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
//! - A 32-bit fragment offset and total message length
//! - A 16-bit per-frame payload length
//! - A 16-bit flags field (bit 0 = SYN, bit 1 = ACK, bit 2 = FIN, bit 3 = FRAG, bit 4 = MORE)
//! - The application payload
//!
//! ## Fragmentation
//!
//! Messages larger than one frame's payload are split across multiple
//! frames that share the message's sequence number. A fragmented frame sets
//! `FLAG_FRAG` and carries its 32-bit byte offset. Every fragment except the
//! last also sets `FLAG_MORE`. Every data frame carries the total message
//! length, allowing receivers to validate bounds before allocating. The
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
//!  2..4   Protocol version = 3 (u16, big-endian)
//!  4..12  Session (u64, big-endian; sender on data, acknowledged on ACK)
//! 12..16  Sequence number (u32, big-endian)
//! 16..20  Ack number (u32, big-endian)
//! 20..24  Fragment offset (u32, big-endian; zero for unfragmented data)
//! 24..28  Total message length (u32, big-endian; zero for control frames)
//! 28..30  Frame payload length (u16, big-endian)
//! 30..32  Flags (u16, big-endian)
//! 32..   Payload
//! ```
//!
//! The header is 32 bytes. The maximum payload per frame is (MTU - 14 - 32)
//! bytes, i.e. 1454 bytes on standard Ethernet; messages larger than that
//! are fragmented.

#![no_std]

#[cfg(test)]
extern crate std;

/// CharlotteOS reliable-message EtherType (IANA unassigned, chosen for
/// internal use until registration).
pub const MSG_ETHERTYPE: u16 = 0x88b5;
pub const WIRE_VERSION: u16 = 3;

/// Header size in bytes.
pub const HEADER_SIZE: usize = 32;
pub const MAX_PAYLOAD_SIZE: usize = 1500 - ETHERNET_HEADER_SIZE - HEADER_SIZE;
pub const ETHERNET_HEADER_SIZE: usize = 14;
pub const FRAME_HEADER_SIZE: usize = ETHERNET_HEADER_SIZE + HEADER_SIZE;
pub const BROADCAST_MAC: [u8; 6] = [0xff; 6];

/// Flags
pub const FLAG_SYN: u16 = 1 << 0;
pub const FLAG_ACK: u16 = 1 << 1;
pub const FLAG_FIN: u16 = 1 << 2;
/// This frame is a fragment of a multi-frame message; the header carries its
/// 32-bit byte offset within the message payload.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    pub session: u64,
    pub sequence: u32,
    pub acknowledgment: u32,
    pub fragment_offset: u32,
    pub total_message_len: u32,
    pub payload_len: u16,
    pub flags: u16,
}

/// Fields written ahead of a reliable-message Ethernet payload.
///
/// Keeping these related values together makes construction self-documenting
/// and avoids an error-prone sequence of similarly typed positional arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub destination: [u8; 6],
    pub source: [u8; 6],
    pub message: MessageHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    FrameTooShort,
    WrongEtherType,
    WrongVersion,
    ReservedBits,
    InvalidFlags,
    PayloadTooLarge,
    InvalidControlPayload,
}

/// Pack a MAC address in the low 48 bits of a scalar IPC result.
pub fn pack_mac(mac: [u8; 6]) -> u64 {
    let mut packed = 0u64;
    for (index, byte) in mac.iter().enumerate() {
        packed |= (*byte as u64) << (index * 8);
    }
    packed
}

pub fn unpack_mac(packed: u64) -> [u8; 6] {
    let mut mac = [0u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = (packed >> (index * 8)) as u8;
    }
    mac
}

/// Typed application IPC envelope carried at the start of moved memory for
/// `relmsg::OP_SEND` and `relmsg::OP_RECV`.
///
/// Layout: `"RMI3"`, payload length (u32 LE), peer MAC (six bytes), two
/// reserved zero bytes, then the payload.
pub const IPC_MESSAGE_MAGIC: [u8; 4] = *b"RMI3";
pub const IPC_MESSAGE_HEADER_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcMessageHeader {
    pub peer: [u8; 6],
    pub payload_len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcHeaderError {
    TooShort,
    WrongMagic,
    ReservedBits,
    LengthOverflow,
}

pub fn build_ipc_message_header(
    buf: &mut [u8; IPC_MESSAGE_HEADER_SIZE],
    peer: [u8; 6],
    payload_len: u32,
) {
    buf[..4].copy_from_slice(&IPC_MESSAGE_MAGIC);
    buf[4..8].copy_from_slice(&payload_len.to_le_bytes());
    buf[8..14].copy_from_slice(&peer);
    buf[14..16].fill(0);
}

pub fn parse_ipc_message_header(buf: &[u8]) -> Result<IpcMessageHeader, IpcHeaderError> {
    if buf.len() < IPC_MESSAGE_HEADER_SIZE {
        return Err(IpcHeaderError::TooShort);
    }
    if buf[..4] != IPC_MESSAGE_MAGIC {
        return Err(IpcHeaderError::WrongMagic);
    }
    if buf[14..16] != [0, 0] {
        return Err(IpcHeaderError::ReservedBits);
    }
    let payload_len =
        u32::from_le_bytes(buf[4..8].try_into().map_err(|_| IpcHeaderError::TooShort)?);
    let required = IPC_MESSAGE_HEADER_SIZE
        .checked_add(payload_len as usize)
        .ok_or(IpcHeaderError::LengthOverflow)?;
    if required > buf.len() {
        return Err(IpcHeaderError::LengthOverflow);
    }
    let peer = buf[8..14].try_into().map_err(|_| IpcHeaderError::TooShort)?;
    Ok(IpcMessageHeader {
        peer,
        payload_len,
    })
}

pub fn build_frame_header(buf: &mut [u8; FRAME_HEADER_SIZE], header: FrameHeader) {
    buf[..6].copy_from_slice(&header.destination);
    buf[6..12].copy_from_slice(&header.source);
    buf[12..14].copy_from_slice(&MSG_ETHERTYPE.to_be_bytes());
    let mut message = [0u8; HEADER_SIZE];
    build_header(&mut message, header.message);
    buf[ETHERNET_HEADER_SIZE..].copy_from_slice(&message);
}

pub fn parse_frame_header_checked(frame: &[u8]) -> Result<FrameHeader, HeaderError> {
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
    let message = parse_header_checked(&header)?;
    if FRAME_HEADER_SIZE + message.payload_len as usize > frame.len() {
        return Err(HeaderError::FrameTooShort);
    }
    Ok(FrameHeader {
        destination,
        source,
        message,
    })
}

/// Build a v3 message header.
pub fn build_header(buf: &mut [u8; HEADER_SIZE], header: MessageHeader) {
    buf[0] = (MSG_ETHERTYPE >> 8) as u8;
    buf[1] = MSG_ETHERTYPE as u8;
    buf[2..4].copy_from_slice(&WIRE_VERSION.to_be_bytes());
    buf[4..12].copy_from_slice(&header.session.to_be_bytes());
    buf[12..16].copy_from_slice(&header.sequence.to_be_bytes());
    buf[16..20].copy_from_slice(&header.acknowledgment.to_be_bytes());
    buf[20..24].copy_from_slice(&header.fragment_offset.to_be_bytes());
    buf[24..28].copy_from_slice(&header.total_message_len.to_be_bytes());
    buf[28..30].copy_from_slice(&header.payload_len.to_be_bytes());
    buf[30..32].copy_from_slice(&header.flags.to_be_bytes());
}

/// Set the fragment byte offset in an already-built message header.
pub fn set_fragment_offset(buf: &mut [u8; HEADER_SIZE], offset: u32) {
    buf[20..24].copy_from_slice(&offset.to_be_bytes());
}

/// Replace the message sequence number in an already-built header.
pub fn set_sequence(buf: &mut [u8; HEADER_SIZE], sequence: u32) {
    buf[12..16].copy_from_slice(&sequence.to_be_bytes());
}

/// Read the fragment byte offset from a message header.
pub fn fragment_offset(buf: &[u8; HEADER_SIZE]) -> u32 {
    u32::from_be_bytes(buf[20..24].try_into().expect("fragment offset field"))
}

/// Parse a received message header without validating its fields.
pub fn parse_header(buf: &[u8; HEADER_SIZE]) -> MessageHeader {
    let session = u64::from_be_bytes(buf[4..12].try_into().expect("session field"));
    let seq = u32::from_be_bytes(buf[12..16].try_into().expect("sequence field"));
    let ack = u32::from_be_bytes(buf[16..20].try_into().expect("ack field"));
    let offset = fragment_offset(buf);
    let total_len = u32::from_be_bytes(buf[24..28].try_into().expect("total length field"));
    let len = u16::from_be_bytes(buf[28..30].try_into().expect("length field"));
    let flags = u16::from_be_bytes(buf[30..32].try_into().expect("flags field"));
    MessageHeader {
        session,
        sequence: seq,
        acknowledgment: ack,
        fragment_offset: offset,
        total_message_len: total_len,
        payload_len: len,
        flags,
    }
}

pub fn parse_header_checked(buf: &[u8; HEADER_SIZE]) -> Result<MessageHeader, HeaderError> {
    if u16::from_be_bytes([buf[0], buf[1]]) != MSG_ETHERTYPE {
        return Err(HeaderError::WrongEtherType);
    }
    if u16::from_be_bytes([buf[2], buf[3]]) != WIRE_VERSION {
        return Err(HeaderError::WrongVersion);
    }
    let parsed = parse_header(buf);
    let offset = parsed.fragment_offset;
    let total_len = parsed.total_message_len;
    let payload_len = parsed.payload_len as usize;
    let flags = parsed.flags;
    if flags & !VALID_FLAGS != 0 || flags & FLAG_MORE != 0 && flags & FLAG_FRAG == 0 {
        return Err(HeaderError::InvalidFlags);
    }
    if offset != 0 && flags & FLAG_FRAG == 0 {
        return Err(HeaderError::ReservedBits);
    }
    if payload_len > MAX_PAYLOAD_SIZE || HEADER_SIZE + payload_len > 4096 {
        return Err(HeaderError::PayloadTooLarge);
    }
    if flags & (FLAG_ACK | FLAG_FIN) != 0 {
        if flags & (FLAG_FRAG | FLAG_MORE) != 0 {
            return Err(HeaderError::InvalidFlags);
        }
        if payload_len != 0 || offset != 0 || total_len != 0 {
            return Err(HeaderError::InvalidControlPayload);
        }
    }
    if flags & (FLAG_ACK | FLAG_FIN) == 0 {
        if payload_len == 0 || total_len == 0 {
            return Err(HeaderError::PayloadTooLarge);
        }
        let end = offset.checked_add(payload_len as u32).ok_or(HeaderError::PayloadTooLarge)?;
        if flags & FLAG_FRAG == 0 {
            if offset != 0 || total_len != payload_len as u32 {
                return Err(HeaderError::ReservedBits);
            }
        } else if total_len <= payload_len as u32
            || end > total_len
            || (flags & FLAG_MORE != 0) != (end < total_len)
        {
            return Err(HeaderError::ReservedBits);
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut hdr = [0u8; HEADER_SIZE];
        let expected = MessageHeader {
            session: 99,
            sequence: 42,
            acknowledgment: 17,
            fragment_offset: 0,
            total_message_len: 0,
            payload_len: 0,
            flags: FLAG_ACK,
        };
        build_header(&mut hdr, expected);
        assert_eq!(parse_header(&hdr), expected);
        assert_eq!(parse_header_checked(&hdr), Ok(expected));
    }

    #[test]
    fn fragment_offset_round_trip() {
        let mut hdr = [0u8; HEADER_SIZE];
        let offset_above_v2 = 70_000;
        let total_above_v2 = 80_000;
        let expected = MessageHeader {
            session: 99,
            sequence: 7,
            acknowledgment: 3,
            fragment_offset: offset_above_v2,
            total_message_len: total_above_v2,
            payload_len: 100,
            flags: FLAG_FRAG | FLAG_MORE,
        };
        build_header(&mut hdr, expected);
        assert_eq!(fragment_offset(&hdr), offset_above_v2);
        assert_eq!(parse_header(&hdr), expected);
        assert_eq!(parse_header_checked(&hdr), Ok(expected));
    }

    #[test]
    fn checked_parser_rejects_malformed_headers() {
        let mut hdr = [0u8; HEADER_SIZE];
        let data = MessageHeader {
            session: 99,
            sequence: 1,
            acknowledgment: 0,
            fragment_offset: 0,
            total_message_len: 1,
            payload_len: 1,
            flags: 0,
        };
        build_header(&mut hdr, data);

        hdr[0] = 0;
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::WrongEtherType));
        build_header(&mut hdr, data);
        hdr[3] = 2;
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::WrongVersion));
        build_header(&mut hdr, data);
        // A nonzero offset requires FLAG_FRAG.
        set_fragment_offset(&mut hdr, 5);
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::ReservedBits));
        build_header(
            &mut hdr,
            MessageHeader {
                flags: 1 << 15,
                ..data
            },
        );
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::InvalidFlags));
        build_header(
            &mut hdr,
            MessageHeader {
                total_message_len: (MAX_PAYLOAD_SIZE + 1) as u32,
                payload_len: (MAX_PAYLOAD_SIZE + 1) as u16,
                ..data
            },
        );
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::PayloadTooLarge));
        build_header(
            &mut hdr,
            MessageHeader {
                flags: FLAG_FIN,
                ..data
            },
        );
        assert_eq!(parse_header_checked(&hdr), Err(HeaderError::InvalidControlPayload));
    }

    #[test]
    fn ethernet_frame_header_round_trip() {
        let destination = [0x52, 0x54, 0, 0x12, 0x34, 2];
        let source = [0x52, 0x54, 0, 0x12, 0x34, 1];
        let mut frame = [0u8; FRAME_HEADER_SIZE];
        let mut header = [0u8; FRAME_HEADER_SIZE];
        build_frame_header(
            &mut header,
            FrameHeader {
                destination,
                source,
                message: MessageHeader {
                    session: 99,
                    sequence: 7,
                    acknowledgment: 6,
                    fragment_offset: 0,
                    total_message_len: 0,
                    payload_len: 0,
                    flags: FLAG_ACK,
                },
            },
        );
        frame[..FRAME_HEADER_SIZE].copy_from_slice(&header);
        assert_eq!(
            parse_frame_header_checked(&frame),
            Ok(FrameHeader {
                destination,
                source,
                message: MessageHeader {
                    session: 99,
                    sequence: 7,
                    acknowledgment: 6,
                    fragment_offset: 0,
                    total_message_len: 0,
                    payload_len: 0,
                    flags: FLAG_ACK,
                }
            })
        );
    }

    #[test]
    fn ethernet_data_frame_header_round_trip() {
        let destination = [0x52, 0x54, 0, 0x12, 0x34, 2];
        let source = [0x52, 0x54, 0, 0x12, 0x34, 1];
        let mut frame = [0u8; FRAME_HEADER_SIZE + 3];
        let mut header = [0u8; FRAME_HEADER_SIZE];
        build_frame_header(
            &mut header,
            FrameHeader {
                destination,
                source,
                message: MessageHeader {
                    session: 99,
                    sequence: 7,
                    acknowledgment: 0,
                    fragment_offset: 0,
                    total_message_len: 3,
                    payload_len: 3,
                    flags: FLAG_SYN,
                },
            },
        );
        frame[..FRAME_HEADER_SIZE].copy_from_slice(&header);
        frame[FRAME_HEADER_SIZE..].copy_from_slice(b"hey");
        assert_eq!(
            parse_frame_header_checked(&frame),
            Ok(FrameHeader {
                destination,
                source,
                message: MessageHeader {
                    session: 99,
                    sequence: 7,
                    acknowledgment: 0,
                    fragment_offset: 0,
                    total_message_len: 3,
                    payload_len: 3,
                    flags: FLAG_SYN,
                }
            })
        );
        assert_eq!(unpack_mac(pack_mac(destination)), destination);
    }

    #[test]
    fn ipc_message_header_supports_lengths_above_v2() {
        let peer = [0x52, 0x54, 0, 0x12, 0x34, 2];
        let payload_len = 1_048_576u32;
        let mut bytes = [0u8; IPC_MESSAGE_HEADER_SIZE + 1];
        build_ipc_message_header(
            (&mut bytes[..IPC_MESSAGE_HEADER_SIZE]).try_into().expect("header slice"),
            peer,
            payload_len,
        );
        assert_eq!(parse_ipc_message_header(&bytes), Err(IpcHeaderError::LengthOverflow));

        let mut header = [0u8; IPC_MESSAGE_HEADER_SIZE];
        build_ipc_message_header(&mut header, peer, 70_000);
        let mut message = std::vec![0u8; IPC_MESSAGE_HEADER_SIZE + 70_000];
        message[..IPC_MESSAGE_HEADER_SIZE].copy_from_slice(&header);
        assert_eq!(
            parse_ipc_message_header(&message),
            Ok(IpcMessageHeader {
                peer,
                payload_len: 70_000
            })
        );
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
