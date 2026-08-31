//! Operator-signed admission bundles joining one exact release to encrypted
//! connector profiles.
//!
//! A [`crate::operations`] envelope binds the SHA-256 of a signed
//! [`crate::release`] envelope. Embedding the operational-envelope digest back
//! into that release would therefore create a digest cycle. `COPSBND2` is a
//! separate transport proof: it carries the immutable release, the encrypted
//! profiles used to validate admission, and signed mappings to connector
//! artifacts and central-object-store keys. Replicated state can retain only
//! the verified release and compact ciphertext references.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use crate::{
    deployment,
    operations,
    release,
    sha256,
};

pub const MAGIC: &[u8; 8] = b"COPSBND2";
pub const VERSION: u16 = 2;
pub const HEADER_LEN: usize = 192;
pub const MAX_BINDINGS: usize = 8;
pub const MAX_OBJECT_KEY_LEN: usize = deployment::MAX_OBJECT_KEY_LEN;
/// Leaves room below relmsg v3's 1 MiB message ceiling for the authenticated
/// follower-to-leader correlation header and transport tag.
pub const MAX_BUNDLE_LEN: usize = 1024 * 1024 - 512;
pub const SIGNATURE_OFFSET: usize = 128;
pub const SIGNATURE_LEN: usize = 64;

const SEQUENCE_OFFSET: usize = 24;
const CLUSTER_ID_OFFSET: usize = 32;
const RELEASE_DIGEST_OFFSET: usize = 64;
const RECIPIENT_KEY_ID_OFFSET: usize = 96;
const SIGNING_KEY_ID_OFFSET: usize = 112;
const BINDING_HEADER_LEN: usize = 108;
const BINDING_SIGNATURE_OFFSET: usize = 40;
pub const BINDING_SIGNATURE_LEN: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BindingFields<'a> {
    /// Signed artifact name of the connector that receives this profile.
    pub target_artifact: &'a [u8],
    /// Opaque key used to fetch the same encrypted envelope after admission.
    pub object_key: &'a [u8],
    /// Complete, already operator-signed `COPSENC1` envelope.
    pub envelope: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BundleFields<'a> {
    /// Monotonic revision of the complete operational set for this release.
    pub sequence: u64,
    pub cluster_id: [u8; 32],
    pub release: &'a [u8],
    pub bindings: &'a [BindingFields<'a>],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall,
    DuplicateBinding,
    InvalidBinding,
    InvalidCluster,
    InvalidRelease,
    InvalidSequence,
    TooLarge,
    TooManyBindings,
    WrongOperationalKey,
    WrongRecipient,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    Valid,
    Expired,
    Invalid,
    WrongCluster,
    WrongOperationalKey,
    WrongRecipient,
    WrongReleaseKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Binding<'a> {
    pub target_artifact: &'a [u8],
    pub object_key: &'a [u8],
    pub envelope: &'a [u8],
    pub envelope_digest: [u8; 32],
    /// Detached operational signature over the compact mapping. This proof is
    /// small enough to replicate and lets the final kernel launch gate verify
    /// it without receiving the full admission bundle.
    pub authorization_signature: [u8; BINDING_SIGNATURE_LEN],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bundle<'a> {
    bytes: &'a [u8],
    pub sequence: u64,
    pub cluster_id: [u8; 32],
    pub release_digest: [u8; 32],
    pub recipient_key_id: [u8; operations::KEY_ID_LEN],
    pub signing_key_id: [u8; operations::KEY_ID_LEN],
    pub release: &'a [u8],
    bindings_offset: usize,
    binding_count: usize,
}

impl<'a> Bundle<'a> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn bindings(&self) -> Bindings<'a> {
        Bindings {
            bytes: self.bytes,
            offset: self.bindings_offset,
            remaining: self.binding_count,
        }
    }
}

pub struct Bindings<'a> {
    bytes: &'a [u8],
    offset: usize,
    remaining: usize,
}

