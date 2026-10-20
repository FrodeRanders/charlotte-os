//! Public profile fixtures exercise only local byte ownership.
use core::hint::black_box;

use catten_services::s3 as wire;
use charlotte_s3::profile::OwnedProfile;
use zeroize::Zeroizing;

mod support;
use support::observe;

fn fixture() -> wire::Profile<'static> {
    wire::Profile {
        endpoint_ipv4: [192, 0, 2, 1],
        host: b"ZZZZZZZZ.local",
        port: 443,
        tls: true,
        ca_certificate_der: b"ZZZZZZZZ-public-ca",
        region: b"us-east-1",
        bucket: b"ZZZZZZZZ-bucket",
        prefix: b"ZZZZZZZZ/path",
        access_key: b"ZZZZZZZZ-access",
        secret_key: b"ZZZZZZZZ-public-secret",
        namespace: b"ZZZZZZZZ-namespace",
        rights: wire::RIGHT_GET | wire::RIGHT_PUT | wire::RIGHT_DELETE,
    }
}

#[test]
fn profile_fields_erase_in_one_snapshot() {
    let bytes = fixture().encode().unwrap();
    let view = wire::Profile::decode(&bytes).unwrap();
    for field in [
        view.host,
        view.region,
        view.bucket,
        view.prefix,
        view.access_key,
        view.secret_key,
        view.namespace,
        view.ca_certificate_der,
    ] {
        let offset = field.as_ptr().addr() - bytes.as_ptr().addr();
        let observed = observe(0, 0, offset, || {
            let profile = OwnedProfile::decode(black_box(&bytes)).unwrap();
            assert_eq!(profile.view().secret_key, fixture().secret_key);
            drop(black_box(profile));
        });
        assert!(observed.copy_erased, "profile field retained: {observed:?}");
        assert_eq!(observed.allocations, 1);
        assert_eq!(observed.growths, 0);
        assert_eq!(observed.copy_releases, 1);
    }
}

#[test]
fn profile_views_borrow_without_allocating() {
    let profile = OwnedProfile::decode(&fixture().encode().unwrap()).unwrap();
    let observed = observe(0, 0, 0, || {
        let view = profile.view();
        assert_eq!(view.ip, fixture().endpoint_ipv4);
        assert_eq!(view.host.as_bytes(), fixture().host);
        assert_eq!(view.port, fixture().port);
        assert!(view.tls);
        assert_eq!(view.ca_der, fixture().ca_certificate_der);
        assert_eq!(view.region.as_bytes(), fixture().region);
        assert_eq!(view.bucket.as_bytes(), fixture().bucket);
        assert_eq!(view.prefix.as_bytes(), fixture().prefix);
        assert_eq!(view.access_key.as_bytes(), fixture().access_key);
        assert_eq!(view.secret_key, fixture().secret_key);
        assert_eq!(view.namespace.unwrap().as_bytes(), fixture().namespace);
        assert_eq!(view.rights, fixture().rights);
        assert!(profile.has(wire::RIGHT_GET));
        assert!(!profile.has(wire::RIGHT_LIST));
    });
    assert_eq!(observed.allocations, 0);
}

#[test]
fn authority_erases_without_growth() {
    for (tls, port, expected) in [
        (true, 443, "ZZZZZZZZ.local"),
        (false, 80, "ZZZZZZZZ.local"),
        (true, 80, "ZZZZZZZZ.local:80"),
        (false, 443, "ZZZZZZZZ.local:443"),
        (false, 8080, "ZZZZZZZZ.local:8080"),
        (true, u16::MAX, "ZZZZZZZZ.local:65535"),
    ] {
        let bytes = wire::Profile {
            tls,
            port,
            ..fixture()
        }
        .encode()
        .unwrap();
        let profile = OwnedProfile::decode(&bytes).unwrap();
        let observed = observe(0, 0, 0, || {
            let authority = profile.view().authority().unwrap();
            assert_eq!(authority.as_str(), expected);
            drop(black_box(authority));
        });
        assert!(observed.copy_erased, "authority retained: {observed:?}");
        assert_eq!(observed.allocations, 1);
        assert_eq!(observed.growths, 0);
        assert_eq!(observed.copy_releases, 1);
    }
}

#[test]
fn invalid_profiles_reject_before_copying() {
    let valid = fixture().encode().unwrap();
    let mut cases =
        vec![Zeroizing::new(Vec::new()), Zeroizing::new(valid[..valid.len() - 1].to_vec())];
    let mut extended = valid.clone();
    extended.push(0);
    cases.push(extended);
    for (offset, replacement) in [
        (6, vec![b'0']),
        (8, 0u16.to_le_bytes().to_vec()),
        (10, 63u16.to_le_bytes().to_vec()),
        (16, 2u16.to_le_bytes().to_vec()),
        (18, 0u16.to_le_bytes().to_vec()),
        (20, 0u64.to_le_bytes().to_vec()),
        (20, (1u64 << 63).to_le_bytes().to_vec()),
        (42, 0u16.to_le_bytes().to_vec()),
        (46, vec![1]),
        (52, vec![1]),
        (wire::PROFILE_HEADER_LEN, vec![b'/']),
    ] {
        let mut bytes = valid.clone();
        bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
        cases.push(bytes);
    }
    let mut invalid_utf8 = valid.clone();
    let prefix = wire::Profile::decode(&valid).unwrap().prefix;
    invalid_utf8[prefix.as_ptr().addr() - valid.as_ptr().addr()] = 0xff;
    cases.push(invalid_utf8);
    for bytes in cases {
        let mut rejected = false;
        let observed = observe(0, 0, 0, || {
            rejected = OwnedProfile::decode(black_box(&bytes)).is_none();
        });
        assert!(rejected);
        assert_eq!(observed.allocations, 0, "invalid profile copied: {observed:?}");
    }
}

#[test]
fn snapshot_survives_original_input_release() {
    let mut bytes = fixture().encode().unwrap();
    let profile = OwnedProfile::decode(&bytes).unwrap();
    bytes.fill(b'X');
    drop(bytes);
    let view = profile.view();
    assert_eq!(view.access_key.as_bytes(), fixture().access_key);
    assert_eq!(view.secret_key, fixture().secret_key);
    assert_eq!(view.namespace.unwrap().as_bytes(), fixture().namespace);
    assert_eq!(view.ca_der, fixture().ca_certificate_der);
}

#[test]
fn empty_namespace_stays_absent() {
    let bytes = wire::Profile {
        namespace: b"",
        ..fixture()
    }
    .encode()
    .unwrap();
    let profile = OwnedProfile::decode(&bytes).unwrap();
    assert!(profile.view().namespace.is_none());
}

#[test]
fn maximum_profile_reserves_without_growth() {
    let ca = vec![
        b'Z';
        wire::MAX_PROFILE_LEN - fixture().encoded_len().unwrap()
            + fixture().ca_certificate_der.len()
    ];
    let bytes = wire::Profile {
        ca_certificate_der: &ca,
        ..fixture()
    }
    .encode()
    .unwrap();
    assert_eq!(bytes.len(), wire::MAX_PROFILE_LEN);
    let observed = observe(0, 0, wire::PROFILE_HEADER_LEN, || {
        let profile = OwnedProfile::decode(&bytes).unwrap();
        assert_eq!(profile.view().ca_der, ca);
        drop(black_box(profile));
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}
