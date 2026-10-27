//! Rustls provider over Charlotte's existing portable crypto operations.
//!
//! Only TLS 1.3 AES-128-GCM/SHA-256, P-256 ECDH and P-256/SHA-256 ECDSA.
//! The embedding domain supplies the pinned Embassy SHA/HMAC/P-256/RNG drivers.
//! RNG callbacks are infallible: the domain's existing entropy owner must abort
//! on unavailable entropy, never invent bytes or replay accepted work.

use alloc::{
    boxed::Box,
    sync::Arc,
    vec,
    vec::Vec,
};
use core::fmt;

use aes_gcm::{
    AeadInPlace,
    KeyInit,
};
use embassy_crypto::{
    HmacSha256,
    Sha256,
    p256 as ec,
};
use p256::pkcs8::{
    DecodePrivateKey,
    EncodePublicKey,
};
use rustls::{
    CipherSuite,
    CipherSuiteCommon,
    ConnectionTrafficSecrets,
    ContentType,
    Error,
    NamedGroup,
    ProtocolVersion,
    SignatureAlgorithm,
    SignatureScheme,
    SupportedCipherSuite,
    Tls13CipherSuite,
    crypto::{
        self,
        ActiveKeyExchange,
        CryptoProvider,
        KeyProvider,
        SecureRandom,
        SharedSecret,
        SupportedKxGroup,
        WebPkiSupportedAlgorithms,
        cipher::{
            AeadKey,
            InboundOpaqueMessage,
            InboundPlainMessage,
            Iv,
            MessageDecrypter,
            MessageEncrypter,
            OutboundOpaqueMessage,
            OutboundPlainMessage,
            PrefixedPayload,
            Tls13AeadAlgorithm,
            UnsupportedOperationError,
            make_tls13_aad,
        },
    },
    pki_types::{
        AlgorithmIdentifier,
        InvalidSignature,
        PrivateKeyDer,
        SignatureVerificationAlgorithm,
        SubjectPublicKeyInfoDer,
        alg_id,
    },
    sign::{
        Signer,
        SigningKey,
    },
};
use zeroize::Zeroizing;

// Match the already-qualified TLS client's destructor selection. ring cannot
// run on Charlotte's x86 soft-float ABI, so it is a host-test dependency only.
#[cfg(all(target_os = "none", target_arch = "x86_64", not(polyval_force_soft)))]
compile_error!("management TLS requires the erasing software POLYVAL backend");

#[cfg(any(target_arch = "aarch64", all(target_arch = "x86_64", target_os = "none")))]
const _: () = {
    assert!(core::mem::needs_drop::<polyval::Polyval>());
    assert!(core::mem::needs_drop::<ghash::GHash>());
};

const _: fn() = || {
    fn erases<T: zeroize::ZeroizeOnDrop>() {}
    erases::<aes::Aes128>();
    erases::<aes::Aes128Enc>();
    erases::<aes::Aes128Dec>();
};

pub(super) fn provider() -> CryptoProvider {
    CryptoProvider {
        cipher_suites: vec![SupportedCipherSuite::Tls13(&Tls13CipherSuite {
            common: CipherSuiteCommon {
                suite: CipherSuite::TLS13_AES_128_GCM_SHA256,
                hash_provider: &Hash,
                // RFC 8446 section 5.5 / Rustls's AES-128-GCM suite policy.
                confidentiality_limit: 1 << 24,
            },
            hkdf_provider: &crypto::tls13::HkdfUsingHmac(&Hmac),
            aead_alg: &Aead,
            quic: None,
        })],
        kx_groups: vec![&P256],
        signature_verification_algorithms: WebPkiSupportedAlgorithms {
            all: &[&VerifyP256],
            mapping: &[(SignatureScheme::ECDSA_NISTP256_SHA256, &[&VerifyP256])],
        },
        secure_random: &Random,
        key_provider: &Keys,
    }
}

#[derive(Debug)]
struct Random;
impl SecureRandom for Random {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), rustls::crypto::GetRandomFailed> {
        embassy_crypto::rng_fill_bytes(bytes);
        Ok(())
    }
}

struct Hash;
impl crypto::hash::Hash for Hash {
    fn start(&self) -> Box<dyn crypto::hash::Context> {
        Box::new(HashContext(Sha256::new()))
    }

    fn hash(&self, bytes: &[u8]) -> crypto::hash::Output {
        crypto::hash::Output::new(&Sha256::digest(bytes))
    }

    fn output_len(&self) -> usize {
        32
    }

