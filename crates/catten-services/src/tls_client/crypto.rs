//! Registered TLS backend and typed mTLS key preparation.

use aes_gcm::{
    AeadInPlace,
    KeyInit,
};
use embassy_crypto::{
    Error,
    driver::InOutBuf,
};
use embassy_crypto_rustcrypto as _;

/// Preserve Charlotte's P-256 SEC1 validation before constructing an Embassy key.
pub(super) fn load_client_key(
    der: &[u8],
) -> Result<embedded_tls::PrivateKey, embedded_tls::TlsError> {
    let typed = p256::SecretKey::from_sec1_der(der)
        .map_err(|_| embedded_tls::TlsError::InvalidPrivateKey)?;
    let scalar = zeroize::Zeroizing::new(typed.to_bytes());
    embassy_crypto::p256::SigningKey::from_bytes((&*scalar).into())
        .map(embedded_tls::PrivateKey::EcdsaP256)
        .map_err(|_| embedded_tls::TlsError::InvalidPrivateKey)
}

// One explicit backend owns the cipher; Embassy's opaque Drop forwards here.
struct ErasingGcm;
impl embassy_crypto::driver::Aes128Gcm for ErasingGcm {
    type Context = aes_gcm::Aes128Gcm;

    fn init(key: &[u8; 16]) -> Self::Context {
        Self::Context::new(key.into())
    }

    fn encrypt(
        ctx: &Self::Context,
        nonce: &[u8; 12],
        aad: &[u8],
        buffer: InOutBuf<'_, '_, u8>,
        tag: &mut [u8; 16],
    ) -> Result<(), Error> {
        let result = ctx
            .encrypt_in_place_detached(nonce.into(), aad, buffer.into_out_with_copied_in())
            .map_err(|_| Error::InvalidInput)?;
        tag.copy_from_slice(&result);
        Ok(())
    }

    fn decrypt(
        ctx: &Self::Context,
        nonce: &[u8; 12],
        aad: &[u8],
        buffer: InOutBuf<'_, '_, u8>,
        tag: &[u8; 16],
    ) -> Result<(), Error> {
        ctx.decrypt_in_place_detached(
            nonce.into(),
            aad,
            buffer.into_out_with_copied_in(),
            tag.into(),
        )
        .map_err(|_| Error::InvalidSignature)
    }
}
embassy_crypto::aes128_gcm_impl!(ErasingGcm);

#[cfg(test)]
mod tests {
    use embassy_crypto::driver::Aes128Gcm as _;
    use embedded_tls::Aes128GcmSha256;

    use super::*;
    #[test]
    fn driver_supports_separate_input_and_output() {
        let ctx = ErasingGcm::init(&[0; 16]);
        let input = [0; 16];
        let mut ciphertext = [0xff; 16];
        let mut tag = [0; 16];
        ErasingGcm::encrypt(
            &ctx,
            &[0; 12],
            b"aad",
            InOutBuf::new(&input, &mut ciphertext).unwrap(),
            &mut tag,
        )
        .unwrap();
        let mut output = [0xff; 16];
        ErasingGcm::decrypt(
            &ctx,
            &[0; 12],
            b"aad",
            InOutBuf::new(&ciphertext, &mut output).unwrap(),
            &tag,
        )
        .unwrap();
        assert_eq!(output, input);
        assert_eq!(input, [0; 16]);
    }

    #[test]
    fn typed_sec1_parser_preserves_curve_oid_validation() {
        // Public fixture scalar 1 with explicit prime256v1 parameters.
        let mut der = [0; 51];
        der[..7].copy_from_slice(&[0x30, 0x31, 0x02, 0x01, 0x01, 0x04, 0x20]);
        der[38] = 1;
        der[39..].copy_from_slice(&[
            0xa0, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
        ]);
        assert!(matches!(load_client_key(&der).unwrap(), embedded_tls::PrivateKey::EcdsaP256(_)));
        assert!(embedded_tls::PrivateKey::from_sec1_der(&der).is_ok());
        der[50] = 8; // Different OID, identical scalar length.
        assert!(load_client_key(&der).is_err());
        // This control documents why the wrapper must retain the typed parser.
        assert!(embedded_tls::PrivateKey::from_sec1_der(&der).is_ok());
    }
    #[test]
    fn selected_tls_cipher_known_answer_and_tamper_rejection() {
        type Cipher = <Aes128GcmSha256 as embedded_tls::TlsCipherSuite>::Cipher;
        let cipher = Cipher::new(&[0; 16]);
        let mut block = [0; 16];
        let expected = [
            0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92, 0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2,
            0xfe, 0x78,
        ];
        let tag = cipher.encrypt(&[0; 12], b"", &mut block).unwrap();
        assert_eq!(block, expected);
        assert_eq!(
            tag,
            [
                0xab, 0x6e, 0x47, 0xd4, 0x2c, 0xec, 0x13, 0xbd, 0xf5, 0x3a, 0x67, 0xb2, 0x12, 0x57,
                0xbd, 0xdf
            ]
        );
        let mut bad = tag;
        bad[0] ^= 1;
        assert!(cipher.decrypt(&[0; 12], b"", &mut block, &bad).is_err());
        assert_eq!(block, expected);
        cipher.decrypt(&[0; 12], b"", &mut block, &tag).unwrap();
        assert_eq!(block, [0; 16]);
    }
    #[test]
    fn p256_ecdh_agrees_and_rejects_invalid_peer() {
        let _serial = super::super::tests::host_crypto();
        use embassy_crypto::p256::{
            PublicKey,
            SecretKey,
        };
        let a = SecretKey::generate().unwrap();
        let b = SecretKey::generate().unwrap();
        let public_a = a.public_key().unwrap();
        let public_b = b.public_key().unwrap();
        assert_eq!(
            a.diffie_hellman(&public_b).unwrap().as_bytes(),
            b.diffie_hellman(&public_a).unwrap().as_bytes()
        );
        let invalid = PublicKey::from_xy(&[0; 32], &[0; 32]);
        assert!(a.diffie_hellman(&invalid).is_err());
    }
    #[test]
    fn p256_signs_and_rejects_changed_digest() {
        let _serial = super::super::tests::host_crypto();
        let key = embassy_crypto::p256::SigningKey::generate().unwrap();
        let digest = embassy_crypto::Sha256::digest(b"public migration fixture");
        let signature = key.sign_prehash(&digest).unwrap();
        let public = key.verifying_key().unwrap();
        public.verify_prehash(&digest, &signature).unwrap();
        let mut bad = digest;
        bad[0] ^= 1;
        assert!(public.verify_prehash(&bad, &signature).is_err());
    }
    #[test]
    fn p384_signs_and_rejects_changed_digest() {
        let _serial = super::super::tests::host_crypto();
        let key = embassy_crypto::p384::SigningKey::generate().unwrap();
        let digest = embassy_crypto::Sha384::digest(b"public migration fixture");
        let signature = key.sign_prehash(&digest).unwrap();
        let public = key.verifying_key().unwrap();
        public.verify_prehash(&digest, &signature).unwrap();
        let mut bad = digest;
        bad[0] ^= 1;
        assert!(public.verify_prehash(&bad, &signature).is_err());
    }
}
