//! Signed HPKE boot resources bound to the exact installed admission policy.
//! No UTC lifetime is invented at boot: credential validity is checked by TLS.
//! Rotation/revocation requires a new installed policy revision and restart;
//! protected installation/antirollback/custody remain separate SEC-04 gates.

use alloc::vec::Vec;

use ed25519_compact::{
    PublicKey,
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
use zeroize::{
    Zeroize,
    Zeroizing,
};

use super::Profile;
use crate::{
    sha256,
    trust::AdmissionTrust,
};

type Kem = X25519HkdfSha256;
type Aead = ChaCha20Poly1305;
type Kdf = HkdfSha256;

pub const MAGIC: &[u8; 8] = b"CMGTENC1";
pub const HEADER_LEN: usize = 176;
pub const MAX_RESOURCE_BYTES: usize = HEADER_LEN + super::MAX_PROFILE_BYTES;
const VERSION: u16 = 1;
const SUITE: u16 = 1; // RFC 9180 X25519 / HKDF-SHA256 / ChaCha20Poly1305.
const TAG_OFFSET: usize = 96;
const SIGNATURE_OFFSET: usize = 112;
const INFO: &[u8] = b"CharlotteOS management boot resource HPKE v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Malformed,
    WrongPolicy,
    WrongRecipient,
    Signature,
    Encryption,
    Authentication,
    Profile(super::Error),
    Capacity,
    Allocation,
}

/// This receipt is produced only by signature/context verification and HPKE
/// authentication. Its secret bytes remain in one erasing owner through launch.
/// It grants no authority to replace the installed kernel boot policy.
pub struct PreparedProfile(Zeroizing<Vec<u8>>);
impl PreparedProfile {
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Binds the complete canonical public policy, including cluster, revision and
/// all four role keys. A resource never chooses a root, role key or recipient.
pub fn policy_digest(trust: &AdmissionTrust) -> Result<[u8; 32], Error> {
    let encoded = trust.encode().ok_or(Error::WrongPolicy)?;
    let mut hash = sha256::Sha256::new();
    hash.update(b"CharlotteOS management boot policy v1\0");
    hash.update(&encoded);
    Ok(hash.finalize())
}

fn decode(bytes: &[u8]) -> Result<&[u8], Error> {
    if !(HEADER_LEN + 1..=MAX_RESOURCE_BYTES).contains(&bytes.len())
        || bytes.get(..8) != Some(MAGIC)
        || bytes[8..10] != VERSION.to_le_bytes()
        || bytes[10..12] != (HEADER_LEN as u16).to_le_bytes()
        || bytes[12..16] != (bytes.len() as u32).to_le_bytes()
        || bytes[80..84] != ((bytes.len() - HEADER_LEN) as u32).to_le_bytes()
        || bytes[84..86] != SUITE.to_le_bytes()
        || bytes[86..TAG_OFFSET].iter().any(|byte| *byte != 0)
    {
        return Err(Error::Malformed);
    }
    Ok(&bytes[HEADER_LEN..])
}

pub fn signature_digest(bytes: &[u8]) -> Result<[u8; 32], Error> {
    decode(bytes)?;
    let mut hash = sha256::Sha256::new();
    hash.update(b"CharlotteOS management boot resource signature v1\0");
    hash.update(&bytes[..SIGNATURE_OFFSET]);
    hash.update(&bytes[HEADER_LEN..]);
    Ok(hash.finalize())
}

pub fn set_signature(bytes: &mut [u8], signature: &[u8; 64]) -> Result<(), Error> {
    decode(bytes)?;
    bytes[SIGNATURE_OFFSET..HEADER_LEN].copy_from_slice(signature);
    Ok(())
}

/// The signing tool signs signature_digest with this policy's operations key
/// before publication. An unsigned resource cannot pass verify/open.
pub fn seal_unsigned(
    profile: &[u8],
    trust: &AdmissionTrust,
    rng: &mut impl CryptoRng,
    output: &mut [u8],
) -> Result<usize, Error> {
    Profile::decode(profile).map_err(Error::Profile)?;
    let digest = policy_digest(trust)?;
    let length = HEADER_LEN.checked_add(profile.len()).ok_or(Error::Capacity)?;
    if length > MAX_RESOURCE_BYTES || output.len() < length {
        return Err(Error::Capacity);
    }
    let public_key = <Kem as KemTrait>::PublicKey::from_bytes(&trust.recipient_key)
        .map_err(|_| Error::WrongRecipient)?;
    // Prepare cryptography before copying any plaintext into the output.
    let (encapsulated, mut sender) =
        hpke::setup_sender_with_rng::<Aead, Kdf, Kem>(&OpModeS::Base, &public_key, INFO, rng)
            .map_err(|_| Error::Encryption)?;
    let bytes = &mut output[..length];
    bytes.fill(0);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(length as u32).to_le_bytes());
    bytes[16..48].copy_from_slice(&digest);
    encapsulated.write_exact(&mut bytes[48..80]);
    bytes[80..84].copy_from_slice(&(profile.len() as u32).to_le_bytes());
    bytes[84..86].copy_from_slice(&SUITE.to_le_bytes());
    let context = sha256::digest(&bytes[..TAG_OFFSET]);
    bytes[HEADER_LEN..].copy_from_slice(profile);
    let tag = match sender.seal_inout_detached(InOutBuf::from(&mut bytes[HEADER_LEN..]), &context) {
        Ok(tag) => tag,
        Err(_) => {
            bytes.zeroize();
            return Err(Error::Encryption);
        }
    };
    tag.write_exact(&mut bytes[TAG_OFFSET..SIGNATURE_OFFSET]);
    Ok(length)
}

