use charlotte_launch::{
    DEFAULT_USER_STACK_PAGES,
    MAX_USER_STACK_PAGES,
    deployment::{
        self,
        DescriptorFields,
        EncodeError,
    },
};
use ed25519_compact::{
    KeyPair,
    Signature,
};
fn fields(stack_pages_per_thread: u16) -> DescriptorFields<'static> {
    DescriptorFields {
        sequence: 7,
        node_key: 0x1234,
        artifact_digest: [0xa5; 32],
        artifact_name: b"orders",
        stack_pages_per_thread,
        object_key: b"releases/orders.elf",
        grants: &[],
    }
}

fn sign(bytes: &mut [u8], pair: &KeyPair) {
    let digest = deployment::signature_digest(bytes).unwrap();
    let signature: Signature = pair.sk.sign(digest, None);
    assert!(deployment::set_signature(bytes, signature.as_ref().try_into().unwrap()));
}

#[test]
fn v2_round_trip_binds_stack_pages() {
    let pair = KeyPair::from_seed([0x31; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(16);
    let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
    sign(&mut bytes, &pair);

    let decoded = deployment::decode(&bytes).unwrap();
    assert_eq!(decoded.format_version, deployment::VERSION);
    assert_eq!(decoded.stack_pages_per_thread, 16);
    assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);

    bytes[70] ^= 1;
    assert_ne!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
}

#[test]
fn v1_decodes_with_the_historical_default() {
    let pair = KeyPair::from_seed([0x42; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(DEFAULT_USER_STACK_PAGES as u16);
    let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
    bytes[..8].copy_from_slice(deployment::LEGACY_MAGIC);
    bytes[8..10].copy_from_slice(&deployment::LEGACY_VERSION.to_le_bytes());
    bytes[70..72].fill(0);
    sign(&mut bytes, &pair);

    let decoded = deployment::decode(&bytes).unwrap();
    assert_eq!(decoded.format_version, deployment::LEGACY_VERSION);
    assert_eq!(decoded.stack_pages_per_thread, DEFAULT_USER_STACK_PAGES as u16);
    assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
}

#[test]
fn encoder_rejects_zero_and_excessive_stack_requirements() {
    assert_eq!(deployment::encoded_len(&fields(0)), Err(EncodeError::InvalidStackPages));
    assert_eq!(
        deployment::encoded_len(&fields(MAX_USER_STACK_PAGES as u16 + 1)),
        Err(EncodeError::InvalidStackPages)
    );
}

#[test]
fn decoder_rejects_invalid_v2_pages_and_noncanonical_legacy_bytes() {
    let pair = KeyPair::from_seed([0x53; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(DEFAULT_USER_STACK_PAGES as u16);
    let mut canonical = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut canonical).unwrap();

    let mut zero = canonical.clone();
    zero[70..72].fill(0);
    assert!(deployment::decode(&zero).is_none());

    let mut excessive = canonical.clone();
    excessive[70..72].copy_from_slice(&(MAX_USER_STACK_PAGES as u16 + 1).to_le_bytes());
    assert!(deployment::decode(&excessive).is_none());

    let mut legacy = canonical;
    legacy[..8].copy_from_slice(deployment::LEGACY_MAGIC);
    legacy[8..10].copy_from_slice(&deployment::LEGACY_VERSION.to_le_bytes());
    assert!(deployment::decode(&legacy).is_none());
}
