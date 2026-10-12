use charlotte_launch::{
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
        shutdown_grace_ms: 12_000,
        placement: charlotte_launch::placement::PlacementPolicy::singleton(),
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
fn v5_round_trip_binds_execution_shutdown_and_placement_resources() {
    let pair = KeyPair::from_seed([0x31; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(16, 8);
    let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
    sign(&mut bytes, &pair);

    let decoded = deployment::decode(&bytes).unwrap();
    assert_eq!(decoded.stack_pages_per_thread, 16);
    assert_eq!(decoded.max_threads, 8);
    assert_eq!(decoded.shutdown_grace_ms, 12_000);
    assert_eq!(decoded.placement, charlotte_launch::placement::PlacementPolicy::singleton());
    assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);

    let mut stack_tamper = bytes.clone();
    stack_tamper[70] ^= 1;
    assert_ne!(deployment::verify(&stack_tamper, public), deployment::VerifyOutcome::Valid);

    bytes[deployment::MAX_THREADS_OFFSET] ^= 1;
    assert_ne!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
}

#[test]
fn v5_round_trip_carries_replica_policy() {
    let pair = KeyPair::from_seed([0x21; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let mut fields = fields(16, 8);
    fields.node_key = 0;
    fields.placement = charlotte_launch::placement::PlacementPolicy {
        replicas: 3,
        max_instances_per_node: 1,
        min_distinct_nodes: 3,
        flags: charlotte_launch::placement::SPREAD_REPLICAS,
        affinity_group: 0,
        anti_affinity_group: 7,
    };
    let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
    sign(&mut bytes, &pair);
    assert_eq!(deployment::decode(&bytes).unwrap().placement, fields.placement);
    assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
}

#[test]
fn retired_formats_reject_even_with_valid_descriptor_and_release_signatures() {
    use charlotte_launch::{
        release,
        sha256,
    };
    let pair = KeyPair::from_seed([0x64; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(4, 3);
    let mut canonical = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut canonical).unwrap();
    sign(&mut canonical, &pair);
    let descriptors = [canonical.as_slice()];
    let release_fields = release::ReleaseFields {
        sequence: 1,
        release_name: b"orders",
        descriptors: &descriptors,
    };
    let mut envelope = vec![0; release::encoded_len(&release_fields).unwrap()];
    release::encode_unsigned(&release_fields, public, &mut envelope).unwrap();
    // Retired layouts exist only as rejection fixtures. Sign raw bytes directly:
    // the current production signing helpers must refuse their grammar too.
    for (magic, version, header_len) in [
        (b"CDEPLOY1", 1u16, 152usize),
        (b"CDEPLOY2", 2, 152),
        (b"CDEPLOY3", 3, 154),
        (b"CDEPLOY4", 4, 158),
    ] {
        let mut retired = canonical.clone();
        retired.drain(header_len..deployment::HEADER_LEN);
        retired[..8].copy_from_slice(magic);
        retired[8..10].copy_from_slice(&version.to_le_bytes());
        retired[10..12].copy_from_slice(&(header_len as u16).to_le_bytes());
        let len = retired.len() as u32;
        retired[12..16].copy_from_slice(&len.to_le_bytes());
        if version == 1 {
            retired[70..72].fill(0);
        }
        let digest = sha256::digest_skipping(
            &retired,
            deployment::SIGNATURE_OFFSET,
            deployment::SIGNATURE_LEN,
        );
        let signature = pair.sk.sign(digest, None);
        assert!(pair.pk.verify(digest, &signature).is_ok());
        retired[deployment::SIGNATURE_OFFSET
            ..deployment::SIGNATURE_OFFSET + deployment::SIGNATURE_LEN]
            .copy_from_slice(signature.as_ref());
        assert!(deployment::decode(&retired).is_none());
        assert_eq!(deployment::verify(&retired, public), deployment::VerifyOutcome::Invalid);
        assert!(deployment::signature_digest(&retired).is_none());
        assert!(!deployment::set_signature(&mut retired, signature.as_ref().try_into().unwrap()));
        let descriptors = [retired.as_slice()];
        let fields = release::ReleaseFields {
            descriptors: &descriptors,
            ..release_fields
        };
        assert_eq!(release::encoded_len(&fields), Err(release::EncodeError::InvalidDescriptor));
        // A cryptographically signed outer envelope cannot revive an old inner
        // descriptor. Preserve canonical framing while substituting its bytes.
        let mut outer = envelope.clone();
        let length_offset = release::HEADER_LEN + release_fields.release_name.len();
        outer.truncate(length_offset);
        outer.extend_from_slice(&(retired.len() as u16).to_le_bytes());
        outer.extend_from_slice(&retired);
        let len = outer.len() as u32;
        outer[12..16].copy_from_slice(&len.to_le_bytes());
        let digest =
            sha256::digest_skipping(&outer, release::SIGNATURE_OFFSET, release::SIGNATURE_LEN);
        let signature = pair.sk.sign(digest, None);
        assert!(pair.pk.verify(digest, &signature).is_ok());
        outer[release::SIGNATURE_OFFSET..release::SIGNATURE_OFFSET + release::SIGNATURE_LEN]
            .copy_from_slice(signature.as_ref());
        assert!(release::decode(&outer).is_none());
        assert_eq!(release::verify(&outer, public), release::VerifyOutcome::Invalid);
    }
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
    let mut invalid_grace = fields(1, 1);
    invalid_grace.shutdown_grace_ms = charlotte_launch::MAX_SHUTDOWN_GRACE_MS + 1;
    assert_eq!(deployment::encoded_len(&invalid_grace), Err(EncodeError::InvalidShutdownGrace));
    let mut pinned_replicas = fields(1, 1);
    pinned_replicas.placement.replicas = 2;
    pinned_replicas.placement.min_distinct_nodes = 2;
    assert_eq!(deployment::encoded_len(&pinned_replicas), Err(EncodeError::InvalidPlacement));
}

#[test]
fn decoder_rejects_invalid_execution_resources() {
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
}

#[test]
fn current_header_requires_exact_magic_version_length_and_complete_fields() {
    let pair = KeyPair::from_seed([0x65; 32].into());
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = fields(4, 1);
    let mut canonical = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, public, &mut canonical).unwrap();
    for (offset, value) in [(0, b'X'), (8, 1), (8, 6), (10, 152), (10, 181), (10, 183)] {
        let mut malformed = canonical.clone();
        malformed[offset] = value;
        assert!(deployment::decode(&malformed).is_none());
    }
    for len in 0..deployment::HEADER_LEN {
        assert!(deployment::decode(&canonical[..len]).is_none());
    }
    assert!(deployment::decode(&canonical).is_some());
}
