//! Operator-signed cluster ingress assignment policy.
//!
//! `CINGPOL1` carries one complete replacement of the bounded ingress table.
//! It is signed by the operations authority, scoped to one cluster, and
//! bounded by trusted UTC. The complete envelope enters Raft so every member
//! derives service identities from the same committed revision.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use crate::{
    ingress,
    sha256,
};

pub const MAGIC: &[u8; 8] = b"CINGPOL1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 160;
pub const KEY_ID_OFFSET: usize = 72;
pub const KEY_ID_LEN: usize = 16;
pub const SIGNATURE_OFFSET: usize = 96;
pub const SIGNATURE_LEN: usize = 64;
pub const MAX_ENCODED_LEN: usize = HEADER_LEN + ingress::MAX_ENCODED_LEN;
/// The envelope may be presented during this bounded admission window. Once
/// committed, the desired policy remains active until a newer signed record.
pub const MAX_VALIDITY_SECONDS: u64 = crate::shutdown::MAX_VALIDITY_SECONDS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyFields<'a> {
    pub sequence: u64,
    pub not_before_unix_seconds: u64,
    pub expires_unix_seconds: u64,
    pub cluster_id: [u8; 32],
    pub assignments: &'a [ingress::ServiceBinding<'a>],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Policy<'a> {
    bytes: &'a [u8],
    pub sequence: u64,
    pub not_before_unix_seconds: u64,
    pub expires_unix_seconds: u64,
    pub cluster_id: [u8; 32],
    assignments: &'a [u8],
}

impl<'a> Policy<'a> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn assignments(&self) -> ingress::ServiceBindings<'a> {
        ingress::decode(self.assignments).expect("validated ingress policy assignments")
    }

    pub fn assignment_bytes(&self) -> &'a [u8] {
        self.assignments
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall,
    InvalidAssignments(ingress::EncodeError),
    InvalidFields,
    TooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    Valid,
    Invalid,
    WrongCluster,
    WrongKey,
}

pub fn encoded_len(fields: &PolicyFields<'_>) -> Result<usize, EncodeError> {
    if fields.sequence == 0
        || fields.not_before_unix_seconds == 0
        || fields.expires_unix_seconds < fields.not_before_unix_seconds
        || fields.expires_unix_seconds.saturating_sub(fields.not_before_unix_seconds)
            > MAX_VALIDITY_SECONDS
        || fields.cluster_id.iter().all(|byte| *byte == 0)
    {
        return Err(EncodeError::InvalidFields);
    }
    HEADER_LEN
        .checked_add(
            ingress::encoded_len(fields.assignments).map_err(EncodeError::InvalidAssignments)?,
        )
        .filter(|len| *len <= MAX_ENCODED_LEN)
        .ok_or(EncodeError::TooLarge)
}

pub fn encode_unsigned(
    fields: &PolicyFields<'_>,
    operations_public_key: &[u8; 32],
    output: &mut [u8],
) -> Result<usize, EncodeError> {
    let len = encoded_len(fields)?;
    if output.len() < len {
        return Err(EncodeError::BufferTooSmall);
    }
    let bytes = &mut output[..len];
    bytes.fill(0);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(len as u32).to_le_bytes());
    bytes[16..24].copy_from_slice(&fields.sequence.to_le_bytes());
    bytes[24..32].copy_from_slice(&fields.not_before_unix_seconds.to_le_bytes());
    bytes[32..40].copy_from_slice(&fields.expires_unix_seconds.to_le_bytes());
    bytes[40..72].copy_from_slice(&fields.cluster_id);
    bytes[KEY_ID_OFFSET..KEY_ID_OFFSET + KEY_ID_LEN]
        .copy_from_slice(&sha256::digest(operations_public_key)[..KEY_ID_LEN]);
    let assignments_len = len - HEADER_LEN;
    bytes[88..92].copy_from_slice(&(assignments_len as u32).to_le_bytes());
    ingress::encode(fields.assignments, &mut bytes[HEADER_LEN..])
        .map_err(EncodeError::InvalidAssignments)?;
    Ok(len)
}

pub fn decode(bytes: &[u8]) -> Option<Policy<'_>> {
    if bytes.len() < HEADER_LEN
        || bytes.len() > MAX_ENCODED_LEN
        || bytes.get(..8)? != MAGIC
        || read_u16(bytes, 8)? != VERSION
        || usize::from(read_u16(bytes, 10)?) != HEADER_LEN
        || usize::try_from(read_u32(bytes, 12)?).ok()? != bytes.len()
        || bytes.get(92..96)?.iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let sequence = read_u64(bytes, 16)?;
    let not_before_unix_seconds = read_u64(bytes, 24)?;
    let expires_unix_seconds = read_u64(bytes, 32)?;
    let cluster_id: [u8; 32] = bytes.get(40..72)?.try_into().ok()?;
    let assignment_len = usize::try_from(read_u32(bytes, 88)?).ok()?;
    if sequence == 0
        || not_before_unix_seconds == 0
        || expires_unix_seconds < not_before_unix_seconds
        || expires_unix_seconds.saturating_sub(not_before_unix_seconds) > MAX_VALIDITY_SECONDS
        || cluster_id.iter().all(|byte| *byte == 0)
        || HEADER_LEN.checked_add(assignment_len)? != bytes.len()
    {
        return None;
    }
    let assignments = bytes.get(HEADER_LEN..)?;
    ingress::decode(assignments)?;
    Some(Policy {
        bytes,
        sequence,
        not_before_unix_seconds,
        expires_unix_seconds,
        cluster_id,
        assignments,
    })
}

pub fn signature_digest(bytes: &[u8]) -> Option<[u8; 32]> {
    decode(bytes)?;
    Some(sha256::digest_skipping(bytes, SIGNATURE_OFFSET, SIGNATURE_LEN))
}

pub fn set_signature(bytes: &mut [u8], signature: &[u8; SIGNATURE_LEN]) -> bool {
    if decode(bytes).is_none() {
        return false;
    }
    bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN].copy_from_slice(signature);
    true
}

pub fn verify(
    bytes: &[u8],
    cluster_id: &[u8; 32],
    operations_public_key: &[u8; 32],
) -> VerifyOutcome {
    let Some(policy) = decode(bytes) else {
        return VerifyOutcome::Invalid;
    };
    if &policy.cluster_id != cluster_id {
        return VerifyOutcome::WrongCluster;
    }
    if bytes[KEY_ID_OFFSET..KEY_ID_OFFSET + KEY_ID_LEN]
        != sha256::digest(operations_public_key)[..KEY_ID_LEN]
    {
        return VerifyOutcome::WrongKey;
    }
    let Ok(public_key) = PublicKey::from_slice(operations_public_key) else {
        return VerifyOutcome::Invalid;
    };
    let Ok(signature) =
        Signature::from_slice(&bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN])
    else {
        return VerifyOutcome::Invalid;
    };
    let digest = sha256::digest_skipping(bytes, SIGNATURE_OFFSET, SIGNATURE_LEN);
    if public_key.verify(digest, &signature).is_ok() {
        VerifyOutcome::Valid
    } else {
        VerifyOutcome::Invalid
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}
