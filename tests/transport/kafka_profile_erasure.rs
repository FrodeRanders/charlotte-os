//! Kafka launch-profile byte ownership with public credential fixtures.
use core::hint::black_box;

use catten_services::kafka as wire;
use zeroize::Zeroizing;

mod support;
use support::observe;

fn fixture() -> wire::Profile<'static> {
    wire::Profile {
        instance_name: b"ZZZZZZZZ-instance",
        authority_endpoints: vec![wire::AuthorityEndpoint {
            service_name: b"kafka/ZZZZZZZZ",
            rights: wire::ALL_RIGHTS,
        }],
        endpoint_ipv4: [192, 0, 2, 1],
        host: b"kafka-1.test",
        port: 9093,
        broker_endpoints: vec![wire::BrokerEndpoint {
            endpoint_ipv4: [192, 0, 2, 2],
            host: b"kafka-2.test",
            port: 9094,
        }],
        tls: true,
        ca_certificate_der: b"ZZZZZZZZ-ca",
        topic: b"ZZZZZZZZ-events",
        partition: 0,
        produce_routes: vec![wire::ProduceRoute {
            topic: b"ZZZZZZZZ-output",
            partition: 1,
        }],
        max_produce_routes: 1,
        group: b"ZZZZZZZZ-group",
        transactional_id: b"ZZZZZZZZ-transaction",
        authentication: wire::Authentication::ScramSha256AndMtlsP256 {
            username: b"ZZZZZZZZ-user",
            password: b"ZZZZZZZZ-password",
            certificate_der: b"ZZZZZZZZ-certificate",
            private_key_der: b"ZZZZZZZZ-private-key",
        },
        rights: wire::ALL_RIGHTS,
        transaction_timeout_ms: 30_000,
    }
}

fn auth_offset(profile: &wire::Profile<'_>) -> usize {
    wire::PROFILE_HEADER_LEN
        + profile.instance_name.len()
        + profile.host.len()
        + profile.ca_certificate_der.len()
        + profile.topic.len()
        + profile.group.len()
        + profile.transactional_id.len()
}

#[test]
fn encoded_profile_erases_all_field_ranges() {
    let profile = fixture();
    let reference = profile.encode().unwrap();
    let decoded = wire::Profile::decode(&reference).unwrap();
    let wire::Authentication::ScramSha256AndMtlsP256 {
        username,
        password,
        certificate_der,
        private_key_der,
    } = decoded.authentication
    else {
        panic!("fixture authentication changed")
    };
    for field in [
        decoded.instance_name,
        decoded.host,
        decoded.ca_certificate_der,
        decoded.topic,
        decoded.group,
        decoded.transactional_id,
        username,
        password,
        certificate_der,
        private_key_der,
        decoded.produce_routes[0].topic,
        decoded.broker_endpoints[0].host,
        decoded.authority_endpoints[0].service_name,
    ] {
        let offset = field.as_ptr().addr() - reference.as_ptr().addr();
        let observed = observe(0, 0, offset, || {
            let encoded = profile.encode().unwrap();
            assert_eq!(&encoded[offset..offset + field.len()], field);
            drop(black_box(encoded));
        });
        assert!(observed.copy_erased, "encoded field retained: {observed:?}");
        assert_eq!(observed.allocations, 1);
        assert_eq!(observed.growths, 0);
        assert_eq!(observed.copy_releases, 1);
    }
}

