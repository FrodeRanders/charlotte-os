use charlotte_launch::ingress::{
    self,
    ServiceBinding,
    ServiceId,
};

#[test]
fn round_trip_multiple_service_assignments() {
    let bindings = [
        ServiceBinding {
            service: ServiceId::tcp_v4([10, 0, 2, 42], 443),
            backend_name: Some(b"orders"),
        },
        ServiceBinding {
            service: ServiceId::tcp_v4([10, 0, 2, 43], 443),
            backend_name: Some(b"payments"),
        },
        ServiceBinding {
            service: ServiceId::tcp_v4([10, 0, 2, 44], 8080),
            backend_name: None,
        },
    ];
    let mut bytes = [0u8; ingress::MAX_ENCODED_LEN];
    let len = ingress::encode(&bindings, &mut bytes).unwrap();
    let decoded = ingress::decode(&bytes[..len]).unwrap().collect::<Vec<_>>();
    assert_eq!(decoded, bindings);
}

#[test]
fn rejects_duplicate_service_identity() {
    let binding = ServiceBinding {
        service: ServiceId::tcp_v4([10, 0, 2, 42], 443),
        backend_name: Some(b"orders"),
    };
    assert_eq!(
        ingress::encoded_len(&[binding, binding]),
        Err(ingress::EncodeError::DuplicateService)
    );
}

#[test]
fn rejects_malformed_and_non_tcp_records() {
    let binding = ServiceBinding {
        service: ServiceId::tcp_v4([10, 0, 2, 42], 443),
        backend_name: Some(b"orders"),
    };
    let mut bytes = [0u8; ingress::MAX_ENCODED_LEN];
    let len = ingress::encode(&[binding], &mut bytes).unwrap();
    bytes[ingress::HEADER_LEN + 4] = 17;
    assert!(ingress::decode(&bytes[..len]).is_none());
}

#[test]
fn compact_service_identity_round_trips_without_table_position() {
    let service = ServiceId::tcp_v4([10, 0, 2, 42], 443);
    assert_eq!(ServiceId::unpack(service.pack()), Some(service));
    assert!(ServiceId::unpack(service.pack() | (1 << 63)).is_none());
}