impl<'a> Iterator for Bindings<'a> {
    type Item = Binding<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let target_len = usize::from(read_u16(self.bytes, self.offset)?);
        let object_len = usize::from(read_u16(self.bytes, self.offset + 2)?);
        let envelope_len = usize::try_from(read_u32(self.bytes, self.offset + 4)?).ok()?;
        let envelope_digest = self.bytes.get(self.offset + 8..self.offset + 40)?.try_into().ok()?;
        let authorization_signature = self
            .bytes
            .get(
                self.offset + BINDING_SIGNATURE_OFFSET
                    ..self.offset + BINDING_SIGNATURE_OFFSET + BINDING_SIGNATURE_LEN,
            )?
            .try_into()
            .ok()?;
        if self
            .bytes
            .get(self.offset + 104..self.offset + BINDING_HEADER_LEN)?
            .iter()
            .any(|byte| *byte != 0)
        {
            self.remaining = 0;
            return None;
        }
        let target_start = self.offset.checked_add(BINDING_HEADER_LEN)?;
        let target_end = target_start.checked_add(target_len)?;
        let object_end = target_end.checked_add(object_len)?;
        let envelope_end = object_end.checked_add(envelope_len)?;
        let binding = Binding {
            target_artifact: self.bytes.get(target_start..target_end)?,
            object_key: self.bytes.get(target_end..object_end)?,
            envelope: self.bytes.get(object_end..envelope_end)?,
            envelope_digest,
            authorization_signature,
        };
        self.offset = envelope_end;
        self.remaining -= 1;
        Some(binding)
    }
}

pub fn valid_object_key(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OBJECT_KEY_LEN
        && value.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

fn release_contains_artifact(release: &release::ReleaseEnvelope<'_>, target: &[u8]) -> bool {
    release.descriptors().any(|bytes| {
        deployment::decode(bytes).is_some_and(|descriptor| descriptor.artifact_name == target)
    })
}

fn validate_fields(fields: &BundleFields<'_>) -> Result<usize, EncodeError> {
    if fields.sequence == 0 {
        return Err(EncodeError::InvalidSequence);
    }
    if fields.cluster_id.iter().all(|byte| *byte == 0) {
        return Err(EncodeError::InvalidCluster);
    }
    let release = release::decode(fields.release).ok_or(EncodeError::InvalidRelease)?;
    if fields.bindings.is_empty() || fields.bindings.len() > MAX_BINDINGS {
        return Err(EncodeError::TooManyBindings);
    }
    let release_digest = sha256::digest(fields.release);
    let mut len = HEADER_LEN.checked_add(fields.release.len()).ok_or(EncodeError::TooLarge)?;
    for (index, binding) in fields.bindings.iter().enumerate() {
        if !deployment::valid_artifact_name(binding.target_artifact)
            || !release_contains_artifact(&release, binding.target_artifact)
            || !valid_object_key(binding.object_key)
        {
            return Err(EncodeError::InvalidBinding);
        }
        let envelope = operations::decode(binding.envelope).ok_or(EncodeError::InvalidBinding)?;
        if envelope.cluster_id != fields.cluster_id || envelope.release_digest != release_digest {
            return Err(EncodeError::InvalidBinding);
        }
        for previous in &fields.bindings[..index] {
            let previous_envelope =
                operations::decode(previous.envelope).ok_or(EncodeError::InvalidBinding)?;
            if previous.target_artifact == binding.target_artifact
                || previous.object_key == binding.object_key
                || previous_envelope.profile_name == envelope.profile_name
            {
                return Err(EncodeError::DuplicateBinding);
            }
        }
        len = len
            .checked_add(BINDING_HEADER_LEN)
            .and_then(|len| len.checked_add(binding.target_artifact.len()))
            .and_then(|len| len.checked_add(binding.object_key.len()))
            .and_then(|len| len.checked_add(binding.envelope.len()))
            .ok_or(EncodeError::TooLarge)?;
    }
    if len > MAX_BUNDLE_LEN {
        return Err(EncodeError::TooLarge);
    }
    Ok(len)
}

pub fn encoded_len(fields: &BundleFields<'_>) -> Result<usize, EncodeError> {
    validate_fields(fields)
}

/// Encode an unsigned admission bundle after verifying every nested
/// operational envelope against the supplied operational and recipient keys.
pub fn encode_unsigned(
    fields: &BundleFields<'_>,
    operational_public_key: &[u8; 32],
    recipient_public_key: &[u8; 32],
    output: &mut [u8],
) -> Result<usize, EncodeError> {
    let len = validate_fields(fields)?;
    if output.len() < len {
        return Err(EncodeError::BufferTooSmall);
    }
    let signing_key_id = operations::signing_key_id(operational_public_key);
    let recipient_key_id = operations::recipient_key_id(recipient_public_key);
    for binding in fields.bindings {
        let envelope = operations::decode(binding.envelope).ok_or(EncodeError::InvalidBinding)?;
        if operations::verify(binding.envelope, operational_public_key)
            != operations::VerifyOutcome::Valid
            || envelope.signing_key_id != signing_key_id
        {
            return Err(EncodeError::WrongOperationalKey);
        }
        if envelope.recipient_key_id != recipient_key_id {
            return Err(EncodeError::WrongRecipient);
        }
    }

    let bytes = &mut output[..len];
    bytes.fill(0);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(len as u32).to_le_bytes());
    bytes[16..20].copy_from_slice(&(fields.release.len() as u32).to_le_bytes());
    bytes[20..22].copy_from_slice(&(fields.bindings.len() as u16).to_le_bytes());
    bytes[SEQUENCE_OFFSET..CLUSTER_ID_OFFSET].copy_from_slice(&fields.sequence.to_le_bytes());
    bytes[CLUSTER_ID_OFFSET..RELEASE_DIGEST_OFFSET].copy_from_slice(&fields.cluster_id);
    bytes[RELEASE_DIGEST_OFFSET..RECIPIENT_KEY_ID_OFFSET]
        .copy_from_slice(&sha256::digest(fields.release));
    bytes[RECIPIENT_KEY_ID_OFFSET..SIGNING_KEY_ID_OFFSET].copy_from_slice(&recipient_key_id);
    bytes[SIGNING_KEY_ID_OFFSET..SIGNATURE_OFFSET].copy_from_slice(&signing_key_id);

    let mut offset = HEADER_LEN;
    let release_end = offset + fields.release.len();
    bytes[offset..release_end].copy_from_slice(fields.release);
    offset = release_end;
    for binding in fields.bindings {
        bytes[offset..offset + 2]
            .copy_from_slice(&(binding.target_artifact.len() as u16).to_le_bytes());
        bytes[offset + 2..offset + 4]
            .copy_from_slice(&(binding.object_key.len() as u16).to_le_bytes());
        bytes[offset + 4..offset + 8]
            .copy_from_slice(&(binding.envelope.len() as u32).to_le_bytes());
        bytes[offset + 8..offset + 40].copy_from_slice(&sha256::digest(binding.envelope));
        offset += BINDING_HEADER_LEN;
        let target_end = offset + binding.target_artifact.len();
        bytes[offset..target_end].copy_from_slice(binding.target_artifact);
        offset = target_end;
        let object_end = offset + binding.object_key.len();
        bytes[offset..object_end].copy_from_slice(binding.object_key);
        offset = object_end;
        let envelope_end = offset + binding.envelope.len();
        bytes[offset..envelope_end].copy_from_slice(binding.envelope);
        offset = envelope_end;
    }
    debug_assert_eq!(offset, len);
    Ok(len)
}