#[test]
fn abandoning_encoded_profile_erases_credentials() {
    fn reject_after_encoding(profile: &wire::Profile<'_>) -> Option<()> {
        let _encoded = black_box(profile.encode().ok()?);
        None
    }
    let profile = fixture();
    let wire::Authentication::ScramSha256AndMtlsP256 {
        username,
        ..
    } = profile.authentication
    else {
        unreachable!()
    };
    let password_offset = auth_offset(&profile) + username.len();
    let observed = observe(0, 0, password_offset, || {
        assert!(reject_after_encoding(&profile).is_none());
    });
    assert!(observed.copy_erased, "abandoned credentials retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn encoder_transfers_the_same_erasing_owner() {
    fn consume(encoded: Zeroizing<Vec<u8>>, address: usize) {
        assert_eq!(encoded.as_ptr().addr(), address);
        assert_eq!(&encoded[..8], &wire::PROFILE_MAGIC);
        drop(black_box(encoded));
    }
    let profile = fixture();
    let observed = observe(0, 0, auth_offset(&profile), || {
        let encoded = profile.encode().unwrap();
        let address = encoded.as_ptr().addr();
        consume(encoded, address);
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn all_authentication_choices_round_trip_under_owner() {
    for authentication in [
        wire::Authentication::None,
        wire::Authentication::ScramSha256 {
            username: b"ZZZZZZZZ-user",
            password: b"ZZZZZZZZ-password",
        },
        wire::Authentication::MtlsP256 {
            certificate_der: b"ZZZZZZZZ-certificate",
            private_key_der: b"ZZZZZZZZ-private-key",
        },
        fixture().authentication,
    ] {
        let profile = wire::Profile {
            authentication,
            ..fixture()
        };
        let encoded = profile.encode().unwrap();
        assert!(wire::Profile::decode(&encoded).unwrap() == profile);
    }
}

#[test]
fn invalid_profiles_reject_before_frame_allocation() {
    let mut cases = Vec::new();
    let mut p = fixture();
    p.port = 0;
    cases.push(p);
    let mut p = fixture();
    p.host = b"bad/host";
    cases.push(p);
    let mut p = fixture();
    p.rights = 0;
    cases.push(p);
    let mut p = fixture();
    p.transaction_timeout_ms = 999;
    cases.push(p);
    let mut p = fixture();
    p.authentication = wire::Authentication::ScramSha256 {
        username: b"user",
        password: b"",
    };
    cases.push(p);
    let mut p = fixture();
    p.authentication = wire::Authentication::MtlsP256 {
        certificate_der: b"certificate",
        private_key_der: b"",
    };
    cases.push(p);
    let mut p = fixture();
    p.tls = false;
    p.ca_certificate_der = b"";
    cases.push(p);
    let mut p = fixture();
    p.authority_endpoints.clear();
    cases.push(p);
    let mut p = fixture();
    p.authority_endpoints.push(p.authority_endpoints[0]);
    cases.push(p);
    let mut p = fixture();
    p.broker_endpoints.push(p.broker_endpoints[0]);
    cases.push(p);
    let mut p = fixture();
    p.produce_routes.push(p.produce_routes[0]);
    cases.push(p);
    let mut p = fixture();
    p.max_produce_routes = 0;
    cases.push(p);
    for profile in cases {
        let observed = observe(0, 0, 0, || assert!(profile.encode().is_err()));
        assert_eq!(observed.allocations, 0);
    }
}

#[test]
fn maximum_profile_erases_and_oversize_rejects_without_allocation() {
    let base = fixture();
    let mut group =
        vec![b'Z'; wire::MAX_PROFILE_BYTES - base.encode().unwrap().len() + base.group.len()];
    let profile = wire::Profile {
        group: &group,
        ..fixture()
    };
    let observed = observe(0, 0, auth_offset(&profile), || {
        let encoded = profile.encode().unwrap();
        assert_eq!(encoded.len(), wire::MAX_PROFILE_BYTES);
        drop(black_box(encoded));
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
    drop(profile);
    group.push(b'Z');
    let profile = wire::Profile {
        group: &group,
        ..fixture()
    };
    let observed = observe(0, 0, 0, || {
        assert_eq!(profile.encode(), Err(wire::ProfileError::TooLarge));
    });
    assert_eq!(observed.allocations, 0);
}

#[test]
fn encoder_preserves_independent_wire_fixture() {
    let encoded = fixture().encode().unwrap();
    assert_eq!(encoded.as_slice(), include_bytes!("fixtures/kafka-profile-v7.bin"));
    assert!(wire::Profile::decode(&encoded).unwrap() == fixture());
}
