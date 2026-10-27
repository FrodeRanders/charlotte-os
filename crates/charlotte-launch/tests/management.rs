use charlotte_launch::{
    management::{
        self,
        Enrollment,
        Error,
        Limits,
        Profile,
        ProfileInput,
        Role,
        sealed,
    },
    operations,
    trust::AdmissionTrust,
};
use ed25519_compact::KeyPair;
use rand_core::UnwrapErr;
use zeroize::Zeroizing;

fn profile() -> Zeroizing<Vec<u8>> {
    let input = ProfileInput {
        limits: Limits::DEFAULT,
        server_chain: &[b"public certificate"],
        server_key: b"public fixture private-key encoding",
        client_roots: &[b"public CA certificate"],
        operators: &[
            Enrollment {
                certificate_sha256: [1; 32],
                role: Role::Reader,
            },
            Enrollment {
                certificate_sha256: [2; 32],
                role: Role::Administrator,
            },
        ],
    };
    let mut bytes = Zeroizing::new(vec![0; management::encoded_len(&input).unwrap()]);
    management::encode(&input, &mut bytes).unwrap();
    bytes
}

fn trust() -> (AdmissionTrust, KeyPair, Zeroizing<[u8; 32]>) {
    let operations = KeyPair::from_seed([19; 32].into());
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (private, recipient_key) = operations::generate_recipient_keypair(&mut rng);
    let trust = AdmissionTrust {
        sequence: 7,
        cluster_id: [11; 32],
        artifact_key: [12; 32],
        deployment_key: [13; 32],
        operations_key: operations.pk.as_ref().try_into().unwrap(),
        recipient_key,
    };
    assert!(trust.is_valid());
    (trust, operations, private)
}

fn sign(bytes: &mut [u8], key: &KeyPair) {
    let signature = key.sk.sign(sealed::signature_digest(bytes).unwrap(), None);
    sealed::set_signature(bytes, signature.as_ref().try_into().unwrap()).unwrap();
}

fn seal(profile: &[u8], trust: &AdmissionTrust, key: &KeyPair) -> Vec<u8> {
    let mut resource = vec![0; sealed::HEADER_LEN + profile.len()];
    sealed::seal_unsigned(profile, trust, &mut UnwrapErr(getrandom::SysRng), &mut resource)
        .unwrap();
    sign(&mut resource, key);
    resource
}

#[test]
fn complete_profile_round_trip_preserves_all_limits_roles_and_bytes() {
    let bytes = profile();
    let decoded = Profile::decode(&bytes).unwrap();
    assert_eq!(decoded.limits, Limits::DEFAULT);
    assert_eq!(decoded.server_chain().collect::<Vec<_>>(), vec![&b"public certificate"[..]]);
    assert_eq!(decoded.client_roots().collect::<Vec<_>>(), vec![&b"public CA certificate"[..]]);
    assert_eq!(decoded.server_key(), b"public fixture private-key encoding");
    assert_eq!(
        decoded.operators().map(Result::unwrap).map(|entry| entry.role).collect::<Vec<_>>(),
        [Role::Reader, Role::Administrator]
    );
}

#[test]
fn every_truncation_extension_retired_tag_and_reserved_field_rejects() {
    let original = profile();
    for length in 0..original.len() {
        assert!(Profile::decode(&original[..length]).is_err(), "prefix {length}");
    }
    let mut extended = original.to_vec();
    extended.push(0);
    assert!(Profile::decode(&extended).is_err());
    for offset in [0, 8, 10, 12, 116, 127] {
        let mut bytes = original.to_vec();
        bytes[offset] ^= 1;
        assert!(Profile::decode(&bytes).is_err(), "offset {offset}");
    }
}

#[test]
fn missing_overflowing_counts_lengths_and_limits_never_decode_a_prefix() {
    let original = profile();
    for offset in [16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 106, 108, 112, 128] {
        let width = if offset == 104 || offset == 106 {
            2
        } else if offset >= 108 {
            4
        } else {
            8
        };
        for value in if offset < 104 {
            &[0][..]
        } else {
            &[0, 255][..]
        } {
            let mut bytes = original.to_vec();
            bytes[offset..offset + width].fill(*value);
            assert!(Profile::decode(&bytes).is_err(), "offset {offset} value {value}");
        }
    }
}

