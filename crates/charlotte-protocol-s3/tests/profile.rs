use charlotte_protocol_s3::{
    PROFILE_FLAG_TLS,
    Profile,
};

fn profile<'a>() -> Profile<'a> {
    Profile {
        endpoint_ipv4: [192, 0, 2, 10],
        host: b"objects.example.test",
        port: 443,
        tls: true,
        ca_certificate_der: b"certificate",
        region: b"eu-north-1",
        bucket: b"charlotte",
        prefix: b"operations/",
        access_key: b"connector-only",
        secret_key: b"not-for-applications",
        namespace: b"s3/reports",
        rights: charlotte_protocol_s3::RIGHT_GET,
    }
}

#[test]
fn immutable_profile_round_trips() {
    let profile = profile();
    let encoded = profile.encode().unwrap();
    assert_eq!(Profile::decode(&encoded), Some(profile));
}

#[test]
fn immutable_profile_rejects_unknown_flags_and_padding() {
    let mut encoded = profile().encode().unwrap();
    encoded[16..18].copy_from_slice(&(PROFILE_FLAG_TLS | 0x8000).to_le_bytes());
    assert_eq!(Profile::decode(&encoded), None);

    let mut encoded = profile().encode().unwrap();
    encoded[52] = 1;
    assert_eq!(Profile::decode(&encoded), None);
}

#[test]
fn tls_profile_requires_a_trust_anchor() {
    let mut profile = profile();
    profile.ca_certificate_der = b"";
    assert_eq!(profile.encode(), None);
}

#[test]
fn profile_rejects_unknown_rights_and_header_injection() {
    let mut invalid_rights = profile();
    invalid_rights.rights |= 1 << 63;
    assert_eq!(invalid_rights.encode(), None);

    let mut injected_host = profile();
    injected_host.host = b"objects.example.test\r\nx-evil: yes";
    assert_eq!(injected_host.encode(), None);

    let mut injected_access_key = profile();
    injected_access_key.access_key = b"access\nkey";
    assert_eq!(injected_access_key.encode(), None);
}

#[test]
fn profile_rejects_ambiguous_prefixes_and_invalid_region() {
    for prefix in [b"/absolute".as_slice(), b"a/../b", b"./a"] {
        let mut profile = profile();
        profile.prefix = prefix;
        assert_eq!(profile.encode(), None);
    }

    let mut invalid_region = profile();
    invalid_region.region = b"eu north 1";
    assert_eq!(invalid_region.encode(), None);
}
