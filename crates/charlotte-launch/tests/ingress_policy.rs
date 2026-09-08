use charlotte_launch::{
    ingress,
    ingress_policy,
};
use ed25519_compact::{
    KeyPair,
    Signature,
};

fn signed_policy(
    pair: &KeyPair,
    sequence: u64,
    assignments: &[ingress::ServiceBinding<'_>],
) -> Vec<u8> {
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let fields = ingress_policy::PolicyFields {
        sequence,
        not_before_unix_seconds: 1_800_000_000,
        expires_unix_seconds: 1_800_003_600,
        cluster_id: [0x42; 32],
        assignments,
    };
    let mut bytes = vec![0; ingress_policy::encoded_len(&fields).unwrap()];
    ingress_policy::encode_unsigned(&fields, public, &mut bytes).unwrap();
    let signature: Signature =
        pair.sk.sign(ingress_policy::signature_digest(&bytes).unwrap(), None);
    assert!(ingress_policy::set_signature(&mut bytes, signature.as_ref().try_into().unwrap()));
    bytes
}

#[test]
fn signed_policy_round_trips_and_binds_cluster() {
    let pair = KeyPair::from_seed([0x31; 32].into());
    let assignments = [
        ingress::ServiceBinding {
            service: ingress::ServiceId::tcp_v4([10, 0, 2, 42], 443),
            backend_name: Some(b"orders"),
        },
        ingress::ServiceBinding {
            service: ingress::ServiceId::tcp_v4([10, 0, 2, 43], 443),
            backend_name: Some(b"payments"),
        },
    ];
    let bytes = signed_policy(&pair, 7, &assignments);
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    assert_eq!(
        ingress_policy::verify(&bytes, &[0x42; 32], public),
        ingress_policy::VerifyOutcome::Valid
    );
    assert_eq!(
        ingress_policy::verify(&bytes, &[0x43; 32], public),
        ingress_policy::VerifyOutcome::WrongCluster
    );
    let other = KeyPair::from_seed([0x41; 32].into());
    assert_eq!(
        ingress_policy::verify(&bytes, &[0x42; 32], other.pk.as_ref().try_into().unwrap()),
        ingress_policy::VerifyOutcome::WrongKey
    );
    let decoded = ingress_policy::decode(&bytes).unwrap();
    assert_eq!(decoded.sequence, 7);
    assert_eq!(decoded.assignments().collect::<Vec<_>>(), assignments);
}

#[test]
fn empty_policy_is_an_authenticated_withdrawal() {
    let pair = KeyPair::from_seed([0x32; 32].into());
    let bytes = signed_policy(&pair, 8, &[]);
    assert_eq!(ingress_policy::decode(&bytes).unwrap().assignments().count(), 0);
}

#[test]
fn mutation_invalidates_signature() {
    let pair = KeyPair::from_seed([0x33; 32].into());
    let assignment = [ingress::ServiceBinding {
        service: ingress::ServiceId::tcp_v4([10, 0, 2, 44], 80),
        backend_name: None,
    }];
    let mut bytes = signed_policy(&pair, 9, &assignment);
    *bytes.last_mut().unwrap() ^= 1;
    let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    assert_eq!(
        ingress_policy::verify(&bytes, &[0x42; 32], public),
        ingress_policy::VerifyOutcome::Invalid
    );
}
