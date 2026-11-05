//! Run the kernel's actual profile validator on a scheduled verifier stack.
use alloc::vec::Vec;

use charlotte_protocol_kafka::{
    ALL_RIGHTS,
    Authentication,
    AuthorityEndpoint,
    BrokerEndpoint,
    MAX_AUTHORITY_ENDPOINTS,
    MAX_BROKER_ENDPOINTS,
    MAX_PRODUCE_ROUTES,
    MAX_PROFILE_BYTES,
    PROFILE_DIGEST_LEN,
    PROFILE_DIGEST_OFFSET,
    ProduceRoute,
    Profile,
    ProfileError,
};

pub(super) fn run() {
    let count = MAX_PRODUCE_ROUTES.max(MAX_AUTHORITY_ENDPOINTS).max(MAX_BROKER_ENDPOINTS);
    let names: Vec<_> =
        (0..count).map(|i| [b'a' + (i / 26) as u8, b'a' + (i % 26) as u8]).collect();
    let mut profile = Profile {
        instance_name: b"validator-fixture",
        authority_endpoints: names[..MAX_AUTHORITY_ENDPOINTS]
            .iter()
            .map(|name| AuthorityEndpoint {
                service_name: name,
                rights: ALL_RIGHTS,
            })
            .collect(),
        endpoint_ipv4: [10, 0, 2, 2],
        host: b"kafka.test",
        port: 9093,
        broker_endpoints: names[..MAX_BROKER_ENDPOINTS - 1]
            .iter()
            .map(|name| BrokerEndpoint {
                endpoint_ipv4: [10, 0, 2, 2],
                host: name,
                port: 9093,
            })
            .collect(),
        tls: false,
        ca_certificate_der: &[],
        topic: b"events",
        partition: 0,
        produce_routes: names[..MAX_PRODUCE_ROUTES]
            .iter()
            .map(|name| ProduceRoute {
                topic: name,
                partition: 1,
            })
            .collect(),
        max_produce_routes: MAX_PRODUCE_ROUTES as u16,
        group: b"workers",
        transactional_id: b"validator-fixture",
        authentication: Authentication::None,
        rights: ALL_RIGHTS,
        transaction_timeout_ms: 60_000,
    };
    let initial = profile.encode().unwrap();
    let group = alloc::vec![b'g';MAX_PROFILE_BYTES-initial.len()+profile.group.len()];
    profile.group = &group;
    let mut encoded = profile.encode().unwrap();
    assert_eq!(encoded.len(), MAX_PROFILE_BYTES);
    assert_eq!(Profile::validate_encoded(&encoded), Ok(()));
    encoded[MAX_PROFILE_BYTES - 1] ^= 1;
    assert_eq!(Profile::validate_encoded(&encoded), Err(ProfileError::DigestMismatch));
    encoded[MAX_PROFILE_BYTES - 1] ^= 1;
    encoded[72..74].copy_from_slice(&((MAX_PRODUCE_ROUTES + 1) as u16).to_le_bytes());
    encoded[PROFILE_DIGEST_OFFSET..PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN].fill(0);
    let digest = charlotte_launch::sha256::digest(&encoded);
    encoded[PROFILE_DIGEST_OFFSET..PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN]
        .copy_from_slice(&digest);
    assert_eq!(Profile::validate_encoded(&encoded), Err(ProfileError::TooManyRoutes));
    crate::logln!(
        "[profile validation] scheduled kernel stack accepted {} bytes, {} routes/{} additional \
         brokers/{} authorities; tamper and authenticated over-count rejected",
        MAX_PROFILE_BYTES,
        MAX_PRODUCE_ROUTES,
        MAX_BROKER_ENDPOINTS - 1,
        MAX_AUTHORITY_ENDPOINTS
    );
}