pub fn decode(bytes: &[u8]) -> Option<Bundle<'_>> {
    if bytes.len() < HEADER_LEN
        || bytes.get(..8)? != MAGIC
        || read_u16(bytes, 8)? != VERSION
        || usize::from(read_u16(bytes, 10)?) != HEADER_LEN
        || read_u16(bytes, 22)? != 0
    {
        return None;
    }
    let total_len = usize::try_from(read_u32(bytes, 12)?).ok()?;
    let release_len = usize::try_from(read_u32(bytes, 16)?).ok()?;
    let binding_count = usize::from(read_u16(bytes, 20)?);
    let sequence = read_u64(bytes, SEQUENCE_OFFSET)?;
    if total_len != bytes.len()
        || total_len > MAX_BUNDLE_LEN
        || sequence == 0
        || binding_count == 0
        || binding_count > MAX_BINDINGS
    {
        return None;
    }
    let cluster_id: [u8; 32] =
        bytes.get(CLUSTER_ID_OFFSET..RELEASE_DIGEST_OFFSET)?.try_into().ok()?;
    if cluster_id.iter().all(|byte| *byte == 0) {
        return None;
    }
    let release_digest: [u8; 32] =
        bytes.get(RELEASE_DIGEST_OFFSET..RECIPIENT_KEY_ID_OFFSET)?.try_into().ok()?;
    let release_end = HEADER_LEN.checked_add(release_len)?;
    let release_bytes = bytes.get(HEADER_LEN..release_end)?;
    let release = release::decode(release_bytes)?;
    if sha256::digest(release_bytes) != release_digest {
        return None;
    }
    let recipient_key_id =
        bytes.get(RECIPIENT_KEY_ID_OFFSET..SIGNING_KEY_ID_OFFSET)?.try_into().ok()?;
    let signing_key_id = bytes.get(SIGNING_KEY_ID_OFFSET..SIGNATURE_OFFSET)?.try_into().ok()?;
    let mut bindings = Bindings {
        bytes,
        offset: release_end,
        remaining: binding_count,
    };
    let mut targets: [&[u8]; MAX_BINDINGS] = [&[]; MAX_BINDINGS];
    let mut objects: [&[u8]; MAX_BINDINGS] = [&[]; MAX_BINDINGS];
    let mut profiles: [&[u8]; MAX_BINDINGS] = [&[]; MAX_BINDINGS];
    for index in 0..binding_count {
        let binding = bindings.next()?;
        if !deployment::valid_artifact_name(binding.target_artifact)
            || !release_contains_artifact(&release, binding.target_artifact)
            || !valid_object_key(binding.object_key)
            || sha256::digest(binding.envelope) != binding.envelope_digest
        {
            return None;
        }
        let envelope = operations::decode(binding.envelope)?;
        if envelope.cluster_id != cluster_id
            || envelope.release_digest != release_digest
            || envelope.signing_key_id != signing_key_id
            || envelope.recipient_key_id != recipient_key_id
            || targets[..index].contains(&binding.target_artifact)
            || objects[..index].contains(&binding.object_key)
            || profiles[..index].contains(&envelope.profile_name)
        {
            return None;
        }
        targets[index] = binding.target_artifact;
        objects[index] = binding.object_key;
        profiles[index] = envelope.profile_name;
    }
    if bindings.offset != total_len || bindings.remaining != 0 {
        return None;
    }
    Some(Bundle {
        bytes,
        sequence,
        cluster_id,
        release_digest,
        recipient_key_id,
        signing_key_id,
        release: release_bytes,
        bindings_offset: release_end,
        binding_count,
    })
}

