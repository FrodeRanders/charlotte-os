// Public GCM known-answer/tamper control shared by host tests and the signed
// opt-in EL0 security probe. This is not included in ordinary service code.
use aes_gcm::{
    AeadInPlace,
    KeyInit,
};

pub(crate) fn check() {
    type Cipher = <embedded_tls::Aes128GcmSha256 as embedded_tls::TlsCipherSuite>::Cipher;
    let cipher = Cipher::new(&[0u8; 16].into());
    let nonce = [0u8; 12].into();
    let mut block = [0u8; 16];
    let expected = [
        0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92, 0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2, 0xfe,
        0x78,
    ];
    let tag = cipher.encrypt_in_place_detached(&nonce, b"", &mut block).unwrap();
    assert_eq!(block, expected);
    assert_eq!(
        tag.as_slice(),
        [
            0xab, 0x6e, 0x47, 0xd4, 0x2c, 0xec, 0x13, 0xbd, 0xf5, 0x3a, 0x67, 0xb2, 0x12, 0x57,
            0xbd, 0xdf,
        ]
    );
    let mut invalid = tag;
    invalid[0] ^= 1;
    assert!(cipher.decrypt_in_place_detached(&nonce, b"", &mut block, &invalid).is_err());
    assert_eq!(block, expected);
    cipher.decrypt_in_place_detached(&nonce, b"", &mut block, &tag).unwrap();
    assert_eq!(block, [0; 16]);
}
