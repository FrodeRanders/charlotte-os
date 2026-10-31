// Public reference vectors through embedded-tls's actual selected hash/HMAC
// types. Shared by hosted tests and the signed native security probe.
use embedded_tls::{
    Aes128GcmSha256,
    TlsCipherSuite,
    crypto::{
        TlsHash,
        TlsHmac,
    },
};

pub(crate) fn check() {
    type Hash = <Aes128GcmSha256 as TlsCipherSuite>::Hash;
    type Mac = <Hash as TlsHash>::Hmac;
    let mut hash = Hash::new();
    hash.update(b"a");
    let mut fork = hash.clone();
    hash.update(b"bc");
    fork.update(b"bc");
    assert_eq!(hash.finalize(), charlotte_launch::sha256::TEST_VECTOR_ABC);
    assert_eq!(fork.finalize(), charlotte_launch::sha256::TEST_VECTOR_ABC);

    // Independently checked with Python hmac/hashlib. Short and partial input
    // covers the concrete adapter's incremental, clone and consuming paths.
    const TAG: [u8; 32] = [
        0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b, 0xf1,
        0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c, 0x2e, 0x32,
        0xcf, 0xf7,
    ];
    for split in 0..=8 {
        let mut mac = Mac::new(&[0x0b; 20]);
        mac.update(&b"Hi There"[..split]);
        let mut fork = mac.clone();
        fork.update(&b"Hi There"[split..]);
        assert_eq!(fork.finalize(), TAG);
        mac.update(&b"Hi There"[split..]);
        assert!(TlsHmac::verify(mac, &TAG));
    }
    let mut mac = Mac::new(&[0x0b; 20]);
    mac.update(b"Hi There");
    let mut tampered = TAG;
    tampered[0] ^= 1;
    assert!(!TlsHmac::verify(mac, &tampered));
}

#[cfg(test)]
#[test]
fn selected_tls_hash_and_hmac_match_reference_through_clones() {
    check();
}