    fn algorithm(&self) -> rustls::crypto::hash::HashAlgorithm {
        rustls::crypto::hash::HashAlgorithm::SHA256
    }
}
struct HashContext(Sha256);
impl crypto::hash::Context for HashContext {
    fn fork_finish(&self) -> crypto::hash::Output {
        crypto::hash::Output::new(&self.0.clone().finalize())
    }

    fn fork(&self) -> Box<dyn crypto::hash::Context> {
        Box::new(Self(self.0.clone()))
    }

    fn finish(self: Box<Self>) -> crypto::hash::Output {
        crypto::hash::Output::new(&self.0.finalize())
    }

    fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

struct Hmac;
impl crypto::hmac::Hmac for Hmac {
    fn with_key(&self, key: &[u8]) -> Box<dyn crypto::hmac::Key> {
        Box::new(HmacKey(HmacSha256::new(key)))
    }

    fn hash_output_len(&self) -> usize {
        32
    }
}
struct HmacKey(HmacSha256);
impl crypto::hmac::Key for HmacKey {
    fn sign_concat(&self, first: &[u8], middle: &[&[u8]], last: &[u8]) -> crypto::hmac::Tag {
        let mut context = self.0.clone();
        context.update(first);
        for bytes in middle {
            context.update(bytes);
        }
        context.update(last);
        let tag = Zeroizing::new(context.finalize());
        crypto::hmac::Tag::new(&*tag)
    }

    fn tag_len(&self) -> usize {
        32
    }
}

#[derive(Debug)]
struct P256;
impl SupportedKxGroup for P256 {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let key = ec::SecretKey::generate().map_err(|_| Error::FailedToGetRandomBytes)?;
        let public = key.public_key().map_err(|_| Error::FailedToGetRandomBytes)?.to_sec1();
        Ok(Box::new(Exchange {
            key,
            public,
        }))
    }

    fn name(&self) -> NamedGroup {
        NamedGroup::secp256r1
    }
}
struct Exchange {
    key: ec::SecretKey,
    public: [u8; 65],
}
impl ActiveKeyExchange for Exchange {
    fn complete(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
        let peer: &[u8; 65] = peer
            .try_into()
            .map_err(|_| Error::PeerMisbehaved(rustls::PeerMisbehaved::InvalidKeyShare))?;
        if peer[0] != 4 {
            return Err(Error::PeerMisbehaved(rustls::PeerMisbehaved::InvalidKeyShare));
        }
        let peer = ec::PublicKey::from_sec1(peer)
            .map_err(|_| Error::PeerMisbehaved(rustls::PeerMisbehaved::InvalidKeyShare))?;
        let secret = self
            .key
            .diffie_hellman(&peer)
            .map_err(|_| Error::PeerMisbehaved(rustls::PeerMisbehaved::InvalidKeyShare))?;
        Ok(SharedSecret::from(&secret.as_bytes()[..]))
    }

    fn pub_key(&self) -> &[u8] {
        &self.public
    }

    fn group(&self) -> NamedGroup {
        NamedGroup::secp256r1
    }
}

#[derive(Debug)]
struct Keys;
impl KeyProvider for Keys {
    fn load_private_key(&self, der: PrivateKeyDer<'static>) -> Result<Arc<dyn SigningKey>, Error> {
        // Keep the typed curve/OID checks used by Charlotte's client. The
        // generic Embassy SEC1 parser alone accepts mismatched curve OIDs.
        let key = match &der {
            PrivateKeyDer::Pkcs8(bytes) => {
                p256::SecretKey::from_pkcs8_der(bytes.secret_pkcs8_der()).map_err(|_| ())
            }
            PrivateKeyDer::Sec1(bytes) => {
                p256::SecretKey::from_sec1_der(bytes.secret_sec1_der()).map_err(|_| ())
            }
            _ => return Err(Error::General("management requires a P-256 key".into())),
        }
        .map_err(|_| Error::General("invalid management P-256 key".into()))?;
        let scalar = Zeroizing::new(key.to_bytes());
        let signing = ec::SigningKey::from_bytes((&*scalar).into())
            .map_err(|_| Error::General("invalid management scalar".into()))?;
        let public = key
            .public_key()
            .to_public_key_der()
            .map_err(|_| Error::General("invalid management public key".into()))?
            .as_bytes()
            .to_vec();
        Ok(Arc::new(Key {
            signing,
            public,
        }))
    }
}