pub fn signature_digest(bytes: &[u8]) -> Option<[u8; 32]> {
    let bundle = decode(bytes)?;
    Some(sha256::digest_skipping(bundle.bytes, SIGNATURE_OFFSET, SIGNATURE_LEN))
}

#[allow(clippy::too_many_arguments)]
fn binding_authorization_digest(
    sequence: u64,
    cluster_id: &[u8; 32],
    release_digest: &[u8; 32],
    recipient_key_id: &[u8; operations::KEY_ID_LEN],
    signing_key_id: &[u8; operations::KEY_ID_LEN],
    target_artifact: &[u8],
    object_key: &[u8],
    envelope_digest: &[u8; 32],
) -> Option<[u8; 32]> {
    let target_len = u16::try_from(target_artifact.len()).ok()?;
    let object_len = u16::try_from(object_key.len()).ok()?;
    let mut hash = sha256::Sha256::new();
    hash.update(b"CharlotteOS COPSBND2 compact binding\0");
    hash.update(&sequence.to_le_bytes());
    hash.update(cluster_id);
    hash.update(release_digest);
    hash.update(recipient_key_id);
    hash.update(signing_key_id);
    hash.update(&target_len.to_le_bytes());
    hash.update(&object_len.to_le_bytes());
    hash.update(envelope_digest);
    hash.update(target_artifact);
    hash.update(object_key);
    Some(hash.finalize())
}

/// Digest signed by operations for one compact binding. It is independent of
/// the full bundle digest, allowing the proof to enter Raft without carrying
/// ciphertext or the rest of the admission transport.
pub fn binding_signature_digest(bytes: &[u8], index: usize) -> Option<[u8; 32]> {
    let bundle = decode(bytes)?;
    let binding = bundle.bindings().nth(index)?;
    binding_authorization_digest(
        bundle.sequence,
        &bundle.cluster_id,
        &bundle.release_digest,
        &bundle.recipient_key_id,
        &bundle.signing_key_id,
        binding.target_artifact,
        binding.object_key,
        &binding.envelope_digest,
    )
}

