//! Memory-carried DNS invocation. Retired packed-name/12-byte calls reject.
#![no_std]
extern crate alloc;

pub const VERSION: u32 = 4;
pub const OP_CALL: u32 = 5;
pub const MAGIC: &[u8; 8] = b"CDNSCL01";
pub const HEADER_LEN: usize = 28;
pub const MAX_NAME_LEN: usize = 256;
pub const MAX_FRAME_LEN: usize = HEADER_LEN + MAX_NAME_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    pub name: &'a [u8],
    pub opcode: u32,
    pub arg: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    InvalidName,
    AllocationFailed,
}

pub fn encode(request: Request<'_>) -> Result<alloc::vec::Vec<u8>, EncodeError> {
    if request.name.is_empty() || request.name.len() > MAX_NAME_LEN {
        return Err(EncodeError::InvalidName);
    }
    let len = HEADER_LEN + request.name.len();
    let mut bytes = alloc::vec::Vec::new();
    bytes.try_reserve_exact(len).map_err(|_| EncodeError::AllocationFailed)?;
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(len as u32).to_le_bytes());
    bytes.extend_from_slice(&(request.name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&request.opcode.to_le_bytes());
    bytes.extend_from_slice(&request.arg.to_le_bytes());
    bytes.extend_from_slice(request.name);
    Ok(bytes)
}

/// Validate the whole frame before lending its complete name. `arg0` at the
/// IPC boundary must also equal this frame's exact length.
pub fn decode(bytes: &[u8]) -> Option<Request<'_>> {
    if bytes.len() < HEADER_LEN || bytes.len() > MAX_FRAME_LEN || bytes.get(..8)? != MAGIC {
        return None;
    }
    let total = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let name_len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    if name_len == 0
        || name_len > MAX_NAME_LEN
        || total != bytes.len()
        || HEADER_LEN.checked_add(name_len)? != total
    {
        return None;
    }
    Some(Request {
        name: &bytes[HEADER_LEN..],
        opcode: u32::from_le_bytes(bytes[16..20].try_into().ok()?),
        arg: i64::from_le_bytes(bytes[20..28].try_into().ok()?),
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn golden_frame_preserves_full_name_and_signed_argument() {
        let request = Request {
            name: b"echo",
            opcode: 1,
            arg: -42,
        };
        let bytes = encode(request).unwrap();
        assert_eq!(
            bytes,
            [
                MAGIC.as_slice(),
                &32u32.to_le_bytes(),
                &4u32.to_le_bytes(),
                &1u32.to_le_bytes(),
                &(-42i64).to_le_bytes(),
                b"echo"
            ]
            .concat()
        );
        assert_eq!(decode(&bytes), Some(request));
    }

    #[test]
    fn complete_identity_boundaries_have_no_packed_aliases() {
        for name in [b"echo\0".as_slice(), &vec![b'x'; 255], &vec![b'x'; 256]] {
            let request = Request {
                name,
                opcode: 7,
                arg: i64::MIN,
            };
            assert_eq!(decode(&encode(request).unwrap()), Some(request));
        }
        assert!(
            encode(Request {
                name: b"",
                opcode: 0,
                arg: 0
            })
            .is_err()
        );
        assert!(
            encode(Request {
                name: &vec![0; 257],
                opcode: 0,
                arg: 0
            })
            .is_err()
        );
    }

    #[test]
    fn malformed_and_retired_records_reject() {
        let frame = encode(Request {
            name: b"echo",
            opcode: 1,
            arg: 42,
        })
        .unwrap();
        for end in 0..frame.len() {
            assert!(decode(&frame[..end]).is_none());
        }
        let mut extra = frame.clone();
        extra.push(0);
        assert!(decode(&extra).is_none());
        for offset in [8, 12] {
            for invalid in [0, u32::MAX] {
                let mut wrong = frame.clone();
                wrong[offset..offset + 4].copy_from_slice(&invalid.to_le_bytes());
                assert!(decode(&wrong).is_none());
            }
        }
        let mut wrong = frame;
        wrong[0] ^= 1;
        assert!(decode(&wrong).is_none());
        let old = [&1u32.to_le_bytes()[..], &42i64.to_le_bytes()[..]].concat();
        assert!(decode(&old).is_none());
    }
}
