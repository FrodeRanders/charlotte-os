use charlotte_launch::{
    operations,
    trust::{
        self,
        AdmissionTrust,
        ProductionTrustCandidate,
        signed_policy::{
            self,
            BootstrapKey,
            PolicyError,
            PolicyExpectation,
        },
    },
};
use ed25519_compact::{
    KeyPair,
    Seed,
};

// Deterministic, publicly known test seeds, never provisioned material.
fn pair(seed: u8) -> KeyPair {
    KeyPair::from_seed(Seed::new([seed; 32]))
}

fn candidate(sequence: u64) -> ProductionTrustCandidate {
    let trust = AdmissionTrust {
        sequence,
        cluster_id: trust::cluster_id(b"signed-policy-tests").unwrap(),
        artifact_key: *pair(20).pk,
        deployment_key: *pair(21).pk,
        operations_key: *pair(22).pk,
        recipient_key: operations::recipient_public_key(&[77; 32]).unwrap(),
    };
    ProductionTrustCandidate::validate(trust, &trust.cluster_id, 1).unwrap()
}

fn sign(
    policy: &ProductionTrustCandidate,
    predecessor: [u8; 32],
) -> [u8; signed_policy::ENCODED_LEN] {
    let key = pair(31);
    let mut bytes =
        signed_policy::encode_unsigned(policy, &BootstrapKey::new(*key.pk).unwrap(), predecessor)
            .unwrap();
    let signature = key.sk.sign(signed_policy::signature_digest(&bytes).unwrap(), None);
    signed_policy::set_signature(&mut bytes, signature.as_ref().try_into().unwrap()).unwrap();
    bytes
}

fn verify(
    bytes: &[u8],
    expected: &PolicyExpectation,
) -> Result<signed_policy::VerifiedPolicy, PolicyError> {
    signed_policy::verify(
        bytes,
        &BootstrapKey::new(*pair(31).pk).unwrap(),
        &candidate(7).public().cluster_id,
        expected,
    )
}

#[test]
fn kernel_boot_handoff_fixture_is_reproducible_public_test_material() {
    // These deterministic seeds and recipient value are publicly known test
    // material. The checked-in kernel record contains public bytes only.
    let public = AdmissionTrust {
        cluster_id: trust::cluster_id(b"boot-trust-tests").unwrap(),
        ..*candidate(7).public()
    };
    let policy = ProductionTrustCandidate::validate(public, &public.cluster_id, 1).unwrap();
    let fixture = include_bytes!("../../catten/src/service/admission/test-policy.bin");
    assert_eq!(fixture, &sign(&policy, [0; 32]));
    let verified = signed_policy::verify(
        fixture,
        &BootstrapKey::new(*pair(31).pk).unwrap(),
        &public.cluster_id,
        &PolicyExpectation::enrollment(7).unwrap(),
    )
    .unwrap();
    assert_eq!(verified.policy().public(), &public);
}

#[test]
fn exact_public_policy_is_signed_with_a_separate_bootstrap_key() {
    let policy = candidate(7);
    let bytes = sign(&policy, [0; 32]);
    assert_eq!(bytes.len(), 328);
    assert_eq!(&bytes[..16], b"CBTRUST1\x01\0\x50\0\x48\x01\0\0");
    assert_eq!(&bytes[80..264], &policy.encode());
    let verified = verify(&bytes, &PolicyExpectation::enrollment(7).unwrap()).unwrap();
    assert_eq!(verified.policy(), &policy);
    assert_eq!(verified.digest(), signed_policy::signature_digest(&bytes).unwrap());
    assert_ne!(verified.digest(), charlotte_launch::sha256::digest(&bytes[..264]));
    assert_eq!(verify(&bytes, &verified.installed_expectation().unwrap()), Ok(verified));
}

