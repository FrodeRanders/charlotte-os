//! Encrypted, operator-signed connector profiles for cluster provisioning.
//!
//! `COPSENC1` is deliberately separate from [`crate::deployment`] and
//! [`crate::release`]. Application artifacts describe logical capability
//! requirements; an organisational operator binds those names to concrete
//! infrastructure in this envelope. The profile is encrypted to a distinct
//! cluster recipient key using RFC 9180 HPKE and signed by a distinct Ed25519
//! operational authority. Neither key is the artifact-signing key.

use ed25519_compact::{
    PublicKey as SigningPublicKey,
    Signature,
};
use hpke::{
    Deserializable,
    Kem as KemTrait,
    OpModeR,
    OpModeS,
    Serializable,
    aead::{
        AeadTag,
        ChaCha20Poly1305,
    },
    inout::InOutBuf,
    kdf::HkdfSha256,
    kem::X25519HkdfSha256,
};
use rand_core::CryptoRng;

use crate::sha256;

type Kem = X25519HkdfSha256;
type Kdf = HkdfSha256;
type Aead = ChaCha20Poly1305;

pub const MAGIC: &[u8; 8] = b"COPSENC1";
pub const VERSION: u16 = 1;
pub const SUITE_HPKE_X25519_HKDF_SHA256_CHACHA20POLY1305: u16 = 1;
pub const HEADER_LEN: usize = 252;
pub const KEY_ID_LEN: usize = 16;
pub const ENCAPSULATED_KEY_LEN: usize = 32;
pub const AEAD_TAG_LEN: usize = 16;
pub const SIGNATURE_LEN: usize = 64;
pub const SIGNATURE_OFFSET: usize = 188;
pub const MAX_PROFILE_NAME_LEN: usize = 256;
pub const MAX_PROFILE_LEN: usize = 64 * 1024;
pub const MAX_ENVELOPE_LEN: usize = HEADER_LEN + MAX_PROFILE_NAME_LEN + MAX_PROFILE_LEN;

pub const PROFILE_KIND_S3: u16 = 1;
pub const PROFILE_KIND_KAFKA: u16 = 2;

const INFO: &[u8] = b"CharlotteOS COPSENC1 HPKE base mode";

const CLUSTER_ID_OFFSET: usize = 44;
const RELEASE_DIGEST_OFFSET: usize = 76;
const RECIPIENT_KEY_ID_OFFSET: usize = 108;
const SIGNING_KEY_ID_OFFSET: usize = 124;
const ENCAPSULATED_KEY_OFFSET: usize = 140;
const AEAD_TAG_OFFSET: usize = 172;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeFields<'a> {
    pub sequence: u64,
    pub expires_unix_seconds: u64,
    pub profile_kind: u16,
    pub cluster_id: [u8; 32],
    pub release_digest: [u8; 32],
    pub profile_name: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Envelope<'a> {
    bytes: &'a [u8],
    pub sequence: u64,
    pub expires_unix_seconds: u64,
    pub profile_kind: u16,
    pub cluster_id: [u8; 32],
    pub release_digest: [u8; 32],
    pub recipient_key_id: [u8; KEY_ID_LEN],
    pub signing_key_id: [u8; KEY_ID_LEN],
    pub profile_name: &'a [u8],
    pub ciphertext: &'a [u8],
}

