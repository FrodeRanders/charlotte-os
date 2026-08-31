//! Signed, bounded multi-component release envelopes.
//!
//! Each nested [`crate::deployment`] descriptor independently authorizes one
//! artifact deployment. This outer envelope additionally binds the exact set
//! to one named, monotonic release so the cluster can admit it with one Raft
//! command. Executable bytes and object-store credentials remain outside the
//! envelope.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use crate::{
    deployment,
    sha256,
};

pub const MAGIC: &[u8; 8] = b"CRELEASE";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 112;
pub const KEY_ID_OFFSET: usize = 32;
pub const SIGNATURE_OFFSET: usize = 48;
pub const SIGNATURE_LEN: usize = 64;
pub const MAX_RELEASE_LEN: usize = 3584;
pub const MAX_RELEASE_NAME_LEN: usize = 48;
pub const MAX_DESCRIPTORS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseFields<'a> {
    /// Monotonic revision within `release_name`.
    pub sequence: u64,
    pub release_name: &'a [u8],
    /// Canonical, independently signed `CDEPLOY1` through `CDEPLOY4`
    /// descriptors.
    pub descriptors: &'a [&'a [u8]],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall,
    DuplicateArtifact,
    InvalidDescriptor,
    InvalidReleaseName,
    InvalidSequence,
    TooLarge,
    TooManyDescriptors,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    Valid,
    Invalid,
    WrongKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseEnvelope<'a> {
    bytes: &'a [u8],
    pub sequence: u64,
    pub release_name: &'a [u8],
    descriptors_offset: usize,
    descriptor_count: usize,
}

impl<'a> ReleaseEnvelope<'a> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn descriptors(&self) -> Descriptors<'a> {
        Descriptors {
            bytes: self.bytes,
            offset: self.descriptors_offset,
            remaining: self.descriptor_count,
        }
    }
}

pub struct Descriptors<'a> {
    bytes: &'a [u8],
    offset: usize,
    remaining: usize,
}

impl<'a> Iterator for Descriptors<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let len = usize::from(read_u16(self.bytes, self.offset)?);
        let start = self.offset.checked_add(2)?;
        let end = start.checked_add(len)?;
        let descriptor = self.bytes.get(start..end)?;
        self.offset = end;
        self.remaining -= 1;
        Some(descriptor)
    }
}