fn binding_offset(bytes: &[u8], index: usize) -> Option<usize> {
    let bundle = decode(bytes)?;
    if index >= bundle.binding_count {
        return None;
    }
    let mut offset = bundle.bindings_offset;
    for _ in 0..index {
        let target_len = usize::from(read_u16(bytes, offset)?);
        let object_len = usize::from(read_u16(bytes, offset + 2)?);
        let envelope_len = usize::try_from(read_u32(bytes, offset + 4)?).ok()?;
        offset = offset
            .checked_add(BINDING_HEADER_LEN)?
            .checked_add(target_len)?
            .checked_add(object_len)?
            .checked_add(envelope_len)?;
    }
    Some(offset)
}

pub fn set_binding_signature(
    bytes: &mut [u8],
    index: usize,
    signature: &[u8; BINDING_SIGNATURE_LEN],
) -> bool {
    let Some(offset) = binding_offset(bytes, index) else {
        return false;
    };
    bytes[offset + BINDING_SIGNATURE_OFFSET
        ..offset + BINDING_SIGNATURE_OFFSET + BINDING_SIGNATURE_LEN]
        .copy_from_slice(signature);
    true
}

#[allow(clippy::too_many_arguments)]
pub fn verify_binding_authorization(
    sequence: u64,
    cluster_id: &[u8; 32],
    release_digest: &[u8; 32],
    recipient_key_id: &[u8; operations::KEY_ID_LEN],
    signing_key_id: &[u8; operations::KEY_ID_LEN],
    target_artifact: &[u8],
    object_key: &[u8],
    envelope_digest: &[u8; 32],
    signature: &[u8; BINDING_SIGNATURE_LEN],
    operational_public_key: &[u8; 32],
) -> bool {
    if signing_key_id != &operations::signing_key_id(operational_public_key) {
        return false;
    }
    let Some(digest) = binding_authorization_digest(
        sequence,
        cluster_id,
        release_digest,
        recipient_key_id,
        signing_key_id,
        target_artifact,
        object_key,
        envelope_digest,
    ) else {
        return false;
    };
    let Ok(public_key) = PublicKey::from_slice(operational_public_key) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    public_key.verify(digest, &signature).is_ok()
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
    release_public_key: &[u8; 32],
    operational_public_key: &[u8; 32],
    recipient_public_key: &[u8; 32],
    expected_cluster_id: &[u8; 32],
    now_unix_seconds: u64,
) -> VerifyOutcome {
    let Some(bundle) = decode(bytes) else {
        return VerifyOutcome::Invalid;
    };
    if &bundle.cluster_id != expected_cluster_id {
        return VerifyOutcome::WrongCluster;
    }
    if bundle.signing_key_id != operations::signing_key_id(operational_public_key) {
        return VerifyOutcome::WrongOperationalKey;
    }
    if bundle.recipient_key_id != operations::recipient_key_id(recipient_public_key) {
        return VerifyOutcome::WrongRecipient;
    }
    if release::verify(bundle.release, release_public_key) != release::VerifyOutcome::Valid {
        return VerifyOutcome::WrongReleaseKey;
    }
    let Ok(public_key) = PublicKey::from_slice(operational_public_key) else {
        return VerifyOutcome::Invalid;
    };
    let Ok(signature) =
        Signature::from_slice(&bundle.bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN])
    else {
        return VerifyOutcome::Invalid;
    };
    let digest = sha256::digest_skipping(bundle.bytes, SIGNATURE_OFFSET, SIGNATURE_LEN);
    if public_key.verify(digest, &signature).is_err() {
        return VerifyOutcome::Invalid;
    }
    for binding in bundle.bindings() {
        if !verify_binding_authorization(
            bundle.sequence,
            &bundle.cluster_id,
            &bundle.release_digest,
            &bundle.recipient_key_id,
            &bundle.signing_key_id,
            binding.target_artifact,
            binding.object_key,
            &binding.envelope_digest,
            &binding.authorization_signature,
            operational_public_key,
        ) {
            return VerifyOutcome::Invalid;
        }
        if operations::verify(binding.envelope, operational_public_key)
            != operations::VerifyOutcome::Valid
        {
            return VerifyOutcome::WrongOperationalKey;
        }
        let Some(envelope) = operations::decode(binding.envelope) else {
            return VerifyOutcome::Invalid;
        };
        if envelope.expires_unix_seconds < now_unix_seconds {
            return VerifyOutcome::Expired;
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
