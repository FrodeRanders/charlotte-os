//! Signed, bounded deployment descriptors for off-cluster publication.
//!
//! An artifact signature blesses executable bytes and their logical identity.
//! This separate descriptor authenticates the operational decision to fetch a
//! particular immutable object, place it on a node, and grant named service
//! capabilities. Object-store credentials and network addresses are
//! deliberately absent: a node resolves `object_key` through its locally
//! configured, capability-scoped object-store connector.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use crate::sha256;

pub const MAGIC: &[u8; 8] = b"CDEPLOY4";
pub const VERSION: u16 = 4;
pub const V3_MAGIC: &[u8; 8] = b"CDEPLOY3";
pub const V3_VERSION: u16 = 3;
pub const V2_MAGIC: &[u8; 8] = b"CDEPLOY2";
pub const V2_VERSION: u16 = 2;
pub const LEGACY_MAGIC: &[u8; 8] = b"CDEPLOY1";
pub const LEGACY_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 158;
pub const LEGACY_HEADER_LEN: usize = 152;
pub const MIN_HEADER_LEN: usize = LEGACY_HEADER_LEN;
pub const SIGNATURE_OFFSET: usize = 88;
pub const SIGNATURE_LEN: usize = 64;
pub const MAX_THREADS_OFFSET: usize = SIGNATURE_OFFSET + SIGNATURE_LEN;
pub const SHUTDOWN_GRACE_MS_OFFSET: usize = MAX_THREADS_OFFSET + 2;
pub const KEY_ID_LEN: usize = 16;
/// Leaves room in one 4 KiB IPC memory object for an acquisition header and
/// the requested service name.
pub const MAX_DESCRIPTOR_LEN: usize = 3584;
pub const MAX_ARTIFACT_NAME_LEN: usize = 48;
pub const MAX_OBJECT_KEY_LEN: usize = 1024;
pub const MAX_SERVICE_NAME_LEN: usize = 128;
pub const MAX_GRANTS: usize = 64;

pub const RIGHT_SEND: u16 = 1 << 0;
pub const RIGHT_CALL: u16 = 1 << 1;
/// Permit the artifact to publish an endpoint under the exact service name.
pub const RIGHT_PUBLISH: u16 = 1 << 2;
pub const CLIENT_RIGHTS: u16 = RIGHT_SEND | RIGHT_CALL;
pub const ALL_GRANT_RIGHTS: u16 = CLIENT_RIGHTS | RIGHT_PUBLISH;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityGrant<'a> {
    pub service: &'a [u8],
    pub rights: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DescriptorFields<'a> {
    /// Monotonic deployment revision used to reject stale notifications.
    pub sequence: u64,
    /// Cluster node identity selected by placement policy.
    pub node_key: u64,
    /// SHA-256 of the exact signed ELF stored at `object_key`.
    pub artifact_digest: [u8; 32],
    pub artifact_name: &'a [u8],
    /// Kernel-enforced stack allocation inherited by every thread in the
    /// protected domain, expressed in 4 KiB pages.
    pub stack_pages_per_thread: u16,
    /// Maximum active threads in the protected domain, including its
    /// bootstrap thread.
    pub max_threads: u16,
    /// Maximum cooperative drain interval before forced domain retirement.
    pub shutdown_grace_ms: u32,
    /// Opaque key interpreted inside the node's preconfigured object-store
    /// connector. It contains neither a bucket endpoint nor credentials.
    pub object_key: &'a [u8],
    pub grants: &'a [CapabilityGrant<'a>],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall,
    InvalidArtifactName,
    InvalidGrant,
    InvalidObjectKey,
    InvalidSequence,
    InvalidStackPages,
    InvalidMaxThreads,
    InvalidShutdownGrace,
    TooLarge,
    TooManyGrants,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    Valid,
    Invalid,
    WrongKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeploymentDescriptor<'a> {
    bytes: &'a [u8],
    pub format_version: u16,
    pub sequence: u64,
    pub node_key: u64,
    pub artifact_digest: [u8; 32],
    pub artifact_name: &'a [u8],
    pub stack_pages_per_thread: u16,
    pub max_threads: u16,
    pub shutdown_grace_ms: u32,
    pub object_key: &'a [u8],
    grants_offset: usize,
    grant_count: usize,
}

