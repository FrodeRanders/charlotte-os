//! Current registration and exact-generation retraction relays.
//! Transport prepends the tag. Retired u8-length tags are never interpreted.

use alloc::vec::Vec;

const MAX_OWNER_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    InvalidIdentity,
    TooLarge,
    AllocationFailed,
}

impl EncodeError {
    pub fn status(self) -> i64 {
        match self {
            Self::AllocationFailed => crate::dns::ERR_RESOURCE_LIMIT,
            Self::InvalidIdentity | Self::TooLarge => crate::dns::ERR_TOO_LARGE,
        }
    }
}

fn validate(owner: &[u8], name: &[u8]) -> Result<(), EncodeError> {
    if owner.is_empty() || name.is_empty() {
        return Err(EncodeError::InvalidIdentity);
    }
    if owner.len() > MAX_OWNER_BYTES || name.len() > crate::MAX_NAME_LEN {
        return Err(EncodeError::TooLarge);
    }
    Ok(())
}

fn buffer(length: usize) -> Result<Vec<u8>, EncodeError> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| EncodeError::AllocationFailed)?;
    Ok(bytes)
}

fn identities(bytes: &mut Vec<u8>, owner: &[u8], name: &[u8]) {
    bytes.extend_from_slice(&(owner.len() as u32).to_le_bytes());
    bytes.extend_from_slice(owner);
    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(name);
}

fn decode_identities(frame: &[u8]) -> Option<(&[u8], &[u8], u64)> {
    let owner_len = u32::from_le_bytes(frame.get(..4)?.try_into().ok()?) as usize;
    let owner_end = 4usize.checked_add(owner_len)?;
    let owner = frame.get(4..owner_end)?;
    let name_start = owner_end.checked_add(4)?;
    let name_len = u32::from_le_bytes(frame.get(owner_end..name_start)?.try_into().ok()?) as usize;
    let end = name_start.checked_add(name_len)?;
    let name = frame.get(name_start..end)?;
    if frame.len() != end.checked_add(8)? {
        return None;
    }
    validate(owner, name).ok()?;
    Some((owner, name, u64::from_le_bytes(frame.get(end..)?.try_into().ok()?)))
}

/// Correlated registration: 0x28 | session:u64 | request:u64 |
/// owner_len:u32 | owner | name_len:u32 | name | deployment_generation:u64.
/// Reply: 0x29 | session:u64 | request:u64 | generation:u64 (zero = failure).
pub mod rregister {
    pub use super::EncodeError;
    use super::*;

    pub const TAG_REQUEST: u8 = 0x28;
    pub const TAG_REPLY: u8 = 0x29;

    #[derive(Debug, PartialEq, Eq)]
    pub struct Request<'frame> {
        pub session: u64,
        pub request_id: u64,
        pub owner: &'frame [u8],
        pub name: &'frame [u8],
        pub deployment_generation: u64,
    }

    impl Request<'_> {
        /// Prepare identities and terminal reply storage before admission.
        pub fn prepare(&self) -> Result<(Vec<u8>, Vec<u8>, PreparedReply), EncodeError> {
            validate(self.owner, self.name)?;
            if self.session == 0 || self.request_id == 0 {
                return Err(EncodeError::InvalidIdentity);
            }
            let mut owner = buffer(self.owner.len())?;
            owner.extend_from_slice(self.owner);
            let mut name = buffer(self.name.len())?;
            name.extend_from_slice(self.name);
            Ok((owner, name, PreparedReply::new(self.session, self.request_id)?))
        }
    }

    pub fn encode_request(
        session: u64,
        request_id: u64,
        owner: &[u8],
        name: &[u8],
        deployment_generation: u64,
    ) -> Result<Vec<u8>, EncodeError> {
        validate(owner, name)?;
        if session == 0 || request_id == 0 {
            return Err(EncodeError::InvalidIdentity);
        }
        let mut bytes = buffer(32 + owner.len() + name.len())?;
        bytes.extend_from_slice(&session.to_le_bytes());
        bytes.extend_from_slice(&request_id.to_le_bytes());
        identities(&mut bytes, owner, name);
        bytes.extend_from_slice(&deployment_generation.to_le_bytes());
        Ok(bytes)
    }

    /// Validate the entire frame without allocating or copying identities.
    pub fn decode_request(frame: &[u8]) -> Option<Request<'_>> {
        if frame.first().copied()? != TAG_REQUEST {
            return None;
        }
        let session = u64::from_le_bytes(frame.get(1..9)?.try_into().ok()?);
        let request_id = u64::from_le_bytes(frame.get(9..17)?.try_into().ok()?);
        if session == 0 || request_id == 0 {
            return None;
        }
        let (owner, name, deployment_generation) = decode_identities(frame.get(17..)?)?;
        Some(Request {
            session,
            request_id,
            owner,
            name,
            deployment_generation,
        })
    }

    /// Admit the complete reply before Raft mutation. Completion patches only
    /// the fixed generation field and transfers the original allocation.
    pub struct PreparedReply(pub(super) Vec<u8>);

    impl PreparedReply {
        pub fn new(session: u64, request_id: u64) -> Result<Self, EncodeError> {
            if session == 0 || request_id == 0 {
                return Err(EncodeError::InvalidIdentity);
            }
            let mut bytes = buffer(24)?;
            bytes.extend_from_slice(&session.to_le_bytes());
            bytes.extend_from_slice(&request_id.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            Ok(Self(bytes))
        }

        pub fn finish(mut self, generation: u64) -> Vec<u8> {
            self.0[16..24].copy_from_slice(&generation.to_le_bytes());
            self.0
        }
    }

    pub fn decode_reply(frame: &[u8]) -> Option<(u64, u64, u64)> {
        if frame.len() != 25 || frame[0] != TAG_REPLY {
            return None;
        }
        let session = u64::from_le_bytes(frame[1..9].try_into().ok()?);
        let request_id = u64::from_le_bytes(frame[9..17].try_into().ok()?);
        if session == 0 || request_id == 0 {
            return None;
        }
        Some((session, request_id, u64::from_le_bytes(frame[17..25].try_into().ok()?)))
    }

    /// Match the captured leader and exact pending episode before consuming
    /// any local publication or caller reply authority.
    pub fn reply_matches(
        expected_peer: &str,
        expected_session: u64,
        expected_request: u64,
        source_peer: &str,
        session: u64,
        request_id: u64,
    ) -> bool {
        expected_peer == source_peer
            && expected_session == session
            && expected_request == request_id
    }
}

