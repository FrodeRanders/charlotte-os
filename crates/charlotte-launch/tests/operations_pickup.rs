use charlotte_launch::operations_pickup::{
    CatalogBinding,
    Pickup,
    catalog_list_encoded_len,
    decode_catalog_list,
    encode_catalog_list,
};

fn binding<'a>(
    profile_name: &'a [u8],
    target: &'a [u8],
    object_key: &'a [u8],
) -> CatalogBinding<'a> {
    CatalogBinding {
        generation: 7,
        bundle_sequence: 9,
        sequence: 11,
        expires_unix_seconds: 2_000_000_000,
        profile_kind: charlotte_launch::operations::PROFILE_KIND_KAFKA,
        release_name: b"orders-v7",
        profile_name,
        target_artifact: target,
        object_key,
        release_digest: [0x11; 32],
        bundle_digest: [0x22; 32],
        envelope_digest: [0x33; 32],
        recipient_key_id: [0x44; 16],
        signing_key_id: [0x55; 16],
        authorization_signature: [0x66; 64],
    }
}

#[test]
fn compact_catalog_list_round_trips_without_secret_material() {
    let bindings = [
        binding(b"kafka/orders/consume", b"orders-consumer", b"ops/consumer.enc"),
        binding(b"kafka/orders/produce", b"orders-producer", b"ops/producer.enc"),
    ];
    let mut encoded = vec![0; catalog_list_encoded_len(&bindings).unwrap()];
    let len = encode_catalog_list(&bindings, &mut encoded).unwrap();
    let decoded = decode_catalog_list(&encoded[..len]).unwrap().collect::<Vec<_>>();
    assert_eq!(decoded, bindings);
    assert!(!encoded.windows(b"password".len()).any(|window| window == b"password"));
}

#[test]
fn catalog_list_rejects_truncation_and_trailing_data() {
    let bindings = [binding(b"s3/reports", b"reports-s3", b"ops/reports.enc")];
    let mut encoded = vec![0; catalog_list_encoded_len(&bindings).unwrap()];
    encode_catalog_list(&bindings, &mut encoded).unwrap();
    assert!(decode_catalog_list(&encoded[..encoded.len() - 1]).is_none());

    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(decode_catalog_list(&trailing).is_none());
}

#[test]
fn pickup_round_trips_exact_fetched_inputs() {
    let binding = binding(b"kafka/orders/tx", b"orders-tx", b"ops/orders-tx.enc");
    let release = vec![0xc3; charlotte_launch::release::HEADER_LEN];
    let artifact = b"signed-elf-placeholder";
    let descriptor = vec![0xa5; charlotte_launch::deployment::HEADER_LEN];
    let envelope = vec![0x5a; charlotte_launch::operations::HEADER_LEN];
    let pickup = Pickup {
        binding,
        now_unix_seconds: 1_900_000_000,
        release: &release,
        artifact,
        descriptor: &descriptor,
        envelope: &envelope,
    };
    let mut encoded = vec![0; pickup.encoded_len().unwrap()];
    let len = pickup.encode(&mut encoded).unwrap();
    assert_eq!(Pickup::decode(&encoded[..len]), Some(pickup));

    encoded[78] = 1;
    assert_eq!(Pickup::decode(&encoded[..len]), None);
}
