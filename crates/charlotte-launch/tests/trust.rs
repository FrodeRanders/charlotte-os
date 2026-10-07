use charlotte_launch::trust::{
    self,
    AdmissionTrust,
    ProductionTrustCandidate,
    ProductionTrustError,
};

fn fixture() -> AdmissionTrust {
    AdmissionTrust {
        sequence: 7,
        cluster_id: [0x11; 32],
        artifact_key: [0x22; 32],
        deployment_key: [0x33; 32],
        operations_key: [0x44; 32],
        recipient_key: [0x55; 32],
    }
}

fn production_fixture() -> AdmissionTrust {
    fn signing_key(seed: u8) -> [u8; 32] {
        *ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([seed; 32])).pk
    }
    AdmissionTrust {
        sequence: 7,
        cluster_id: trust::cluster_id(b"preflight-tests").unwrap(),
        artifact_key: signing_key(20),
        deployment_key: signing_key(21),
        operations_key: signing_key(22),
        // Publicly known, deterministic test material; never a provisioned key.
        recipient_key: charlotte_launch::operations::recipient_public_key(&[77; 32]).unwrap(),
    }
}

fn validate(trust: AdmissionTrust) -> Result<ProductionTrustCandidate, ProductionTrustError> {
    ProductionTrustCandidate::validate(trust, &production_fixture().cluster_id, 1)
}

#[test]
fn production_candidate_round_trips_the_existing_public_wire_format() {
    let trust = production_fixture();
    let candidate = validate(trust).unwrap();
    assert_eq!(candidate.public(), &trust);
    assert_eq!(candidate.encode(), trust.encode().unwrap());
    assert_eq!(
        ProductionTrustCandidate::decode(&candidate.encode(), &trust.cluster_id, 7),
        Ok(candidate)
    );
    // A candidate carries public fields only. It does not modify development
    // parsing or authorize a kernel launch.
    let development = charlotte_launch::development_admission_trust(b"preflight-tests").unwrap();
    assert!(AdmissionTrust::decode(&development.encode().unwrap()).is_some());
    assert!(validate(development).is_err());
}

#[test]
fn production_candidate_requires_cluster_context_and_a_nonzero_revision_floor() {
    use ProductionTrustError::*;
    let mut trust = production_fixture();
    assert_eq!(
        ProductionTrustCandidate::validate(trust, &trust.cluster_id, 0),
        Err(InvalidRevisionFloor)
    );
    assert_eq!(ProductionTrustCandidate::validate(trust, &[9; 32], 1), Err(ClusterMismatch));
    assert_eq!(
        ProductionTrustCandidate::validate(trust, &trust.cluster_id, 8),
        Err(RevisionBelowFloor)
    );
    trust.sequence = u64::MAX;
    assert!(ProductionTrustCandidate::validate(trust, &trust.cluster_id, u64::MAX).is_ok());
    let bytes = trust.encode().unwrap();
    assert_eq!(
        ProductionTrustCandidate::decode(&bytes[..bytes.len() - 1], &trust.cluster_id, 1),
        Err(MalformedPolicy)
    );
    let mut bytes = bytes;
    bytes[12] = 1;
    assert_eq!(
        ProductionTrustCandidate::decode(&bytes, &trust.cluster_id, 1),
        Err(MalformedPolicy)
    );
    trust.sequence = 0;
    assert_eq!(validate(trust), Err(MalformedPolicy));
}

#[test]
fn production_candidate_rejects_every_development_fixture_in_every_role() {
    use charlotte_launch::{
        CLUSTER_PUBLIC_KEY,
        DEVELOPMENT_OPERATIONS_PUBLIC_KEY,
        DEVELOPMENT_RECIPIENT_PUBLIC_KEY,
    };
    for fixture in
        [CLUSTER_PUBLIC_KEY, DEVELOPMENT_OPERATIONS_PUBLIC_KEY, DEVELOPMENT_RECIPIENT_PUBLIC_KEY]
    {
        for role in 0..4 {
            let mut trust = production_fixture();
            match role {
                0 => trust.artifact_key = fixture,
                1 => trust.deployment_key = fixture,
                2 => trust.operations_key = fixture,
                _ => trust.recipient_key = fixture,
            }
            assert_eq!(validate(trust), Err(ProductionTrustError::DevelopmentKey));
        }
    }
    for fixture in [CLUSTER_PUBLIC_KEY, DEVELOPMENT_OPERATIONS_PUBLIC_KEY] {
        let mut trust = production_fixture();
        trust.artifact_key = fixture;
        trust.artifact_key[31] ^= 0x80;
        assert_eq!(validate(trust), Err(ProductionTrustError::DevelopmentKey));
        let mut trust = production_fixture();
        trust.recipient_key = curve25519_dalek::edwards::CompressedEdwardsY(fixture)
            .decompress()
            .unwrap()
            .to_montgomery()
            .to_bytes();
        assert_eq!(validate(trust), Err(ProductionTrustError::DevelopmentKey));
    }
}