pub fn verify(bytes: &[u8], trust: &AdmissionTrust) -> Result<(), Error> {
    decode(bytes)?;
    if bytes[16..48] != policy_digest(trust)? {
        return Err(Error::WrongPolicy);
    }
    let public = PublicKey::from_slice(&trust.operations_key).map_err(|_| Error::Signature)?;
    let signature = Signature::from_slice(&bytes[SIGNATURE_OFFSET..HEADER_LEN])
        .map_err(|_| Error::Signature)?;
    public.verify(signature_digest(bytes)?, &signature).map_err(|_| Error::Signature)
}

pub fn open(
    bytes: &[u8],
    trust: &AdmissionTrust,
    recipient_private: &[u8; 32],
) -> Result<PreparedProfile, Error> {
    // No allocation or private-key operation precedes policy/signature checks.
    verify(bytes, trust)?;
    if crate::operations::recipient_public_key(recipient_private).ok() != Some(trust.recipient_key)
    {
        return Err(Error::WrongRecipient);
    }
    let private = <Kem as KemTrait>::PrivateKey::from_bytes(recipient_private)
        .map_err(|_| Error::WrongRecipient)?;
    let encapsulated =
        <Kem as KemTrait>::EncappedKey::from_bytes(&bytes[48..80]).map_err(|_| Error::Malformed)?;
    let tag = AeadTag::<Aead>::from_bytes(&bytes[TAG_OFFSET..SIGNATURE_OFFSET])
        .map_err(|_| Error::Malformed)?;
    let mut receiver =
        hpke::setup_receiver::<Aead, Kdf, Kem>(&OpModeR::Base, &private, &encapsulated, INFO)
            .map_err(|_| Error::Authentication)?;
    let ciphertext = &bytes[HEADER_LEN..];
    let mut plaintext = Zeroizing::new(Vec::new());
    plaintext.try_reserve_exact(ciphertext.len()).map_err(|_| Error::Allocation)?;
    plaintext.extend_from_slice(ciphertext);
    let context = sha256::digest(&bytes[..TAG_OFFSET]);
    receiver
        .open_inout_detached(InOutBuf::from(&mut plaintext[..]), &context, &tag)
        .map_err(|_| Error::Authentication)?;
    Profile::decode(&plaintext).map_err(Error::Profile)?;
    Ok(PreparedProfile(plaintext))
}
