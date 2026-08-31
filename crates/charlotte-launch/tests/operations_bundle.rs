use charlotte_launch::{
    deployment::{
        self,
        DescriptorFields,
    },
    operations::{
        self,
        EnvelopeFields,
        PROFILE_KIND_KAFKA,
    },
    operations_bundle::{
        self,
        BindingFields,
        BundleFields,
        VerifyOutcome,
    },
    release::{
        self,
        ReleaseFields,
    },
};
use ed25519_compact::{
    KeyPair,
    Signature,
};
use rand_core::UnwrapErr;

fn sign_deployment(bytes: &mut [u8], pair: &KeyPair) {
    let signature: Signature = pair.sk.sign(deployment::signature_digest(bytes).unwrap(), None);
    assert!(deployment::set_signature(bytes, signature.as_ref().try_into().unwrap()));
}

fn signed_release(pair: &KeyPair) -> Vec<u8> {
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let descriptor_fields = DescriptorFields {
        sequence: 3,
        node_key: 0,
        artifact_digest: [0x33; 32],
        artifact_name: b"kafka",
        stack_pages_per_thread: 32,
        max_threads: 16,
        object_key: b"releases/kafka.elf",
        grants: &[],
    };
    let mut descriptor = vec![0; deployment::encoded_len(&descriptor_fields).unwrap()];
    deployment::encode_unsigned(&descriptor_fields, public, &mut descriptor).unwrap();
    sign_deployment(&mut descriptor, pair);
    let descriptors = [descriptor.as_slice()];
    let fields = ReleaseFields {
        sequence: 7,
        release_name: b"orders",
        descriptors: &descriptors,
    };
    let mut release = vec![0; release::encoded_len(&fields).unwrap()];
    release::encode_unsigned(&fields, public, &mut release).unwrap();
    let signature: Signature = pair.sk.sign(release::signature_digest(&release).unwrap(), None);
    assert!(release::set_signature(&mut release, signature.as_ref().try_into().unwrap()));
    release
}

struct Fixture {
    bundle: Vec<u8>,
    release_pair: KeyPair,
    operational_pair: KeyPair,
    recipient_public: [u8; 32],
    cluster_id: [u8; 32],
}

fn fixture(expiry: u64) -> Fixture {
    let release_pair = KeyPair::from_seed([0x31; 32].into());
    let operational_pair = KeyPair::from_seed([0x42; 32].into());
    let release = signed_release(&release_pair);
    let operational_public: &[u8; 32] = operational_pair.pk.as_ref().try_into().unwrap();
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (_, recipient_public) = operations::generate_recipient_keypair(&mut rng);
    let cluster_id = [0x11; 32];
    let envelope_fields = EnvelopeFields {
        sequence: 9,
        expires_unix_seconds: expiry,
        profile_kind: PROFILE_KIND_KAFKA,
        cluster_id,
        release_digest: charlotte_launch::sha256::digest(&release),
        profile_name: b"kafka/orders/transactional",
    };
    let profile = b"broker=managed.example credential=operator-only";
    let mut envelope = vec![0; operations::encoded_len(&envelope_fields, profile.len()).unwrap()];
    operations::seal_unsigned(
        &envelope_fields,
        profile,
        &recipient_public,
        operational_public,
        &mut rng,
        &mut envelope,
    )
    .unwrap();
    let signature: Signature =
        operational_pair.sk.sign(operations::signature_digest(&envelope).unwrap(), None);
    assert!(operations::set_signature(&mut envelope, signature.as_ref().try_into().unwrap()));

    let bindings = [BindingFields {
        target_artifact: b"kafka",
        object_key: b"operations/orders-kafka.cops",
        envelope: &envelope,
    }];
    let fields = BundleFields {
        sequence: 5,
        cluster_id,
        release: &release,
        bindings: &bindings,
    };
    let mut bundle = vec![0; operations_bundle::encoded_len(&fields).unwrap()];
    operations_bundle::encode_unsigned(&fields, operational_public, &recipient_public, &mut bundle)
        .unwrap();
    let binding_signature: Signature = operational_pair
        .sk
        .sign(operations_bundle::binding_signature_digest(&bundle, 0).unwrap(), None);
    assert!(operations_bundle::set_binding_signature(
        &mut bundle,
        0,
        binding_signature.as_ref().try_into().unwrap()
    ));
    let signature: Signature =
        operational_pair.sk.sign(operations_bundle::signature_digest(&bundle).unwrap(), None);
    assert!(operations_bundle::set_signature(&mut bundle, signature.as_ref().try_into().unwrap()));
    Fixture {
        bundle,
        release_pair,
        operational_pair,
        recipient_public,
        cluster_id,
    }
}