impl<'a> DeploymentDescriptor<'a> {
    pub fn grants(&self) -> CapabilityGrants<'a> {
        CapabilityGrants {
            bytes: self.bytes,
            offset: self.grants_offset,
            remaining: self.grant_count,
        }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

pub struct CapabilityGrants<'a> {
    bytes: &'a [u8],
    offset: usize,
    remaining: usize,
}

impl<'a> Iterator for CapabilityGrants<'a> {
    type Item = CapabilityGrant<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let rights = read_u16(self.bytes, self.offset)?;
        let name_len = usize::from(read_u16(self.bytes, self.offset + 2)?);
        let start = self.offset.checked_add(4)?;
        let end = start.checked_add(name_len)?;
        let service = self.bytes.get(start..end)?;
        self.offset = end;
        self.remaining -= 1;
        Some(CapabilityGrant {
            service,
            rights,
        })
    }
}

fn valid_name(value: &[u8], maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

/// Whether `value` is a valid logical artifact name in a signed deployment
/// descriptor and in the deployment ingress URL.
pub fn valid_artifact_name(value: &[u8]) -> bool {
    valid_name(value, MAX_ARTIFACT_NAME_LEN)
}

fn valid_rights(rights: u16) -> bool {
    rights != 0 && rights & !ALL_GRANT_RIGHTS == 0
}

pub fn encoded_len(fields: &DescriptorFields<'_>) -> Result<usize, EncodeError> {
    if fields.sequence == 0 {
        return Err(EncodeError::InvalidSequence);
    }
    if !(1..=crate::MAX_USER_STACK_PAGES).contains(&usize::from(fields.stack_pages_per_thread)) {
        return Err(EncodeError::InvalidStackPages);
    }
    if !(1..=crate::MAX_USER_THREADS).contains(&usize::from(fields.max_threads)) {
        return Err(EncodeError::InvalidMaxThreads);
    }
    if fields.shutdown_grace_ms > crate::MAX_SHUTDOWN_GRACE_MS {
        return Err(EncodeError::InvalidShutdownGrace);
    }
    if !valid_artifact_name(fields.artifact_name) {
        return Err(EncodeError::InvalidArtifactName);
    }
    if !valid_name(fields.object_key, MAX_OBJECT_KEY_LEN) {
        return Err(EncodeError::InvalidObjectKey);
    }
    if fields.grants.len() > MAX_GRANTS {
        return Err(EncodeError::TooManyGrants);
    }
    let mut len = HEADER_LEN
        .checked_add(fields.artifact_name.len())
        .and_then(|len| len.checked_add(fields.object_key.len()))
        .ok_or(EncodeError::TooLarge)?;
    for grant in fields.grants {
        if !valid_name(grant.service, MAX_SERVICE_NAME_LEN) || !valid_rights(grant.rights) {
            return Err(EncodeError::InvalidGrant);
        }
        len = len.checked_add(4 + grant.service.len()).ok_or(EncodeError::TooLarge)?;
    }
    if len > MAX_DESCRIPTOR_LEN {
        return Err(EncodeError::TooLarge);
    }
    Ok(len)
}

/// Encode a canonical descriptor with an all-zero signature field. The
/// caller signs [`signature_digest`] and writes the result with
/// [`set_signature`].
pub fn encode_unsigned(
    fields: &DescriptorFields<'_>,
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
    bytes[24..32].copy_from_slice(&fields.node_key.to_le_bytes());
    bytes[32..64].copy_from_slice(&fields.artifact_digest);
    bytes[64..66].copy_from_slice(&(fields.artifact_name.len() as u16).to_le_bytes());
    bytes[66..68].copy_from_slice(&(fields.object_key.len() as u16).to_le_bytes());
    bytes[68..70].copy_from_slice(&(fields.grants.len() as u16).to_le_bytes());
    bytes[70..72].copy_from_slice(&fields.stack_pages_per_thread.to_le_bytes());
    bytes[72..88].copy_from_slice(&sha256::digest(public_key)[..KEY_ID_LEN]);
    bytes[MAX_THREADS_OFFSET..MAX_THREADS_OFFSET + 2]
        .copy_from_slice(&fields.max_threads.to_le_bytes());
    bytes[SHUTDOWN_GRACE_MS_OFFSET..SHUTDOWN_GRACE_MS_OFFSET + 4]
        .copy_from_slice(&fields.shutdown_grace_ms.to_le_bytes());

    let mut offset = HEADER_LEN;
    let name_end = offset + fields.artifact_name.len();
    bytes[offset..name_end].copy_from_slice(fields.artifact_name);
    offset = name_end;
    let key_end = offset + fields.object_key.len();
    bytes[offset..key_end].copy_from_slice(fields.object_key);
    offset = key_end;
    for grant in fields.grants {
        bytes[offset..offset + 2].copy_from_slice(&grant.rights.to_le_bytes());
        bytes[offset + 2..offset + 4].copy_from_slice(&(grant.service.len() as u16).to_le_bytes());
        offset += 4;
        let end = offset + grant.service.len();
        bytes[offset..end].copy_from_slice(grant.service);
        offset = end;
    }
    debug_assert_eq!(offset, len);
    Ok(len)
}

pub fn signature_digest(bytes: &[u8]) -> Option<[u8; 32]> {
    let descriptor = decode(bytes)?;
    Some(sha256::digest_skipping(descriptor.bytes, SIGNATURE_OFFSET, SIGNATURE_LEN))
}

pub fn set_signature(bytes: &mut [u8], signature: &[u8; SIGNATURE_LEN]) -> bool {
    if decode(bytes).is_none() {
        return false;
    }
    bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN].copy_from_slice(signature);
    true
}