#[test]
fn production_candidate_rejects_shared_signing_roles_and_converted_recipient_material() {
    let mut trust = production_fixture();
    trust.deployment_key = trust.artifact_key;
    assert_eq!(validate(trust), Err(ProductionTrustError::SharedRoleKey));
    // Negation changes the Ed25519 byte string, but keeps the same Montgomery
    // identity. It does not constitute an independent compromise domain.
    trust.deployment_key[31] ^= 0x80;
    assert_eq!(validate(trust), Err(ProductionTrustError::SharedRoleKey));
    for signing in [
        production_fixture().artifact_key,
        production_fixture().deployment_key,
        production_fixture().operations_key,
    ] {
        let mut trust = production_fixture();
        trust.recipient_key = curve25519_dalek::edwards::CompressedEdwardsY(signing)
            .decompress()
            .unwrap()
            .to_montgomery()
            .to_bytes();
        assert_eq!(validate(trust), Err(ProductionTrustError::SharedRoleKey));
    }
}

#[test]
fn production_candidate_rejects_invalid_and_non_prime_order_signing_points() {
    use curve25519_dalek::constants::{
        ED25519_BASEPOINT_POINT,
        EIGHT_TORSION,
    };
    let mixed_order = (ED25519_BASEPOINT_POINT + EIGHT_TORSION[1]).compress().to_bytes();
    for point in [
        [0xff; 32],
        mixed_order,
        EIGHT_TORSION[0].compress().to_bytes(),
        EIGHT_TORSION[1].compress().to_bytes(),
    ] {
        for role in 0..3 {
            let mut trust = production_fixture();
            match role {
                0 => trust.artifact_key = point,
                1 => trust.deployment_key = point,
                _ => trust.operations_key = point,
            }
            assert_eq!(validate(trust), Err(ProductionTrustError::InvalidSigningKey));
        }
    }
}

#[test]
fn production_candidate_rejects_non_contributory_and_aliased_recipients() {
    let mut modulus = [0xff; 32];
    modulus[0] = 0xed;
    modulus[31] = 0x7f;
    let mut identity = [0; 32];
    identity[0] = 1;
    let mut alias = production_fixture().recipient_key;
    alias[31] |= 0x80;
    for recipient in [identity, modulus, alias, [0xff; 32]] {
        let mut trust = production_fixture();
        trust.recipient_key = recipient;
        assert_eq!(validate(trust), Err(ProductionTrustError::InvalidRecipientKey));
    }
    for point in curve25519_dalek::constants::EIGHT_TORSION {
        let mut trust = production_fixture();
        trust.recipient_key = point.to_montgomery().to_bytes();
        assert!(validate(trust).is_err());
    }
}

#[test]
fn role_aware_trust_round_trips() {
    let trust = fixture();
    let bytes = trust.encode().unwrap();
    assert_eq!(AdmissionTrust::decode(&bytes), Some(trust));
    assert_eq!(trust::cluster_id(b"orders"), trust::cluster_id(b"orders"));
    assert_ne!(trust::cluster_id(b"orders"), trust::cluster_id(b"payments"));
}

#[test]
fn trust_rejects_zero_and_cross_domain_key_reuse() {
    let mut trust = fixture();
    trust.sequence = 0;
    assert!(trust.encode().is_none());

    let mut trust = fixture();
    trust.operations_key = trust.deployment_key;
    assert!(trust.encode().is_none());

    let mut trust = fixture();
    trust.recipient_key = trust.artifact_key;
    assert!(trust.encode().is_none());

    let mut bytes = fixture().encode().unwrap();
    bytes[12] = 1;
    assert!(AdmissionTrust::decode(&bytes).is_none());
}