#[test]
fn bundle_joins_exact_release_mapping_and_encrypted_profile() {
    let fixture = fixture(2_000_000_000);
    let release_public: &[u8; 32] = fixture.release_pair.pk.as_ref().try_into().unwrap();
    let operational_public: &[u8; 32] = fixture.operational_pair.pk.as_ref().try_into().unwrap();
    assert_eq!(
        operations_bundle::verify(
            &fixture.bundle,
            release_public,
            operational_public,
            &fixture.recipient_public,
            &fixture.cluster_id,
            1_900_000_000,
        ),
        VerifyOutcome::Valid
    );
    let bundle = operations_bundle::decode(&fixture.bundle).unwrap();
    assert_eq!(bundle.sequence, 5);
    assert_eq!(bundle.release_digest, charlotte_launch::sha256::digest(bundle.release));
    let binding = bundle.bindings().next().unwrap();
    assert_eq!(binding.target_artifact, b"kafka");
    assert_eq!(binding.object_key, b"operations/orders-kafka.cops");
    assert_eq!(binding.envelope_digest, charlotte_launch::sha256::digest(binding.envelope));
    assert!(operations_bundle::verify_binding_authorization(
        bundle.sequence,
        &bundle.cluster_id,
        &bundle.release_digest,
        &bundle.recipient_key_id,
        &bundle.signing_key_id,
        binding.target_artifact,
        binding.object_key,
        &binding.envelope_digest,
        &binding.authorization_signature,
        operational_public,
    ));
    assert!(!operations_bundle::verify_binding_authorization(
        bundle.sequence,
        &bundle.cluster_id,
        &bundle.release_digest,
        &bundle.recipient_key_id,
        &bundle.signing_key_id,
        b"different-connector",
        binding.object_key,
        &binding.envelope_digest,
        &binding.authorization_signature,
        operational_public,
    ));
}

#[test]
fn outer_signature_protects_the_connector_mapping() {
    let mut fixture = fixture(2_000_000_000);
    let release_public: &[u8; 32] = fixture.release_pair.pk.as_ref().try_into().unwrap();
    let operational_public: &[u8; 32] = fixture.operational_pair.pk.as_ref().try_into().unwrap();
    let offset = fixture
        .bundle
        .windows(b"operations/orders-kafka.cops".len())
        .position(|window| window == b"operations/orders-kafka.cops")
        .unwrap();
    fixture.bundle[offset + 11] ^= 1;
    assert!(operations_bundle::decode(&fixture.bundle).is_some());
    assert_eq!(
        operations_bundle::verify(
            &fixture.bundle,
            release_public,
            operational_public,
            &fixture.recipient_public,
            &fixture.cluster_id,
            1_900_000_000,
        ),
        VerifyOutcome::Invalid
    );
}

#[test]
fn verification_rejects_wrong_authorities_context_and_expiry() {
    let fixture = fixture(1_900_000_000);
    let release_public: &[u8; 32] = fixture.release_pair.pk.as_ref().try_into().unwrap();
    let operational_public: &[u8; 32] = fixture.operational_pair.pk.as_ref().try_into().unwrap();
    let wrong_pair = KeyPair::from_seed([0x55; 32].into());
    let wrong_public: &[u8; 32] = wrong_pair.pk.as_ref().try_into().unwrap();
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (_, wrong_recipient) = operations::generate_recipient_keypair(&mut rng);

    assert_eq!(
        operations_bundle::verify(
            &fixture.bundle,
            wrong_public,
            operational_public,
            &fixture.recipient_public,
            &fixture.cluster_id,
            1,
        ),
        VerifyOutcome::WrongReleaseKey
    );
    assert_eq!(
        operations_bundle::verify(
            &fixture.bundle,
            release_public,
            wrong_public,
            &fixture.recipient_public,
            &fixture.cluster_id,
            1,
        ),
        VerifyOutcome::WrongOperationalKey
    );
    assert_eq!(
        operations_bundle::verify(
            &fixture.bundle,
            release_public,
            operational_public,
            &wrong_recipient,
            &fixture.cluster_id,
            1,
        ),
        VerifyOutcome::WrongRecipient
    );
    assert_eq!(
        operations_bundle::verify(
            &fixture.bundle,
            release_public,
            operational_public,
            &fixture.recipient_public,
            &[0x77; 32],
            1,
        ),
        VerifyOutcome::WrongCluster
    );
    assert_eq!(
        operations_bundle::verify(
            &fixture.bundle,
            release_public,
            operational_public,
            &fixture.recipient_public,
            &fixture.cluster_id,
            1_900_000_001,
        ),
        VerifyOutcome::Expired
    );
}

#[test]
fn mappings_are_unique_and_target_a_release_component() {
    let fixture = fixture(2_000_000_000);
    let decoded = operations_bundle::decode(&fixture.bundle).unwrap();
    let existing = decoded.bindings().next().unwrap();
    let duplicates = [
        BindingFields {
            target_artifact: existing.target_artifact,
            object_key: existing.object_key,
            envelope: existing.envelope,
        },
        BindingFields {
            target_artifact: existing.target_artifact,
            object_key: b"operations/other.cops",
            envelope: existing.envelope,
        },
    ];
    let fields = BundleFields {
        sequence: 6,
        cluster_id: fixture.cluster_id,
        release: decoded.release,
        bindings: &duplicates,
    };
    assert_eq!(
        operations_bundle::encoded_len(&fields),
        Err(operations_bundle::EncodeError::DuplicateBinding)
    );

    let missing = [BindingFields {
        target_artifact: b"s3",
        object_key: b"operations/s3.cops",
        envelope: existing.envelope,
    }];
    let fields = BundleFields {
        sequence: 6,
        cluster_id: fixture.cluster_id,
        release: decoded.release,
        bindings: &missing,
    };
    assert_eq!(
        operations_bundle::encoded_len(&fields),
        Err(operations_bundle::EncodeError::InvalidBinding)
    );
}