pub fn decode(bytes: &[u8]) -> Option<DeploymentDescriptor<'_>> {
    if bytes.len() < LEGACY_HEADER_LEN {
        return None;
    }
    let format_version = read_u16(bytes, 8)?;
    let (header_len, stack_pages_per_thread, max_threads, shutdown_grace_ms) =
        match (bytes.get(0..8)?, format_version) {
            (magic, VERSION) if magic == MAGIC => {
                if usize::from(read_u16(bytes, 10)?) != HEADER_LEN {
                    return None;
                }
                let pages = read_u16(bytes, 70)?;
                if !(1..=crate::MAX_USER_STACK_PAGES).contains(&usize::from(pages)) {
                    return None;
                }
                let max_threads = read_u16(bytes, MAX_THREADS_OFFSET)?;
                if !(1..=crate::MAX_USER_THREADS).contains(&usize::from(max_threads)) {
                    return None;
                }
                let grace = read_u32(bytes, SHUTDOWN_GRACE_MS_OFFSET)?;
                if grace > crate::MAX_SHUTDOWN_GRACE_MS {
                    return None;
                }
                (HEADER_LEN, pages, max_threads, grace)
            }
            (magic, V3_VERSION) if magic == V3_MAGIC => {
                let header_len = MAX_THREADS_OFFSET + 2;
                if usize::from(read_u16(bytes, 10)?) != header_len {
                    return None;
                }
                let pages = read_u16(bytes, 70)?;
                if !(1..=crate::MAX_USER_STACK_PAGES).contains(&usize::from(pages)) {
                    return None;
                }
                let max_threads = read_u16(bytes, MAX_THREADS_OFFSET)?;
                if !(1..=crate::MAX_USER_THREADS).contains(&usize::from(max_threads)) {
                    return None;
                }
                (header_len, pages, max_threads, crate::DEFAULT_SHUTDOWN_GRACE_MS)
            }
            (magic, V2_VERSION) if magic == V2_MAGIC => {
                if usize::from(read_u16(bytes, 10)?) != LEGACY_HEADER_LEN {
                    return None;
                }
                let pages = read_u16(bytes, 70)?;
                if !(1..=crate::MAX_USER_STACK_PAGES).contains(&usize::from(pages)) {
                    return None;
                }
                (
                    LEGACY_HEADER_LEN,
                    pages,
                    crate::DEFAULT_USER_MAX_THREADS.try_into().ok()?,
                    crate::DEFAULT_SHUTDOWN_GRACE_MS,
                )
            }
            (magic, LEGACY_VERSION) if magic == LEGACY_MAGIC => {
                if usize::from(read_u16(bytes, 10)?) != LEGACY_HEADER_LEN
                    || read_u16(bytes, 70)? != 0
                {
                    return None;
                }
                (
                    LEGACY_HEADER_LEN,
                    crate::DEFAULT_USER_STACK_PAGES.try_into().ok()?,
                    crate::DEFAULT_USER_MAX_THREADS.try_into().ok()?,
                    crate::DEFAULT_SHUTDOWN_GRACE_MS,
                )
            }
            _ => return None,
        };
    let total_len = usize::try_from(read_u32(bytes, 12)?).ok()?;
    if total_len != bytes.len() || total_len > MAX_DESCRIPTOR_LEN {
        return None;
    }
    let sequence = read_u64(bytes, 16)?;
    if sequence == 0 {
        return None;
    }
    let node_key = read_u64(bytes, 24)?;
    let artifact_digest = bytes.get(32..64)?.try_into().ok()?;
    let name_len = usize::from(read_u16(bytes, 64)?);
    let object_key_len = usize::from(read_u16(bytes, 66)?);
    let grant_count = usize::from(read_u16(bytes, 68)?);
    if grant_count > MAX_GRANTS {
        return None;
    }
    let name_start = header_len;
    let name_end = name_start.checked_add(name_len)?;
    let object_key_end = name_end.checked_add(object_key_len)?;
    let artifact_name = bytes.get(name_start..name_end)?;
    let object_key = bytes.get(name_end..object_key_end)?;
    if !valid_artifact_name(artifact_name) || !valid_name(object_key, MAX_OBJECT_KEY_LEN) {
        return None;
    }
    let mut grants = CapabilityGrants {
        bytes,
        offset: object_key_end,
        remaining: grant_count,
    };
    for _ in 0..grant_count {
        let grant = grants.next()?;
        if !valid_name(grant.service, MAX_SERVICE_NAME_LEN) || !valid_rights(grant.rights) {
            return None;
        }
    }
    if grants.offset != total_len || grants.remaining != 0 {
        return None;
    }
    Some(DeploymentDescriptor {
        bytes,
        format_version,
        sequence,
        node_key,
        artifact_digest,
        artifact_name,
        stack_pages_per_thread,
        max_threads,
        shutdown_grace_ms,
        object_key,
        grants_offset: object_key_end,
        grant_count,
    })
}

pub fn verify(bytes: &[u8], public_key_bytes: &[u8; 32]) -> VerifyOutcome {
    let Some(descriptor) = decode(bytes) else {
        return VerifyOutcome::Invalid;
    };
    if descriptor.bytes[72..88] != sha256::digest(public_key_bytes)[..KEY_ID_LEN] {
        return VerifyOutcome::WrongKey;
    }
    let Ok(public_key) = PublicKey::from_slice(public_key_bytes) else {
        return VerifyOutcome::Invalid;
    };
    let Ok(signature) = Signature::from_slice(
        &descriptor.bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN],
    ) else {
        return VerifyOutcome::Invalid;
    };
    let digest = sha256::digest_skipping(descriptor.bytes, SIGNATURE_OFFSET, SIGNATURE_LEN);
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
