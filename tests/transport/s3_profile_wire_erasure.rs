//! Encoded launch-profile ownership with public credential fixtures.
use core::hint::black_box;

use catten_services::s3 as wire;
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

fn secret_offset(profile: &wire::Profile<'_>) -> usize {
    wire::PROFILE_HEADER_LEN
        + profile.host.len()
        + profile.region.len()
        + profile.bucket.len()
        + profile.prefix.len()
        + profile.access_key.len()
}

#[test]
fn encoded_profile_erases_all_field_ranges() {
    let reference = fixture().encode().unwrap();
    let view = wire::Profile::decode(&reference).unwrap();
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
        let offset = field.as_ptr().addr() - reference.as_ptr().addr();
        let observed = observe(0, 0, offset, || {
            let bytes = fixture().encode().unwrap();
            assert!(wire::Profile::decode(&bytes) == Some(fixture()));
            drop(black_box(bytes));
        });
        assert!(observed.copy_erased, "encoded field retained: {observed:?}");
        assert_eq!(observed.allocations, 1);
        assert_eq!(observed.growths, 0);
        assert_eq!(observed.copy_releases, 1);
    }
}

#[test]
fn abandoning_encoded_profile_erases_credentials() {
    fn reject_after_encoding() -> Option<()> {
        let _encoded = black_box(fixture().encode()?);
        None
    }
    let mut rejected = false;
    let observed = observe(0, 0, secret_offset(&fixture()), || {
        rejected = reject_after_encoding().is_none();
    });
    assert!(rejected);
    assert!(observed.copy_erased, "abandoned profile retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn encoder_transfers_the_same_erasing_owner() {
    fn consume(encoded: Zeroizing<Vec<u8>>, original: usize) {
        assert_eq!(encoded.as_ptr().addr(), original);
        assert!(wire::Profile::decode(&encoded) == Some(fixture()));
        drop(black_box(encoded));
    }
    let observed = observe(0, 0, secret_offset(&fixture()), || {
        let encoded = fixture().encode().unwrap();
        let original = encoded.as_ptr().addr();
        consume(encoded, original);
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn invalid_profile_encoder_rejects_without_allocation() {
    for profile in [
        wire::Profile {
            host: b"",
            ..fixture()
        },
        wire::Profile {
            port: 0,
            ..fixture()
        },
        wire::Profile {
            rights: 0,
            ..fixture()
        },
        wire::Profile {
            rights: 1 << 63,
            ..fixture()
        },
        wire::Profile {
            region: b"bad region",
            ..fixture()
        },
        wire::Profile {
            bucket: b"bad/bucket",
            ..fixture()
        },
        wire::Profile {
            access_key: b"bad\nkey",
            ..fixture()
        },
        wire::Profile {
            secret_key: b"",
            ..fixture()
        },
        wire::Profile {
            namespace: b"bad\rnamespace",
            ..fixture()
        },
        wire::Profile {
            prefix: b"../escape",
            ..fixture()
        },
        wire::Profile {
            ca_certificate_der: b"",
            ..fixture()
        },
    ] {
        let mut rejected = false;
        let observed = observe(0, 0, 0, || {
            rejected = profile.encode().is_none();
        });
        assert!(rejected);
        assert_eq!(observed.allocations, 0);
    }
    let oversized_ca = vec![b'Z'; wire::MAX_PROFILE_LEN];
    let oversized_field = vec![b'Z'; u16::MAX as usize + 1];
    for profile in [
        wire::Profile {
            ca_certificate_der: &oversized_ca,
            ..fixture()
        },
        wire::Profile {
            secret_key: &oversized_field,
            ..fixture()
        },
    ] {
        let observed = observe(0, 0, 0, || assert!(profile.encode().is_none()));
        assert_eq!(observed.allocations, 0);
    }
}

#[test]
fn maximum_encoded_profile_erases_without_growth() {
    let ca = vec![
        b'Z';
        wire::MAX_PROFILE_LEN - fixture().encoded_len().unwrap()
            + fixture().ca_certificate_der.len()
    ];
    let profile = wire::Profile {
        ca_certificate_der: &ca,
        ..fixture()
    };
    let observed = observe(0, 0, secret_offset(&profile), || {
        let encoded = profile.encode().unwrap();
        assert_eq!(encoded.len(), wire::MAX_PROFILE_LEN);
        assert!(wire::Profile::decode(&encoded) == Some(profile));
        drop(black_box(encoded));
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn profile_preserves_fixed_wire_fixture() {
    // Independently generated with Python struct.pack from the public fixture.
    let header = [
        67, 72, 83, 51, 80, 70, 49, 0, 1, 0, 64, 0, 188, 0, 0, 0, 1, 0, 187, 1, 7, 0, 0, 0, 0, 0,
        0, 0, 192, 0, 2, 1, 14, 0, 9, 0, 15, 0, 13, 0, 15, 0, 22, 0, 18, 0, 0, 0, 18, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let fields = concat!(
        "ZZZZZZZZ.local",
        "us-east-1",
        "ZZZZZZZZ-bucket",
        "ZZZZZZZZ/path",
        "ZZZZZZZZ-access",
        "ZZZZZZZZ-public-secret",
        "ZZZZZZZZ-namespace",
        "ZZZZZZZZ-public-ca"
    );
    let encoded = fixture().encode().unwrap();
    assert_eq!(&encoded[..wire::PROFILE_HEADER_LEN], &header);
    assert_eq!(&encoded[wire::PROFILE_HEADER_LEN..], fields.as_bytes());
}
