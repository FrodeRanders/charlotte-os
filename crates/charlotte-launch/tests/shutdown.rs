use charlotte_launch::shutdown::{
    self,
    ShutdownFields,
    VerifyOutcome,
};
use ed25519_compact::KeyPair;

fn fields() -> ShutdownFields {
    ShutdownFields {
        sequence: 7,
        target_node: 0x1234,
        not_before_unix_seconds: 1_788_600_000,
        expires_unix_seconds: 1_788_600_300,
        node_grace_ms: 30_000,
        phase_grace_ms: 2_000,
        reason: shutdown::REASON_POWER_OFF,
    }
}

fn signed_intent(pair: &KeyPair) -> Vec<u8> {
    let mut bytes = vec![0; shutdown::ENCODED_LEN];
    shutdown::encode_unsigned(&fields(), pair.pk.as_ref().try_into().unwrap(), &mut bytes).unwrap();
    let digest = shutdown::signature_digest(&bytes).unwrap();
    let signature = pair.sk.sign(digest, None);
    assert!(shutdown::set_signature(&mut bytes, signature.as_ref().try_into().unwrap()));
    bytes
}

#[test]
fn signed_shutdown_intent_round_trips() {
    let pair = KeyPair::from_seed([9; 32].into());
    let bytes = signed_intent(&pair);
    assert_eq!(shutdown::decode(&bytes), Some(fields()));
    assert_eq!(
        shutdown::verify(&bytes, pair.pk.as_ref().try_into().unwrap()),
        VerifyOutcome::Valid
    );
}

#[test]
fn mutations_and_wrong_keys_are_rejected() {
    let pair = KeyPair::from_seed([9; 32].into());
    let other = KeyPair::from_seed([8; 32].into());
    let mut bytes = signed_intent(&pair);
    bytes[48] ^= 1;
    assert_eq!(
        shutdown::verify(&bytes, pair.pk.as_ref().try_into().unwrap()),
        VerifyOutcome::Invalid
    );

    let bytes = signed_intent(&pair);
    assert_eq!(
        shutdown::verify(&bytes, other.pk.as_ref().try_into().unwrap()),
        VerifyOutcome::WrongKey
    );
}

#[test]
fn policy_bounds_are_enforced() {
    let pair = KeyPair::from_seed([9; 32].into());
    let key = pair.pk.as_ref().try_into().unwrap();
    let mut output = [0; shutdown::ENCODED_LEN];

    let mut invalid = fields();
    invalid.target_node = 0;
    assert_eq!(
        shutdown::encode_unsigned(&invalid, key, &mut output),
        Err(shutdown::EncodeError::InvalidFields)
    );

    invalid = fields();
    invalid.phase_grace_ms = invalid.node_grace_ms + 1;
    assert_eq!(
        shutdown::encode_unsigned(&invalid, key, &mut output),
        Err(shutdown::EncodeError::InvalidFields)
    );

    invalid = fields();
    invalid.expires_unix_seconds =
        invalid.not_before_unix_seconds + shutdown::MAX_VALIDITY_SECONDS + 1;
    assert_eq!(
        shutdown::encode_unsigned(&invalid, key, &mut output),
        Err(shutdown::EncodeError::InvalidFields)
    );
}
