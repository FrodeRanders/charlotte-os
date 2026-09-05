//! Signed, node-targeted cluster shutdown intents.
//!
//! The intent is authored by the cluster operator and can be submitted to any
//! member. The Raft leader verifies it before committing the desired node
//! transition; the target node derives its local monotonic deadline only after
//! observing the committed record. Absolute monotonic timestamps therefore
//! never cross machines.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use crate::sha256;

pub const MAGIC: &[u8; 8] = b"CSHUTDN1";
pub const VERSION: u16 = 1;
pub const ENCODED_LEN: usize = 144;
pub const KEY_ID_OFFSET: usize = 64;
pub const KEY_ID_LEN: usize = 16;
pub const SIGNATURE_OFFSET: usize = 80;
pub const SIGNATURE_LEN: usize = 64;
pub const REASON_POWER_OFF: u32 = 1;
pub const MAX_VALIDITY_SECONDS: u64 = 24 * 60 * 60;
pub const MAX_NODE_GRACE_MS: u32 = 15 * 60 * 1_000;
pub const MAX_PHASE_GRACE_MS: u32 = 60 * 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownFields {
    /// Monotonic operator sequence for this target node.
    pub sequence: u64,
    /// Stable cluster node key. Zero is deliberately not a broadcast target.
    pub target_node: u64,
    pub not_before_unix_seconds: u64,
    pub expires_unix_seconds: u64,
    /// Complete time budget derived into the target node's monotonic epoch.
    pub node_grace_ms: u32,
    /// Maximum budget for each ordinary service phase.
    pub phase_grace_ms: u32,
    pub reason: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall,
    InvalidFields,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    Valid,
    Invalid,
    WrongKey,
}

fn valid(fields: &ShutdownFields) -> bool {
    fields.sequence != 0
        && fields.target_node != 0
        && fields.not_before_unix_seconds != 0
        && fields.expires_unix_seconds >= fields.not_before_unix_seconds
        && fields.expires_unix_seconds.saturating_sub(fields.not_before_unix_seconds)
            <= MAX_VALIDITY_SECONDS
        && fields.node_grace_ms != 0
        && fields.node_grace_ms <= MAX_NODE_GRACE_MS
        && fields.phase_grace_ms != 0
        && fields.phase_grace_ms <= MAX_PHASE_GRACE_MS
        && fields.phase_grace_ms <= fields.node_grace_ms
        && fields.reason == REASON_POWER_OFF
}

/// Encode a canonical intent whose signature field is all zero.
pub fn encode_unsigned(
    fields: &ShutdownFields,
    public_key: &[u8; 32],
    output: &mut [u8],
) -> Result<usize, EncodeError> {
    if output.len() < ENCODED_LEN {
        return Err(EncodeError::BufferTooSmall);
    }
    if !valid(fields) {
        return Err(EncodeError::InvalidFields);
    }
    let bytes = &mut output[..ENCODED_LEN];
    bytes.fill(0);
    bytes[0..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(ENCODED_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(ENCODED_LEN as u32).to_le_bytes());
    bytes[16..24].copy_from_slice(&fields.sequence.to_le_bytes());
    bytes[24..32].copy_from_slice(&fields.target_node.to_le_bytes());
    bytes[32..40].copy_from_slice(&fields.not_before_unix_seconds.to_le_bytes());
    bytes[40..48].copy_from_slice(&fields.expires_unix_seconds.to_le_bytes());
    bytes[48..52].copy_from_slice(&fields.node_grace_ms.to_le_bytes());
    bytes[52..56].copy_from_slice(&fields.phase_grace_ms.to_le_bytes());
    bytes[56..60].copy_from_slice(&fields.reason.to_le_bytes());
    bytes[KEY_ID_OFFSET..SIGNATURE_OFFSET]
        .copy_from_slice(&sha256::digest(public_key)[..KEY_ID_LEN]);
    Ok(ENCODED_LEN)
}

pub fn decode(bytes: &[u8]) -> Option<ShutdownFields> {
    if bytes.len() != ENCODED_LEN
        || bytes.get(0..8)? != MAGIC
        || read_u16(bytes, 8)? != VERSION
        || usize::from(read_u16(bytes, 10)?) != ENCODED_LEN
        || usize::try_from(read_u32(bytes, 12)?).ok()? != ENCODED_LEN
        || read_u32(bytes, 60)? != 0
    {
        return None;
    }
    let fields = ShutdownFields {
        sequence: read_u64(bytes, 16)?,
        target_node: read_u64(bytes, 24)?,
        not_before_unix_seconds: read_u64(bytes, 32)?,
        expires_unix_seconds: read_u64(bytes, 40)?,
        node_grace_ms: read_u32(bytes, 48)?,
        phase_grace_ms: read_u32(bytes, 52)?,
        reason: read_u32(bytes, 56)?,
    };
    valid(&fields).then_some(fields)
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

pub fn verify(bytes: &[u8], public_key_bytes: &[u8; 32]) -> VerifyOutcome {
    if decode(bytes).is_none() {
        return VerifyOutcome::Invalid;
    }
    if bytes[KEY_ID_OFFSET..SIGNATURE_OFFSET] != sha256::digest(public_key_bytes)[..KEY_ID_LEN] {
        return VerifyOutcome::WrongKey;
    }
    let Ok(public_key) = PublicKey::from_slice(public_key_bytes) else {
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