/// Retraction: 0x2a | owner_len:u32 | owner | name_len:u32 | name |
/// generation:u64. Generation must be nonzero; repeated delivery remains
/// fenced by the exact catalog generation, never an unconditional delete.
pub mod runregister {
    pub use super::EncodeError;
    use super::*;

    pub const TAG_REQUEST: u8 = 0x2a;

    pub fn encode_request(
        owner: &[u8],
        name: &[u8],
        generation: u64,
    ) -> Result<Vec<u8>, EncodeError> {
        validate(owner, name)?;
        if generation == 0 {
            return Err(EncodeError::InvalidIdentity);
        }
        let mut bytes = buffer(16 + owner.len() + name.len())?;
        identities(&mut bytes, owner, name);
        bytes.extend_from_slice(&generation.to_le_bytes());
        Ok(bytes)
    }

    pub fn decode_request(frame: &[u8]) -> Option<(&[u8], &[u8], u64)> {
        if frame.first().copied()? != TAG_REQUEST {
            return None;
        }
        let result = decode_identities(frame.get(1..)?)?;
        (result.2 != 0).then_some(result)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn tagged(tag: u8, body: &[u8]) -> Vec<u8> {
        [&[tag], body].concat()
    }

    #[test]
    fn full_identity_boundaries_and_explicit_deployment_generation() {
        let owner = vec![b'o'; MAX_OWNER_BYTES];
        let name = vec![b'n'; crate::MAX_NAME_LEN];
        for generation in [0, 7, u64::MAX] {
            let body = rregister::encode_request(3, 9, &owner, &name, generation).unwrap();
            let frame = tagged(rregister::TAG_REQUEST, &body);
            let request = rregister::decode_request(&frame).unwrap();
            assert_eq!(request.owner, owner);
            assert_eq!(request.name, name);
            assert_eq!(request.deployment_generation, generation);
            assert_eq!((request.session, request.request_id), (3, 9));
        }
        let frame = tagged(
            runregister::TAG_REQUEST,
            &runregister::encode_request(&owner, &name, 7).unwrap(),
        );
        assert_eq!(
            runregister::decode_request(&frame),
            Some((owner.as_slice(), name.as_slice(), 7))
        );
    }

    #[test]
    fn writers_reject_invalid_fields_without_truncation() {
        for (owner, name) in [(&b""[..], &b"n"[..]), (b"o", b"")] {
            assert!(rregister::encode_request(1, 1, owner, name, 0).is_err());
            assert!(runregister::encode_request(owner, name, 1).is_err());
        }
        let owner = vec![0; MAX_OWNER_BYTES + 1];
        let name = vec![0; crate::MAX_NAME_LEN + 1];
        assert!(rregister::encode_request(1, 1, &owner, b"n", 0).is_err());
        assert!(rregister::encode_request(1, 1, b"o", &name, 0).is_err());
        assert!(runregister::encode_request(&owner, b"n", 1).is_err());
        assert!(runregister::encode_request(b"o", &name, 1).is_err());
        assert!(runregister::encode_request(b"o", b"n", 0).is_err());
        for (session, id) in [(0, 1), (1, 0)] {
            assert!(rregister::encode_request(session, id, b"o", b"n", 0).is_err());
            assert!(rregister::PreparedReply::new(session, id).is_err());
        }
    }

    #[test]
    fn golden_frames_and_complete_invalid_records_reject() {
        let body = rregister::encode_request(3, 9, b"o", b"n", 7).unwrap();
        assert_eq!(
            body,
            [
                3u64.to_le_bytes().as_slice(),
                9u64.to_le_bytes().as_slice(),
                1u32.to_le_bytes().as_slice(),
                b"o",
                1u32.to_le_bytes().as_slice(),
                b"n",
                7u64.to_le_bytes().as_slice()
            ]
            .concat()
        );
        let mut old = vec![1, b'o', 1, b'n'];
        old.extend_from_slice(&7u64.to_le_bytes());
        assert!(rregister::decode_request(&tagged(rregister::TAG_REQUEST, &old)).is_none());
        assert!(runregister::decode_request(&tagged(runregister::TAG_REQUEST, &old)).is_none());
        for (owner, name) in [
            (vec![b'o'; MAX_OWNER_BYTES + 1], vec![b'n']),
            (vec![b'o'], vec![b'n'; crate::MAX_NAME_LEN + 1]),
            (Vec::new(), vec![b'n']),
            (vec![b'o'], Vec::new()),
        ] {
            // Manually complete the invalid frame so rejection tests bounds,
            // rather than merely discovering insufficient payload bytes.
            let mut payload = Vec::new();
            identities(&mut payload, &owner, &name);
            payload.extend_from_slice(&7u64.to_le_bytes());
            assert!(
                runregister::decode_request(&tagged(runregister::TAG_REQUEST, &payload)).is_none()
            );
            let request =
                [3u64.to_le_bytes().as_slice(), 9u64.to_le_bytes().as_slice(), &payload].concat();
            assert!(rregister::decode_request(&tagged(rregister::TAG_REQUEST, &request)).is_none());
            assert!(
                rregister::Request {
                    session: 3,
                    request_id: 9,
                    owner: &owner,
                    name: &name,
                    deployment_generation: 0
                }
                .prepare()
                .is_err()
            );
        }
        let mut zero =
            tagged(runregister::TAG_REQUEST, &runregister::encode_request(b"o", b"n", 1).unwrap());
        let end = zero.len();
        zero[end - 8..].fill(0);
        assert!(runregister::decode_request(&zero).is_none());
        let left = vec![b'n'; 256];
        let mut right = left.clone();
        right[255] = b'r';
        let a = rregister::encode_request(3, 9, b"o", &left, 0).unwrap();
        let b = rregister::encode_request(3, 9, b"o", &right, 0).unwrap();
        assert_ne!(a, b);
        assert_eq!(
            rregister::decode_request(&tagged(rregister::TAG_REQUEST, &b)).unwrap().name,
            right
        );
    }

    #[test]
    fn malformed_requests_and_retired_tags_reject() {
        let register = tagged(
            rregister::TAG_REQUEST,
            &rregister::encode_request(3, 9, b"owner", b"name", 7).unwrap(),
        );
        let unregister = tagged(
            runregister::TAG_REQUEST,
            &runregister::encode_request(b"owner", b"name", 7).unwrap(),
        );
        for (frame, decode) in [(register, true), (unregister, false)] {
            let accepts = |bytes: &[u8]| {
                if decode {
                    rregister::decode_request(bytes).is_some()
                } else {
                    runregister::decode_request(bytes).is_some()
                }
            };
            for end in 0..frame.len() {
                assert!(!accepts(&frame[..end]));
            }
            let mut extra = frame.clone();
            extra.push(0);
            assert!(!accepts(&extra));
            let mut oversized = frame.clone();
            let offset = if decode {
                17
            } else {
                1
            };
            oversized[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(!accepts(&oversized));
            for tag in [0x14, 0x15, 0x16] {
                let mut retired = frame.clone();
                retired[0] = tag;
                assert!(!accepts(&retired));
            }
        }
        let mut frame = tagged(
            rregister::TAG_REQUEST,
            &rregister::encode_request(3, 9, b"o", b"n", 7).unwrap(),
        );
        frame.truncate(frame.len() - 8); // No missing-generation compatibility default.
        assert!(rregister::decode_request(&frame).is_none());
    }

    #[test]
    fn replies_preserve_prepared_allocation_and_reject_wrong_episodes() {
        for generation in [0, 7, u64::MAX] {
            let prepared = rregister::PreparedReply::new(3, 9).unwrap();
            let pointer = prepared.0.as_ptr();
            let body = prepared.finish(generation);
            assert_eq!(body.as_ptr(), pointer);
            let frame = tagged(rregister::TAG_REPLY, &body);
            assert_eq!(rregister::decode_reply(&frame), Some((3, 9, generation)));
            for end in 0..frame.len() {
                assert!(rregister::decode_reply(&frame[..end]).is_none());
            }
            let mut extra = frame.clone();
            extra.push(0);
            assert!(rregister::decode_reply(&extra).is_none());
            for offset in [1, 9] {
                let mut zero = frame.clone();
                zero[offset..offset + 8].fill(0);
                assert!(rregister::decode_reply(&zero).is_none());
            }
            let mut retired = frame;
            retired[0] = 0x16;
            assert!(rregister::decode_reply(&retired).is_none());
        }
        assert!(rregister::reply_matches("leader", 3, 9, "leader", 3, 9));
        for (peer, session, request) in [("other", 3, 9), ("leader", 2, 9), ("leader", 3, 8)] {
            assert!(!rregister::reply_matches("leader", 3, 9, peer, session, request));
        }
    }
}