#[test]
fn unknown_zero_duplicate_and_conflicting_enrollments_reject() {
    let mut bytes = profile();
    let start = management::HEADER_LEN
        + 4
        + b"public certificate".len()
        + 4
        + b"public CA certificate".len();
    for role in [0, 3, 255] {
        bytes[start + 32] = role;
        assert!(Profile::decode(&bytes).is_err());
    }
    bytes = profile();
    bytes[start + 33] = 1;
    assert!(Profile::decode(&bytes).is_err());
    bytes = profile();
    bytes[start..start + 32].fill(0);
    assert!(matches!(Profile::decode(&bytes), Err(Error::InvalidEnrollment)));
    bytes = profile();
    bytes[start + 40..start + 72].fill(1);
    assert!(matches!(Profile::decode(&bytes), Err(Error::DuplicateEnrollment)));
}

#[test]
fn encoder_rejection_writes_nothing_and_capacity_is_shared() {
    let input = ProfileInput {
        limits: Limits::DEFAULT,
        server_chain: &[b"cert"],
        server_key: b"key",
        client_roots: &[b"root"],
        operators: &[],
    };
    let mut output = [0xab; 1024];
    assert!(management::encode(&input, &mut output).is_err());
    assert_eq!(output, [0xab; 1024]);
    assert_eq!(sealed::MAX_RESOURCE_BYTES, sealed::HEADER_LEN + management::MAX_PROFILE_BYTES);
}

#[test]
fn signed_hpke_resource_authenticates_exact_policy_and_retains_erasing_profile() {
    let profile = profile();
    let (trust, key, private) = trust();
    let resource = seal(&profile, &trust, &key);
    assert!(
        !resource
            .windows(b"fixture private-key".len())
            .any(|bytes| bytes == b"fixture private-key")
    );
    sealed::verify(&resource, &trust).unwrap();
    let prepared = sealed::open(&resource, &trust, &private).unwrap();
    assert_eq!(prepared.bytes(), &*profile);
    let mut rotated = trust;
    rotated.sequence += 1;
    assert_eq!(sealed::verify(&resource, &rotated), Err(sealed::Error::WrongPolicy));
    for role in 0..5 {
        let mut foreign = trust;
        match role {
            0 => foreign.cluster_id[0] ^= 1,
            1 => foreign.artifact_key[0] ^= 1,
            2 => foreign.deployment_key[0] ^= 1,
            3 => foreign.operations_key[0] ^= 1,
            _ => foreign.recipient_key[0] ^= 1,
        }
        assert!(matches!(
            sealed::open(&resource, &foreign, &private),
            Err(sealed::Error::WrongPolicy)
        ));
    }
    assert!(matches!(
        sealed::open(&resource, &trust, &[42; 32]),
        Err(sealed::Error::WrongRecipient)
    ));
}

#[test]
fn unsigned_wrong_signature_truncation_extension_and_changed_payload_reject() {
    let profile = profile();
    let (trust, key, private) = trust();
    let resource = seal(&profile, &trust, &key);
    for length in 0..resource.len() {
        assert!(sealed::verify(&resource[..length], &trust).is_err());
    }
    let mut extended = resource.clone();
    extended.push(0);
    assert!(sealed::verify(&extended, &trust).is_err());
    for offset in
        [0, 8, 10, 12, 16, 48, 80, 84, 86, 96, 112, sealed::HEADER_LEN, resource.len() - 1]
    {
        let mut bytes = resource.clone();
        bytes[offset] ^= 1;
        assert!(sealed::open(&bytes, &trust, &private).is_err(), "offset {offset}");
    }
    let mut unsigned = resource.clone();
    unsigned[112..sealed::HEADER_LEN].fill(0);
    assert_eq!(sealed::verify(&unsigned, &trust), Err(sealed::Error::Signature));
    sign(&mut unsigned, &KeyPair::from_seed([20; 32].into()));
    assert_eq!(sealed::verify(&unsigned, &trust), Err(sealed::Error::Signature));
}

#[test]
fn correctly_resigned_ciphertext_cannot_bypass_hpke_authentication() {
    let profile = profile();
    let (trust, key, private) = trust();
    let mut resource = seal(&profile, &trust, &key);
    resource[sealed::HEADER_LEN] ^= 1;
    sign(&mut resource, &key);
    sealed::verify(&resource, &trust).unwrap();
    assert!(matches!(
        sealed::open(&resource, &trust, &private),
        Err(sealed::Error::Authentication)
    ));
}