struct Key {
    signing: ec::SigningKey,
    public: Vec<u8>,
}
impl fmt::Debug for Key {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("ManagementP256Key")
    }
}
impl SigningKey for Key {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered
            .contains(&SignatureScheme::ECDSA_NISTP256_SHA256)
            .then(|| Box::new(P256Signer(self.signing.clone())) as Box<dyn Signer>)
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ECDSA
    }

    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        Some(self.public.as_slice().into())
    }
}
struct P256Signer(ec::SigningKey);
impl fmt::Debug for P256Signer {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("ManagementP256Signer")
    }
}
impl Signer for P256Signer {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, Error> {
        let signature = self
            .0
            .sign_prehash(&Sha256::digest(message))
            .map_err(|_| Error::General("management signing failed".into()))?;
        let signature = p256::ecdsa::Signature::from_slice(&signature.to_bytes())
            .map_err(|_| Error::General("management signature encoding failed".into()))?;
        Ok(signature.to_der().as_bytes().to_vec())
    }

    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ECDSA_NISTP256_SHA256
    }
}

#[derive(Debug)]
struct VerifyP256;
impl SignatureVerificationAlgorithm for VerifyP256 {
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_P256
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::ECDSA_SHA256
    }

    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        let public_key: &[u8; 65] = public_key.try_into().map_err(|_| InvalidSignature)?;
        let key = ec::VerifyingKey::from_sec1(public_key).map_err(|_| InvalidSignature)?;
        let signature =
            p256::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
        let signature = ec::Signature::from_bytes((&signature.to_bytes()).into())
            .map_err(|_| InvalidSignature)?;
        key.verify_prehash(&Sha256::digest(message), &signature).map_err(|_| InvalidSignature)
    }
}

struct Aead;
impl Tls13AeadAlgorithm for Aead {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        Box::new(RecordCipher::new(key, iv))
    }

    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        Box::new(RecordCipher::new(key, iv))
    }

    fn key_len(&self) -> usize {
        16
    }

    fn extract_keys(
        &self,
        _: AeadKey,
        _: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Err(UnsupportedOperationError)
    }
}
struct RecordCipher {
    cipher: aes_gcm::Aes128Gcm,
    iv: Zeroizing<[u8; 12]>,
}
impl RecordCipher {
    fn new(key: AeadKey, iv: Iv) -> Self {
        let mut nonce = Zeroizing::new([0; 12]);
        nonce.copy_from_slice(iv.as_ref());
        Self {
            cipher: aes_gcm::Aes128Gcm::new_from_slice(key.as_ref())
                .expect("Rustls supplies AES-128 key"),
            iv: nonce,
        }
    }

    fn nonce(&self, sequence: u64) -> Zeroizing<[u8; 12]> {
        let mut nonce = Zeroizing::new(*self.iv);
        for (target, counter) in nonce[4..].iter_mut().zip(sequence.to_be_bytes()) {
            *target ^= counter;
        }
        nonce
    }
}
impl MessageEncrypter for RecordCipher {
    fn encrypt(
        &mut self,
        message: OutboundPlainMessage<'_>,
        sequence: u64,
    ) -> Result<OutboundOpaqueMessage, Error> {
        let length = self.encrypted_payload_len(message.payload.len());
        let mut payload = PrefixedPayload::with_capacity(length);
        payload.extend_from_chunks(&message.payload);
        payload.extend_from_slice(&[u8::from(message.typ)]);
        let nonce = self.nonce(sequence);
        let tag = self
            .cipher
            .encrypt_in_place_detached((&*nonce).into(), &make_tls13_aad(length), payload.as_mut())
            .map_err(|_| Error::EncryptError)?;
        payload.extend_from_slice(&tag);
        Ok(OutboundOpaqueMessage::new(
            ContentType::ApplicationData,
            ProtocolVersion::TLSv1_2,
            payload,
        ))
    }

    fn encrypted_payload_len(&self, length: usize) -> usize {
        length + 1 + 16
    }
}
impl MessageDecrypter for RecordCipher {
    fn decrypt<'a>(
        &mut self,
        mut message: InboundOpaqueMessage<'a>,
        sequence: u64,
    ) -> Result<InboundPlainMessage<'a>, Error> {
        let length = message.payload.len();
        if length < 17 {
            return Err(Error::DecryptError);
        }
        let nonce = self.nonce(sequence);
        let (ciphertext, tag) = message.payload.split_at_mut(length - 16);
        self.cipher
            .decrypt_in_place_detached(
                (&*nonce).into(),
                &make_tls13_aad(length),
                ciphertext,
                (&*tag).into(),
            )
            .map_err(|_| Error::DecryptError)?;
        message.payload.truncate(length - 16);
        message.into_tls13_unpadded_message()
    }
}