#[test]
fn every_record_byte_is_either_structurally_checked_or_authenticated() {
    let bytes = sign(&candidate(7), [0; 32]);
    let expected = PolicyExpectation::enrollment(7).unwrap();
    for offset in 0..bytes.len() {
        let mut tampered = bytes;
        tampered[offset] ^= 1;
        assert!(verify(&tampered, &expected).is_err(), "tampered byte {offset} accepted");
    }
    for length in 0..bytes.len() {
        assert!(verify(&bytes[..length], &expected).is_err());
        assert!(signed_policy::signature_digest(&bytes[..length]).is_err());
        assert!(signed_policy::set_signature(&mut bytes[..length].to_vec(), &[0; 64]).is_err());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(verify(&trailing, &expected).is_err());
    assert!(signed_policy::signature_digest(&trailing).is_err());
}

#[test]
fn a_record_cannot_select_its_own_anchor_or_cluster() {
    let mut bytes = sign(&candidate(7), [0; 32]);
    let expected = PolicyExpectation::enrollment(1).unwrap();
    let other_root = BootstrapKey::new(*pair(32).pk).unwrap();
    assert_eq!(
        signed_policy::verify(&bytes, &other_root, &candidate(7).public().cluster_id, &expected),
        Err(PolicyError::WrongBootstrapKey)
    );
    // Forging the selector to match a caller's anchor does not forge a signature.
    bytes[16..48].copy_from_slice(&charlotte_launch::sha256::digest(pair(32).pk.as_ref()));
    assert_eq!(
        signed_policy::verify(&bytes, &other_root, &candidate(7).public().cluster_id, &expected),
        Err(PolicyError::InvalidSignature)
    );
    assert!(
        signed_policy::verify(
            &sign(&candidate(7), [0; 32]),
            &BootstrapKey::new(*pair(31).pk).unwrap(),
            &trust::cluster_id(b"another-cluster").unwrap(),
            &expected
        )
        .is_err()
    );
}

#[test]
fn unsigned_wrong_signer_and_unscoped_signatures_reject() {
    let root = BootstrapKey::new(*pair(31).pk).unwrap();
    let mut bytes = signed_policy::encode_unsigned(&candidate(7), &root, [0; 32]).unwrap();
    let expected = PolicyExpectation::enrollment(1).unwrap();
    assert_eq!(verify(&bytes, &expected), Err(PolicyError::InvalidSignature));
    let wrong = pair(32).sk.sign(signed_policy::signature_digest(&bytes).unwrap(), None);
    signed_policy::set_signature(&mut bytes, wrong.as_ref().try_into().unwrap()).unwrap();
    assert_eq!(verify(&bytes, &expected), Err(PolicyError::InvalidSignature));
    let unscoped = pair(31).sk.sign(charlotte_launch::sha256::digest(&bytes[..264]), None);
    signed_policy::set_signature(&mut bytes, unscoped.as_ref().try_into().unwrap()).unwrap();
    assert_eq!(verify(&bytes, &expected), Err(PolicyError::InvalidSignature));
}

#[test]
fn bootstrap_key_rejects_fixture_aliases_weak_points_and_operational_role_reuse() {
    for fixture in [
        charlotte_launch::CLUSTER_PUBLIC_KEY,
        charlotte_launch::DEVELOPMENT_OPERATIONS_PUBLIC_KEY,
        charlotte_launch::DEVELOPMENT_RECIPIENT_PUBLIC_KEY,
    ] {
        assert!(BootstrapKey::new(fixture).is_err());
        let mut negated = fixture;
        negated[31] ^= 0x80;
        assert!(BootstrapKey::new(negated).is_err());
    }
    // The recipient fixture is also forbidden after conversion to a signing
    // point: its published private material is not a fresh bootstrap root.
    let recipient_point = curve25519_dalek::montgomery::MontgomeryPoint(
        charlotte_launch::DEVELOPMENT_RECIPIENT_PUBLIC_KEY,
    )
    .to_edwards(0)
    .unwrap();
    assert!(BootstrapKey::new(recipient_point.compress().to_bytes()).is_err());
    assert!(BootstrapKey::new((-recipient_point).compress().to_bytes()).is_err());
    use curve25519_dalek::constants::{
        ED25519_BASEPOINT_POINT,
        EIGHT_TORSION,
    };
    for weak in [
        [0; 32],
        [0xff; 32],
        EIGHT_TORSION[0].compress().to_bytes(),
        (ED25519_BASEPOINT_POINT + EIGHT_TORSION[1]).compress().to_bytes(),
    ] {
        assert!(BootstrapKey::new(weak).is_err());
    }
    for signing_key in [
        candidate(7).public().artifact_key,
        candidate(7).public().deployment_key,
        candidate(7).public().operations_key,
    ] {
        for sign in [0, 0x80] {
            let mut alias = signing_key;
            alias[31] ^= sign;
            let bootstrap = BootstrapKey::new(alias).unwrap();
            assert_eq!(
                signed_policy::encode_unsigned(&candidate(7), &bootstrap, [0; 32]),
                Err(PolicyError::SharedBootstrapRole)
            );
        }
    }
    let root = pair(31);
    let mut trust = *candidate(7).public();
    trust.recipient_key = curve25519_dalek::edwards::CompressedEdwardsY(*root.pk)
        .decompress()
        .unwrap()
        .to_montgomery()
        .to_bytes();
    let shared_candidate = ProductionTrustCandidate::validate(trust, &trust.cluster_id, 1).unwrap();
    assert_eq!(
        signed_policy::encode_unsigned(
            &shared_candidate,
            &BootstrapKey::new(*root.pk).unwrap(),
            [0; 32]
        ),
        Err(PolicyError::SharedBootstrapRole)
    );
    // A valid signature must not bypass the same role check in verification.
    let mut bytes = sign(&candidate(7), [0; 32]);
    bytes[80..264].copy_from_slice(&shared_candidate.encode());
    let signature = root.sk.sign(signed_policy::signature_digest(&bytes).unwrap(), None);
    signed_policy::set_signature(&mut bytes, signature.as_ref().try_into().unwrap()).unwrap();
    assert_eq!(
        verify(&bytes, &PolicyExpectation::enrollment(1).unwrap()),
        Err(PolicyError::SharedBootstrapRole)
    );
}

#[test]
fn signed_revision_lineage_rejects_rollback_conflicts_gaps_and_forks() {
    let initial = sign(&candidate(7), [0; 32]);
    let accepted = verify(&initial, &PolicyExpectation::enrollment(7).unwrap()).unwrap();
    let installed = accepted.installed_expectation().unwrap();
    let next = sign(&candidate(8), installed.signing_predecessor(8).unwrap());
    let advanced = verify(&next, &installed).unwrap();
    assert_eq!(
        verify(&initial, &advanced.installed_expectation().unwrap()),
        Err(PolicyError::RevisionRollback)
    );
    assert_eq!(
        verify(&initial, &PolicyExpectation::enrollment(8).unwrap()),
        Err(PolicyError::RevisionRollback)
    );
    assert_eq!(
        verify(&next, &PolicyExpectation::enrollment(8).unwrap()),
        Err(PolicyError::WrongPredecessor)
    );
    assert_eq!(
        verify(&sign(&candidate(8), [0; 32]), &installed),
        Err(PolicyError::WrongPredecessor)
    );
    assert_eq!(
        verify(&sign(&candidate(8), [0xa5; 32]), &installed),
        Err(PolicyError::WrongPredecessor)
    );
    assert_eq!(
        verify(&sign(&candidate(9), accepted.digest()), &installed),
        Err(PolicyError::RevisionGap)
    );
    let mut substituted = *candidate(7).public();
    substituted.operations_key = *pair(24).pk;
    let substituted =
        ProductionTrustCandidate::validate(substituted, &substituted.cluster_id, 1).unwrap();
    assert_eq!(
        verify(&sign(&substituted, [0; 32]), &installed),
        Err(PolicyError::RevisionConflict)
    );
    let mut rotated = *candidate(8).public();
    rotated.operations_key = *pair(24).pk;
    let rotated = ProductionTrustCandidate::validate(rotated, &rotated.cluster_id, 1).unwrap();
    let fork = sign(&rotated, accepted.digest());
    // Both successors may be signed by an authorized operator. Once one is
    // installed, its exact digest fences the other. The installer must commit
    // acceptance state atomically, not reuse a stale expectation concurrently.
    assert!(verify(&fork, &installed).is_ok());
    assert_eq!(
        verify(&fork, &advanced.installed_expectation().unwrap()),
        Err(PolicyError::RevisionConflict)
    );
}

#[test]
fn changed_signature_does_not_change_the_accepted_policy_digest() {
    let mut bytes = sign(&candidate(7), [0; 32]);
    let first = verify(&bytes, &PolicyExpectation::enrollment(7).unwrap()).unwrap();
    let noise = ed25519_compact::Noise::new([42; ed25519_compact::Noise::BYTES]);
    let signature = pair(31).sk.sign(first.digest(), Some(noise));
    assert_ne!(&bytes[264..], signature.as_ref());
    signed_policy::set_signature(&mut bytes, signature.as_ref().try_into().unwrap()).unwrap();
    assert_eq!(verify(&bytes, &first.installed_expectation().unwrap()), Ok(first));
}

#[test]
fn expectation_requires_explicit_valid_state_and_never_wraps_revisions() {
    assert_eq!(PolicyExpectation::enrollment(0), Err(PolicyError::InvalidExpectation));
    assert_eq!(PolicyExpectation::installed(0, [9; 32]), Err(PolicyError::InvalidExpectation));
    assert_eq!(PolicyExpectation::installed(1, [0; 32]), Err(PolicyError::InvalidExpectation));
    let bytes = sign(&candidate(u64::MAX), [0; 32]);
    let accepted = verify(&bytes, &PolicyExpectation::enrollment(u64::MAX).unwrap()).unwrap();
    let installed = accepted.installed_expectation().unwrap();
    assert_eq!(verify(&bytes, &installed), Ok(accepted));
    assert_eq!(installed.signing_predecessor(0), Err(PolicyError::RevisionGap));
    assert_eq!(installed.signing_predecessor(u64::MAX), Err(PolicyError::RevisionGap));
    assert!(
        PolicyExpectation::installed(u64::MAX - 1, [9; 32])
            .unwrap()
            .signing_predecessor(u64::MAX)
            .is_ok()
    );
}
