use charlotte_launch::{
    DEFAULT_USER_MAX_THREADS,
    DEFAULT_USER_STACK_PAGES,
    MAX_USER_STACK_PAGES,
    MAX_USER_THREADS,
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

fn fields(stack_pages_per_thread: u16, max_threads: u16) -> DescriptorFields<'static> {
    DescriptorFields {
        sequence: 7,
        node_key: 0x1234,
        artifact_digest: [0xa5; 32],
        artifact_name: b"orders",
        stack_pages_per_thread,
        max_threads,
        object_key: b"releases/orders.elf",
        grants: &[],
    }
}

fn sign(bytes: &mut [u8], pair: &KeyPair) {
    let digest = deployment::signature_digest(bytes).unwrap();
    let signature: Signature = pair.sk.sign(digest, None);
    assert!(deployment::set_signature(bytes, signature.as_ref().try_into().unwrap()));
}

fn downgrade(mut bytes: Vec<u8>, magic: &[u8; 8], version: u16) -> Vec<u8> {
    bytes.drain(deployment::LEGACY_HEADER_LEN..deployment::HEADER_LEN);
    let total_len = bytes.len() as u32;
    bytes[..8].copy_from_slice(magic);
    bytes[8..10].copy_from_slice(&version.to_le_bytes());
    bytes[10..12].copy_from_slice(&(deployment::LEGACY_HEADER_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&total_len.to_le_bytes());
    bytes
}

#[test]
fn v3_round_trip_binds_execution_resources() {
    let pair = KeyPair::from_seed([0x31; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(16, 8);
    let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
    sign(&mut bytes, &pair);

    let decoded = deployment::decode(&bytes).unwrap();
    assert_eq!(decoded.format_version, deployment::VERSION);
    assert_eq!(decoded.stack_pages_per_thread, 16);
    assert_eq!(decoded.max_threads, 8);
    assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);

    let mut stack_tamper = bytes.clone();
    stack_tamper[70] ^= 1;
    assert_ne!(deployment::verify(&stack_tamper, public), deployment::VerifyOutcome::Valid);

    bytes[deployment::MAX_THREADS_OFFSET] ^= 1;
    assert_ne!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
}

#[test]
fn v2_decodes_with_stack_pages_and_default_thread_limit() {
    let pair = KeyPair::from_seed([0x42; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(12, 3);
    let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
    let mut bytes = downgrade(bytes, deployment::V2_MAGIC, deployment::V2_VERSION);
    sign(&mut bytes, &pair);

    let decoded = deployment::decode(&bytes).unwrap();
    assert_eq!(decoded.format_version, deployment::V2_VERSION);
    assert_eq!(decoded.stack_pages_per_thread, 12);
    assert_eq!(decoded.max_threads, DEFAULT_USER_MAX_THREADS as u16);
    assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
}

#[test]
fn v1_decodes_with_historical_defaults() {
    let pair = KeyPair::from_seed([0x64; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(DEFAULT_USER_STACK_PAGES as u16, 3);
    let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
    let mut bytes = downgrade(bytes, deployment::LEGACY_MAGIC, deployment::LEGACY_VERSION);
    bytes[70..72].fill(0);
    sign(&mut bytes, &pair);

    let decoded = deployment::decode(&bytes).unwrap();
    assert_eq!(decoded.format_version, deployment::LEGACY_VERSION);
    assert_eq!(decoded.stack_pages_per_thread, DEFAULT_USER_STACK_PAGES as u16);
    assert_eq!(decoded.max_threads, DEFAULT_USER_MAX_THREADS as u16);
    assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
}

#[test]
fn encoder_rejects_invalid_execution_resources() {
    assert_eq!(deployment::encoded_len(&fields(0, 1)), Err(EncodeError::InvalidStackPages));
    assert_eq!(
        deployment::encoded_len(&fields(MAX_USER_STACK_PAGES as u16 + 1, 1)),
        Err(EncodeError::InvalidStackPages)
    );
    assert_eq!(deployment::encoded_len(&fields(1, 0)), Err(EncodeError::InvalidMaxThreads));
    assert_eq!(
        deployment::encoded_len(&fields(1, MAX_USER_THREADS as u16 + 1)),
        Err(EncodeError::InvalidMaxThreads)
    );
}

#[test]
fn decoder_rejects_invalid_v3_and_noncanonical_legacy_resources() {
    let pair = KeyPair::from_seed([0x75; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(DEFAULT_USER_STACK_PAGES as u16, 1);
    let mut canonical = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut canonical).unwrap();

    let mut zero_stack = canonical.clone();
    zero_stack[70..72].fill(0);
    assert!(deployment::decode(&zero_stack).is_none());

    let mut excessive_stack = canonical.clone();
    excessive_stack[70..72].copy_from_slice(&(MAX_USER_STACK_PAGES as u16 + 1).to_le_bytes());
    assert!(deployment::decode(&excessive_stack).is_none());

    let mut zero_threads = canonical.clone();
    zero_threads[deployment::MAX_THREADS_OFFSET..deployment::MAX_THREADS_OFFSET + 2].fill(0);
    assert!(deployment::decode(&zero_threads).is_none());

    let mut excessive_threads = canonical.clone();
    excessive_threads[deployment::MAX_THREADS_OFFSET..deployment::MAX_THREADS_OFFSET + 2]
        .copy_from_slice(&(MAX_USER_THREADS as u16 + 1).to_le_bytes());
    assert!(deployment::decode(&excessive_threads).is_none());

    let mut legacy = downgrade(canonical, deployment::LEGACY_MAGIC, deployment::LEGACY_VERSION);
    assert!(deployment::decode(&legacy).is_none());
    legacy[70..72].fill(0);
    assert!(deployment::decode(&legacy).is_some());
}