impl<'a> Envelope<'a> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall,
    Encryption,
    InvalidContext,
    InvalidExpiry,
    InvalidKey,
    InvalidProfileKind,
    InvalidProfileName,
    InvalidSequence,
    ProfileTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyOutcome {
    Valid,
    Invalid,
    WrongKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenError {
    Authentication,
    Expired,
    Invalid,
    OutputTooSmall,
    WrongCluster,
    WrongRecipient,
    WrongRelease,
    WrongSigningKey,
}

pub fn valid_profile_name(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROFILE_NAME_LEN
        && value.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

pub fn valid_profile_kind(value: u16) -> bool {
    matches!(value, PROFILE_KIND_S3 | PROFILE_KIND_KAFKA)
}

fn key_id(key: &[u8]) -> [u8; KEY_ID_LEN] {
    sha256::digest(key)[..KEY_ID_LEN].try_into().expect("fixed key id length")
}

pub fn recipient_key_id(public_key: &[u8; 32]) -> [u8; KEY_ID_LEN] {
    key_id(public_key)
}

pub fn signing_key_id(public_key: &[u8; 32]) -> [u8; KEY_ID_LEN] {
    key_id(public_key)
}

pub fn encoded_len(fields: &EnvelopeFields<'_>, profile_len: usize) -> Result<usize, EncodeError> {
    if fields.sequence == 0 {
        return Err(EncodeError::InvalidSequence);
    }
    if fields.expires_unix_seconds == 0 {
        return Err(EncodeError::InvalidExpiry);
    }
    if !valid_profile_kind(fields.profile_kind) {
        return Err(EncodeError::InvalidProfileKind);
    }
    if fields.cluster_id.iter().all(|byte| *byte == 0)
        || fields.release_digest.iter().all(|byte| *byte == 0)
    {
        return Err(EncodeError::InvalidContext);
    }
    if !valid_profile_name(fields.profile_name) {
        return Err(EncodeError::InvalidProfileName);
    }
    if profile_len == 0 || profile_len > MAX_PROFILE_LEN {
        return Err(EncodeError::ProfileTooLarge);
    }
    HEADER_LEN
        .checked_add(fields.profile_name.len())
        .and_then(|len| len.checked_add(profile_len))
        .filter(|len| *len <= MAX_ENVELOPE_LEN)
        .ok_or(EncodeError::ProfileTooLarge)
}

fn context_digest(bytes: &[u8], profile_name: &[u8]) -> [u8; 32] {
    let mut hash = sha256::Sha256::new();
    hash.update(b"CharlotteOS COPSENC1 context\0");
    hash.update(&bytes[..ENCAPSULATED_KEY_OFFSET]);
    hash.update(profile_name);
    hash.finalize()
}

pub fn generate_recipient_keypair(rng: &mut impl CryptoRng) -> ([u8; 32], [u8; 32]) {
    let (private_key, public_key) = Kem::gen_keypair_with_rng(rng);
    let mut private_bytes = [0u8; 32];
    let mut public_bytes = [0u8; 32];
    private_key.write_exact(&mut private_bytes);
    public_key.write_exact(&mut public_bytes);
    (private_bytes, public_bytes)
}

pub fn recipient_public_key(private_key: &[u8; 32]) -> Result<[u8; 32], EncodeError> {
    let private_key = <Kem as KemTrait>::PrivateKey::from_bytes(private_key)
        .map_err(|_| EncodeError::InvalidKey)?;
    let mut public_key = [0u8; 32];
    Kem::sk_to_pk(&private_key).write_exact(&mut public_key);
    Ok(public_key)
}

/// Encode and encrypt an unsigned envelope. The caller must sign
/// [`signature_digest`] with the operational Ed25519 key and call
/// [`set_signature`] before publication.
pub fn seal_unsigned(
    fields: &EnvelopeFields<'_>,
    profile: &[u8],
    recipient_public_key: &[u8; 32],
    operational_signing_public_key: &[u8; 32],
    rng: &mut impl CryptoRng,
    output: &mut [u8],
) -> Result<usize, EncodeError> {
    let len = encoded_len(fields, profile.len())?;
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
    bytes[24..32].copy_from_slice(&fields.expires_unix_seconds.to_le_bytes());
    bytes[32..34].copy_from_slice(&fields.profile_kind.to_le_bytes());
    bytes[34..36].copy_from_slice(&(fields.profile_name.len() as u16).to_le_bytes());
    bytes[36..40].copy_from_slice(&(profile.len() as u32).to_le_bytes());
    bytes[40..42].copy_from_slice(&SUITE_HPKE_X25519_HKDF_SHA256_CHACHA20POLY1305.to_le_bytes());
    bytes[CLUSTER_ID_OFFSET..RELEASE_DIGEST_OFFSET].copy_from_slice(&fields.cluster_id);
    bytes[RELEASE_DIGEST_OFFSET..RECIPIENT_KEY_ID_OFFSET].copy_from_slice(&fields.release_digest);
    bytes[RECIPIENT_KEY_ID_OFFSET..SIGNING_KEY_ID_OFFSET]
        .copy_from_slice(&recipient_key_id(recipient_public_key));
    bytes[SIGNING_KEY_ID_OFFSET..ENCAPSULATED_KEY_OFFSET]
        .copy_from_slice(&signing_key_id(operational_signing_public_key));

    let name_start = HEADER_LEN;
    let name_end = name_start + fields.profile_name.len();
    bytes[name_start..name_end].copy_from_slice(fields.profile_name);
    bytes[name_end..len].copy_from_slice(profile);

    let public_key = <Kem as KemTrait>::PublicKey::from_bytes(recipient_public_key)
        .map_err(|_| EncodeError::InvalidKey)?;
    let (encapsulated, mut sender) =
        hpke::setup_sender_with_rng::<Aead, Kdf, Kem>(&OpModeS::Base, &public_key, INFO, rng)
            .map_err(|_| EncodeError::Encryption)?;
    encapsulated.write_exact(&mut bytes[ENCAPSULATED_KEY_OFFSET..AEAD_TAG_OFFSET]);
    let context = context_digest(bytes, fields.profile_name);
    let tag = sender
        .seal_inout_detached(InOutBuf::from(&mut bytes[name_end..len]), &context)
        .map_err(|_| EncodeError::Encryption)?;
    tag.write_exact(&mut bytes[AEAD_TAG_OFFSET..SIGNATURE_OFFSET]);
    Ok(len)
}

pub fn decode(bytes: &[u8]) -> Option<Envelope<'_>> {
    if bytes.len() < HEADER_LEN
        || bytes.get(0..8)? != MAGIC
        || read_u16(bytes, 8)? != VERSION
        || usize::from(read_u16(bytes, 10)?) != HEADER_LEN
        || read_u16(bytes, 40)? != SUITE_HPKE_X25519_HKDF_SHA256_CHACHA20POLY1305
        || read_u16(bytes, 42)? != 0
    {
        return None;
    }
    let total_len = usize::try_from(read_u32(bytes, 12)?).ok()?;
    let sequence = read_u64(bytes, 16)?;
    let expires_unix_seconds = read_u64(bytes, 24)?;
    let profile_kind = read_u16(bytes, 32)?;
    let name_len = usize::from(read_u16(bytes, 34)?);
    let ciphertext_len = usize::try_from(read_u32(bytes, 36)?).ok()?;
    let cluster_id: [u8; 32] =
        bytes.get(CLUSTER_ID_OFFSET..RELEASE_DIGEST_OFFSET)?.try_into().ok()?;
    let release_digest: [u8; 32] =
        bytes.get(RELEASE_DIGEST_OFFSET..RECIPIENT_KEY_ID_OFFSET)?.try_into().ok()?;
    if total_len != bytes.len()
        || total_len > MAX_ENVELOPE_LEN
        || sequence == 0
        || expires_unix_seconds == 0
        || !valid_profile_kind(profile_kind)
        || cluster_id.iter().all(|byte| *byte == 0)
        || release_digest.iter().all(|byte| *byte == 0)
        || ciphertext_len == 0
        || ciphertext_len > MAX_PROFILE_LEN
    {
        return None;
    }
    let name_end = HEADER_LEN.checked_add(name_len)?;
    let ciphertext_end = name_end.checked_add(ciphertext_len)?;
    if ciphertext_end != total_len {
        return None;
    }
    let profile_name = bytes.get(HEADER_LEN..name_end)?;
    if !valid_profile_name(profile_name) {
        return None;
    }
    Some(Envelope {
        bytes,
        sequence,
        expires_unix_seconds,
        profile_kind,
        cluster_id,
        release_digest,
        recipient_key_id: bytes
            .get(RECIPIENT_KEY_ID_OFFSET..SIGNING_KEY_ID_OFFSET)?
            .try_into()
            .ok()?,
        signing_key_id: bytes
            .get(SIGNING_KEY_ID_OFFSET..ENCAPSULATED_KEY_OFFSET)?
            .try_into()
            .ok()?,
        profile_name,
        ciphertext: bytes.get(name_end..ciphertext_end)?,
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

pub fn verify(bytes: &[u8], operational_public_key: &[u8; 32]) -> VerifyOutcome {
    let Some(envelope) = decode(bytes) else {
        return VerifyOutcome::Invalid;
    };
    if envelope.signing_key_id != signing_key_id(operational_public_key) {
        return VerifyOutcome::WrongKey;
    }
    let Ok(public_key) = SigningPublicKey::from_slice(operational_public_key) else {
        return VerifyOutcome::Invalid;
    };
    let Ok(signature) =
        Signature::from_slice(&envelope.bytes[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_LEN])
    else {
        return VerifyOutcome::Invalid;
    };
    let digest = sha256::digest_skipping(envelope.bytes, SIGNATURE_OFFSET, SIGNATURE_LEN);
    if public_key.verify(digest, &signature).is_ok() {
        VerifyOutcome::Valid
    } else {
        VerifyOutcome::Invalid
    }
}

/// Verify policy context and decrypt a connector profile into caller-owned
/// memory. The caller must zeroize that memory after transferring it into a
/// read-only connector launch profile.
pub fn open(
    bytes: &[u8],
    recipient_private_key: &[u8; 32],
    operational_public_key: &[u8; 32],
    expected_cluster_id: &[u8; 32],
    expected_release_digest: &[u8; 32],
    now_unix_seconds: u64,
    output: &mut [u8],
) -> Result<usize, OpenError> {
    match verify(bytes, operational_public_key) {
        VerifyOutcome::Valid => {}
        VerifyOutcome::WrongKey => return Err(OpenError::WrongSigningKey),
        VerifyOutcome::Invalid => return Err(OpenError::Invalid),
    }
    let envelope = decode(bytes).ok_or(OpenError::Invalid)?;
    if &envelope.cluster_id != expected_cluster_id {
        return Err(OpenError::WrongCluster);
    }
    if &envelope.release_digest != expected_release_digest {
        return Err(OpenError::WrongRelease);
    }
    if now_unix_seconds > envelope.expires_unix_seconds {
        return Err(OpenError::Expired);
    }
    if output.len() < envelope.ciphertext.len() {
        return Err(OpenError::OutputTooSmall);
    }
    let private_key = <Kem as KemTrait>::PrivateKey::from_bytes(recipient_private_key)
        .map_err(|_| OpenError::WrongRecipient)?;
    let mut public_key = [0u8; 32];
    Kem::sk_to_pk(&private_key).write_exact(&mut public_key);
    if envelope.recipient_key_id != key_id(&public_key) {
        return Err(OpenError::WrongRecipient);
    }
    let encapsulated = <Kem as KemTrait>::EncappedKey::from_bytes(
        bytes.get(ENCAPSULATED_KEY_OFFSET..AEAD_TAG_OFFSET).ok_or(OpenError::Invalid)?,
    )
    .map_err(|_| OpenError::Invalid)?;
    let tag = AeadTag::<Aead>::from_bytes(
        bytes.get(AEAD_TAG_OFFSET..SIGNATURE_OFFSET).ok_or(OpenError::Invalid)?,
    )
    .map_err(|_| OpenError::Invalid)?;
    let mut receiver =
        hpke::setup_receiver::<Aead, Kdf, Kem>(&OpModeR::Base, &private_key, &encapsulated, INFO)
            .map_err(|_| OpenError::Authentication)?;
    let context = context_digest(bytes, envelope.profile_name);
    let plaintext = &mut output[..envelope.ciphertext.len()];
    plaintext.copy_from_slice(envelope.ciphertext);
    if receiver.open_inout_detached(InOutBuf::from(&mut *plaintext), &context, &tag).is_err() {
        plaintext.fill(0);
        return Err(OpenError::Authentication);
    }
    Ok(plaintext.len())
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
