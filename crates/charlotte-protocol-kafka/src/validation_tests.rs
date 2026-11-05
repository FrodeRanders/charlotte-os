extern crate std;

use core::cell::Cell;
use std::alloc::{
    GlobalAlloc,
    Layout,
    System,
};

use super::*;

std::thread_local! { static TRACE: Cell<(bool, usize)> = const { Cell::new((false, 0)) }; }
struct Tracer;
#[global_allocator]
static ALLOCATOR: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACE.try_with(|trace| {
            let (active, count) = trace.get();
            if active {
                trace.set((true, count + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

fn measured<T>(f: impl FnOnce() -> T) -> (T, usize) {
    TRACE.with(|trace| trace.set((true, 0)));
    let result = f();
    let count = TRACE.with(|trace| {
        let (_, count) = trace.get();
        trace.set((false, 0));
        count
    });
    (result, count)
}

#[test]
fn maximum_profile_validates_without_heap_and_old_owned_decoder_allocates() {
    let topics: Vec<_> = (0..MAX_PRODUCE_ROUTES).map(|i| alloc::format!("topic-{i}")).collect();
    let hosts: Vec<_> =
        (0..MAX_BROKER_ENDPOINTS - 1).map(|i| alloc::format!("broker-{i}.test")).collect();
    let names: Vec<_> =
        (0..MAX_AUTHORITY_ENDPOINTS).map(|i| alloc::format!("endpoint-{i}")).collect();
    let mut profile = tests::profile_without_authentication();
    profile.produce_routes = topics
        .iter()
        .map(|topic| ProduceRoute {
            topic: topic.as_bytes(),
            partition: 1,
        })
        .collect();
    profile.broker_endpoints = hosts
        .iter()
        .map(|host| BrokerEndpoint {
            endpoint_ipv4: [10, 0, 2, 2],
            host: host.as_bytes(),
            port: 9093,
        })
        .collect();
    profile.authority_endpoints = names
        .iter()
        .map(|name| AuthorityEndpoint {
            service_name: name.as_bytes(),
            rights: ALL_RIGHTS,
        })
        .collect();
    let first = profile.encode().unwrap();
    let group = alloc::vec![b'g'; MAX_PROFILE_BYTES-first.len()+profile.group.len()];
    profile.group = &group;
    let encoded = profile.encode().unwrap();
    assert_eq!(encoded.len(), MAX_PROFILE_BYTES);
    assert_eq!(measured(|| Profile::validate_encoded(&encoded)), (Ok(()), 0));
    let (decoded, allocations) = measured(|| Profile::decode(&encoded));
    assert!(decoded.is_ok());
    assert_eq!(allocations, 3, "old kernel validity check must exercise real vector allocation");
    let decoded = decoded.unwrap();
    assert_eq!(decoded.produce_routes.len(), MAX_PRODUCE_ROUTES);
    assert_eq!(decoded.broker_endpoints.len(), MAX_BROKER_ENDPOINTS - 1);
    assert_eq!(decoded.authority_endpoints.len(), MAX_AUTHORITY_ENDPOINTS);
    assert!(decoded == profile);
    std::println!(
        "Kafka validator: {}-byte frame, 64 routes/31 additional brokers/64 authorities; \
         scratch={} bytes; validate allocations=0; owned decoder allocations=3",
        encoded.len(),
        core::mem::size_of::<ProfileScratch<'static>>()
    );
}

#[test]
fn validation_and_owned_decode_share_complete_acceptance_and_zero_heap_rejection() {
    let mut profile = tests::profile_without_authentication();
    for authentication in [
        Authentication::None,
        Authentication::ScramSha256 {
            username: b"user",
            password: b"password",
        },
        Authentication::MtlsP256 {
            certificate_der: b"certificate",
            private_key_der: b"private-key",
        },
        Authentication::ScramSha256AndMtlsP256 {
            username: b"user",
            password: b"password",
            certificate_der: b"certificate",
            private_key_der: b"private-key",
        },
    ] {
        profile.authentication = authentication;
        profile.tls = authentication != Authentication::None;
        profile.ca_certificate_der = if profile.tls {
            b"CA"
        } else {
            b""
        };
        let encoded = profile.encode().unwrap();
        assert_eq!(measured(|| Profile::validate_encoded(&encoded)), (Ok(()), 0));
        for index in 0..encoded.len() {
            let mut changed = encoded.to_vec();
            changed[index] ^= 1;
            // Check both digest rejection and fully authenticated malformed fields.
            for refresh in [false, true] {
                if refresh {
                    tests::refresh_profile_digest(&mut changed);
                }
                let expected = Profile::decode(&changed).map(|_| ());
                assert_eq!(measured(|| Profile::validate_encoded(&changed)), (expected, 0));
            }
        }
        for len in 0..encoded.len() {
            let expected = Profile::decode(&encoded[..len]).map(|_| ());
            assert_eq!(measured(|| Profile::validate_encoded(&encoded[..len])), (expected, 0));
        }
    }
}
