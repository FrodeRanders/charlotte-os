//! Exact full-name publication observation and retirement. No scalar fallback.
#![no_std]
extern crate alloc;

pub const VERSION: u32 = 2;
pub const MAGIC: &[u8; 8] = b"CNSGEN01";
pub const HEADER_LEN: usize = 24;
pub const MAX_NAME_LEN: usize = 256;
pub const MAX_FRAME_LEN: usize = HEADER_LEN + MAX_NAME_LEN;
pub const OP_UNREGISTER_GENERATION: u32 = 10;
pub const OP_PUBLICATION_GENERATION: u32 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Observe,
    Retire,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    pub name: &'a [u8],
    pub generation: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    Invalid,
    AllocationFailed,
}
fn valid(action: Action, request: Request<'_>) -> bool {
    !request.name.is_empty()
        && request.name.len() <= MAX_NAME_LEN
        && match action {
            Action::Observe => request.generation == 0,
            Action::Retire => (1..=i64::MAX as u64).contains(&request.generation),
        }
}
pub fn encode(action: Action, request: Request<'_>) -> Result<alloc::vec::Vec<u8>, EncodeError> {
    if !valid(action, request) {
        return Err(EncodeError::Invalid);
    }
    let len = HEADER_LEN + request.name.len();
    let mut bytes = alloc::vec::Vec::new();
    bytes.try_reserve_exact(len).map_err(|_| EncodeError::AllocationFailed)?;
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(len as u32).to_le_bytes());
    bytes.extend_from_slice(&(request.name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&request.generation.to_le_bytes());
    bytes.extend_from_slice(request.name);
    Ok(bytes)
}
pub fn decode(action: Action, bytes: &[u8]) -> Option<Request<'_>> {
    if !(HEADER_LEN..=MAX_FRAME_LEN).contains(&bytes.len()) || bytes.get(..8)? != MAGIC {
        return None;
    }
    let total = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let name_len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    if total != bytes.len() || HEADER_LEN.checked_add(name_len)? != total {
        return None;
    }
    let request = Request {
        name: &bytes[HEADER_LEN..],
        generation: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
    };
    valid(action, request).then_some(request)
}
#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    #[test]
    fn golden_and_full_identity_round_trip() {
        let request = Request {
            name: b"echo\0",
            generation: 7,
        };
        let bytes = encode(Action::Retire, request).unwrap();
        assert_eq!(
            bytes,
            [
                MAGIC.as_slice(),
                &29u32.to_le_bytes(),
                &5u32.to_le_bytes(),
                &7u64.to_le_bytes(),
                b"echo\0"
            ]
            .concat()
        );
        assert_eq!(decode(Action::Retire, &bytes), Some(request));
        for name in [vec![b'a'; 255], vec![b'b'; 256]] {
            let request = Request {
                name: &name,
                generation: i64::MAX as u64,
            };
            assert_eq!(
                decode(Action::Retire, &encode(Action::Retire, request).unwrap()),
                Some(request)
            );
        }
        let query = Request {
            name: b"system.console.echo.primary.v1",
            generation: 0,
        };
        assert_eq!(decode(Action::Observe, &encode(Action::Observe, query).unwrap()), Some(query));
    }
    #[test]
    fn action_and_generation_are_explicit() {
        for (action, generation) in
            [(Action::Retire, 0), (Action::Retire, u64::MAX), (Action::Observe, 1)]
        {
            assert!(
                encode(
                    action,
                    Request {
                        name: b"echo",
                        generation
                    }
                )
                .is_err()
            );
        }
        let bytes = encode(
            Action::Observe,
            Request {
                name: b"echo",
                generation: 0,
            },
        )
        .unwrap();
        assert!(decode(Action::Retire, &bytes).is_none());
        let bytes = encode(
            Action::Retire,
            Request {
                name: b"echo",
                generation: 1,
            },
        )
        .unwrap();
        assert!(decode(Action::Observe, &bytes).is_none());
        for name in [vec![], vec![0; 257]] {
            assert!(
                encode(
                    Action::Observe,
                    Request {
                        name: &name,
                        generation: 0
                    }
                )
                .is_err()
            );
        }
    }
    #[test]
    fn malformed_and_retired_records_reject() {
        let frame = encode(
            Action::Retire,
            Request {
                name: b"echo",
                generation: 1,
            },
        )
        .unwrap();
        for end in 0..frame.len() {
            assert!(decode(Action::Retire, &frame[..end]).is_none());
        }
        let mut trailer = frame.clone();
        trailer.push(0);
        assert!(decode(Action::Retire, &trailer).is_none());
        for offset in [8, 12] {
            for value in [0, u32::MAX] {
                let mut wrong = frame.clone();
                wrong[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                assert!(decode(Action::Retire, &wrong).is_none());
            }
        }
        let mut wrong = frame;
        wrong[0] ^= 1;
        assert!(decode(Action::Retire, &wrong).is_none());
        assert!(decode(Action::Retire, &1u64.to_le_bytes()).is_none());
        assert!(decode(Action::Observe, b"system.console.echo.primary.v1").is_none());
    }
}