pub fn valid_release_name(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RELEASE_NAME_LEN
        && value.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

fn validate_descriptors(descriptors: &[&[u8]]) -> Result<(), EncodeError> {
    if descriptors.is_empty() || descriptors.len() > MAX_DESCRIPTORS {
        return Err(EncodeError::TooManyDescriptors);
    }
    for (index, bytes) in descriptors.iter().enumerate() {
        let descriptor = deployment::decode(bytes).ok_or(EncodeError::InvalidDescriptor)?;
        for previous in &descriptors[..index] {
            let previous = deployment::decode(previous).ok_or(EncodeError::InvalidDescriptor)?;
            if previous.artifact_name == descriptor.artifact_name {
                return Err(EncodeError::DuplicateArtifact);
            }
        }
    }
    Ok(())
}

pub fn encoded_len(fields: &ReleaseFields<'_>) -> Result<usize, EncodeError> {
    if fields.sequence == 0 {
        return Err(EncodeError::InvalidSequence);
    }
    if !valid_release_name(fields.release_name) {
        return Err(EncodeError::InvalidReleaseName);
    }
    validate_descriptors(fields.descriptors)?;
    let mut len = HEADER_LEN.checked_add(fields.release_name.len()).ok_or(EncodeError::TooLarge)?;
    for descriptor in fields.descriptors {
        len = len.checked_add(2 + descriptor.len()).ok_or(EncodeError::TooLarge)?;
    }
    if len > MAX_RELEASE_LEN {
        return Err(EncodeError::TooLarge);
    }
    Ok(len)
}

/// Encode a canonical envelope with an all-zero signature field.
pub fn encode_unsigned(
    fields: &ReleaseFields<'_>,
    public_key: &[u8; 32],
    output: &mut [u8],
) -> Result<usize, EncodeError> {
    let len = encoded_len(fields)?;
    if output.len() < len {
        return Err(EncodeError::BufferTooSmall);
    }
    let bytes = &mut output[..len];
    bytes.fill(0);
    bytes[0..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(len as u32).to_le_bytes());
    bytes[16..24].copy_from_slice(&fields.sequence.to_le_bytes());
    bytes[24..26].copy_from_slice(&(fields.release_name.len() as u16).to_le_bytes());
    bytes[26..28].copy_from_slice(&(fields.descriptors.len() as u16).to_le_bytes());
    bytes[KEY_ID_OFFSET..SIGNATURE_OFFSET]
        .copy_from_slice(&sha256::digest(public_key)[..deployment::KEY_ID_LEN]);

    let mut offset = HEADER_LEN;
    let name_end = offset + fields.release_name.len();
    bytes[offset..name_end].copy_from_slice(fields.release_name);
    offset = name_end;
    for descriptor in fields.descriptors {
        bytes[offset..offset + 2].copy_from_slice(&(descriptor.len() as u16).to_le_bytes());
        offset += 2;
        let end = offset + descriptor.len();
        bytes[offset..end].copy_from_slice(descriptor);
        offset = end;
    }
    debug_assert_eq!(offset, len);
    Ok(len)
}

pub fn decode(bytes: &[u8]) -> Option<ReleaseEnvelope<'_>> {
    if bytes.len() < HEADER_LEN
        || bytes.get(0..8)? != MAGIC
        || read_u16(bytes, 8)? != VERSION
        || usize::from(read_u16(bytes, 10)?) != HEADER_LEN
    {
        return None;
    }
    let total_len = usize::try_from(read_u32(bytes, 12)?).ok()?;
    if total_len != bytes.len() || total_len > MAX_RELEASE_LEN || read_u32(bytes, 28)? != 0 {
        return None;
    }
    let sequence = read_u64(bytes, 16)?;
    let name_len = usize::from(read_u16(bytes, 24)?);
    let descriptor_count = usize::from(read_u16(bytes, 26)?);
    if sequence == 0 || descriptor_count == 0 || descriptor_count > MAX_DESCRIPTORS {
        return None;
    }
    let name_end = HEADER_LEN.checked_add(name_len)?;
    let release_name = bytes.get(HEADER_LEN..name_end)?;
    if !valid_release_name(release_name) {
        return None;
    }
    let mut descriptors = Descriptors {
        bytes,
        offset: name_end,
        remaining: descriptor_count,
    };
    let mut names: [&[u8]; MAX_DESCRIPTORS] = [&[]; MAX_DESCRIPTORS];
    for index in 0..descriptor_count {
        let descriptor = deployment::decode(descriptors.next()?)?;
        if names[..index].contains(&descriptor.artifact_name) {
            return None;
        }
        names[index] = descriptor.artifact_name;
    }
    if descriptors.offset != total_len || descriptors.remaining != 0 {
        return None;
    }
    Some(ReleaseEnvelope {
        bytes,
        sequence,
        release_name,
        descriptors_offset: name_end,
        descriptor_count,
    })
}

pub fn signature_digest(bytes: &[u8]) -> Option<[u8; 32]> {
    let envelope = decode(bytes)?;
    Some(sha256::digest_skipping(envelope.bytes, SIGNATURE_OFFSET, SIGNATURE_LEN))
}

pub fn set_signature(bytes: &mut [u8], signature: &[u8; SIGNATURE_LEN]) -> bool {
    if decode(bytes).is_none() {
        return false;
    }
    bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN].copy_from_slice(signature);
    true
}

/// Verify the outer release signature and every nested deployment signature
/// against the same cluster key.
pub fn verify(bytes: &[u8], public_key_bytes: &[u8; 32]) -> VerifyOutcome {
    let Some(envelope) = decode(bytes) else {
        return VerifyOutcome::Invalid;
    };
    if envelope.bytes[KEY_ID_OFFSET..SIGNATURE_OFFSET]
        != sha256::digest(public_key_bytes)[..deployment::KEY_ID_LEN]
    {
        return VerifyOutcome::WrongKey;
    }
    let Ok(public_key) = PublicKey::from_slice(public_key_bytes) else {
        return VerifyOutcome::Invalid;
    };
    let Ok(signature) =
        Signature::from_slice(&envelope.bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN])
    else {
        return VerifyOutcome::Invalid;
    };
    let digest = sha256::digest_skipping(envelope.bytes, SIGNATURE_OFFSET, SIGNATURE_LEN);
    if public_key.verify(digest, &signature).is_err() {
        return VerifyOutcome::Invalid;
    }
    for descriptor in envelope.descriptors() {
        match deployment::verify(descriptor, public_key_bytes) {
            deployment::VerifyOutcome::Valid => {}
            deployment::VerifyOutcome::WrongKey => return VerifyOutcome::WrongKey,
            deployment::VerifyOutcome::Invalid => return VerifyOutcome::Invalid,
        }
    }
    VerifyOutcome::Valid
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
