use charlotte_launch::operations::{
    self,
    EnvelopeFields,
    OpenError,
    PROFILE_KIND_KAFKA,
    VerifyOutcome,
};
use ed25519_compact::{
    KeyPair,
    Signature,
};
use rand_core::UnwrapErr;

fn sign(envelope: &mut [u8], pair: &KeyPair) {
    let digest = operations::signature_digest(envelope).expect("valid unsigned envelope");
    let signature: Signature = pair.sk.sign(digest, None);
    let signature: &[u8; operations::SIGNATURE_LEN] =
        signature.as_ref().try_into().expect("Ed25519 signature length");
    assert!(operations::set_signature(envelope, signature));
}

struct Fixture {
    envelope: Vec<u8>,
    recipient_private: [u8; 32],
    operational: KeyPair,
    cluster_id: [u8; 32],
    release_digest: [u8; 32],
    plaintext: Vec<u8>,
}

fn fixture() -> Fixture {
    let operational = KeyPair::from_seed([0x42; 32].into());
    let operational_public: &[u8; 32] = operational.pk.as_ref().try_into().unwrap();
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (recipient_private, recipient_public) = operations::generate_recipient_keypair(&mut rng);
    let cluster_id = [0x11; 32];
    let release_digest = [0x22; 32];
    let plaintext = b"kafka endpoint=broker.internal password=not-for-applications".to_vec();
    let fields = EnvelopeFields {
        sequence: 17,
        expires_unix_seconds: 2_000_000_000,
        profile_kind: PROFILE_KIND_KAFKA,
        cluster_id,
        release_digest,
        profile_name: b"kafka/orders/transactional",
    };
    let len = operations::encoded_len(&fields, plaintext.len()).unwrap();
    let mut envelope = vec![0u8; len];
    operations::seal_unsigned(
        &fields,
        &plaintext,
        &recipient_public,
        operational_public,
        &mut rng,
        &mut envelope,
    )
    .unwrap();
    sign(&mut envelope, &operational);
    Fixture {
        envelope,
        recipient_private,
        operational,
        cluster_id,
        release_digest,
        plaintext,
    }
}

#[test]
fn encrypted_profile_round_trip_binds_context_and_hides_plaintext() {
    let fixture = fixture();
    assert!(
        !fixture
            .envelope
            .windows(fixture.plaintext.len())
            .any(|window| window == fixture.plaintext)
    );
    let operational_public: &[u8; 32] = fixture.operational.pk.as_ref().try_into().unwrap();
    assert_eq!(operations::verify(&fixture.envelope, operational_public), VerifyOutcome::Valid);
    let decoded = operations::decode(&fixture.envelope).unwrap();
    assert_eq!(decoded.sequence, 17);
    assert_eq!(decoded.profile_name, b"kafka/orders/transactional");
    assert_eq!(decoded.profile_kind, PROFILE_KIND_KAFKA);

    let mut plaintext = vec![0u8; operations::MAX_PROFILE_LEN];
    let len = operations::open(
        &fixture.envelope,
        &fixture.recipient_private,
        operational_public,
        &fixture.cluster_id,
        &fixture.release_digest,
        1_900_000_000,
        &mut plaintext,
    )
    .unwrap();
    assert_eq!(&plaintext[..len], fixture.plaintext);
}

#[test]
fn envelope_rejects_tampering_even_when_resigned_without_hpke_key() {
    let mut fixture = fixture();
    let operational_public: &[u8; 32] = fixture.operational.pk.as_ref().try_into().unwrap();
    let ciphertext_offset = fixture.envelope.len() - fixture.plaintext.len();
    fixture.envelope[ciphertext_offset] ^= 0x80;
    assert_eq!(operations::verify(&fixture.envelope, operational_public), VerifyOutcome::Invalid);
    sign(&mut fixture.envelope, &fixture.operational);
    assert_eq!(operations::verify(&fixture.envelope, operational_public), VerifyOutcome::Valid);
    let mut output = vec![0xa5; fixture.plaintext.len()];
    assert_eq!(
        operations::open(
            &fixture.envelope,
            &fixture.recipient_private,
            operational_public,
            &fixture.cluster_id,
            &fixture.release_digest,
            1_900_000_000,
            &mut output,
        ),
        Err(OpenError::Authentication)
    );
    assert!(output.iter().all(|byte| *byte == 0));
}

#[test]
fn envelope_rejects_wrong_recipient_context_and_expiry() {
    let fixture = fixture();
    let operational_public: &[u8; 32] = fixture.operational.pk.as_ref().try_into().unwrap();
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (wrong_private, _) = operations::generate_recipient_keypair(&mut rng);
    let mut output = vec![0u8; fixture.plaintext.len()];

    assert_eq!(
        operations::open(
            &fixture.envelope,
            &wrong_private,
            operational_public,
            &fixture.cluster_id,
            &fixture.release_digest,
            1_900_000_000,
            &mut output,
        ),
        Err(OpenError::WrongRecipient)
    );
    assert_eq!(
        operations::open(
            &fixture.envelope,
            &fixture.recipient_private,
            operational_public,
            &[0x33; 32],
            &fixture.release_digest,
            1_900_000_000,
            &mut output,
        ),
        Err(OpenError::WrongCluster)
    );
    assert_eq!(
        operations::open(
            &fixture.envelope,
            &fixture.recipient_private,
            operational_public,
            &fixture.cluster_id,
            &[0x44; 32],
            1_900_000_000,
            &mut output,
        ),
        Err(OpenError::WrongRelease)
    );
    assert_eq!(
        operations::open(
            &fixture.envelope,
            &fixture.recipient_private,
            operational_public,
            &fixture.cluster_id,
            &fixture.release_digest,
            2_000_000_001,
            &mut output,
        ),
        Err(OpenError::Expired)
    );
}

#[test]
fn profile_bounds_are_independent_of_ciphertext_fields() {
    let mut fields = EnvelopeFields {
        sequence: 1,
        expires_unix_seconds: 2,
        profile_kind: PROFILE_KIND_KAFKA,
        cluster_id: [1; 32],
        release_digest: [2; 32],
        profile_name: b"kafka/example",
    };
    assert!(operations::encoded_len(&fields, 0).is_err());
    assert!(operations::encoded_len(&fields, operations::MAX_PROFILE_LEN + 1).is_err());
    fields.cluster_id = [0; 32];
    assert!(operations::encoded_len(&fields, 1).is_err());
    fields.cluster_id = [1; 32];
    fields.release_digest = [0; 32];
    assert!(operations::encoded_len(&fields, 1).is_err());
    assert!(operations::decode(&[0u8; operations::HEADER_LEN]).is_none());
}
