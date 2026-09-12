//! Catalog host tests.

use alloc::vec;

use ed25519_compact::{
    KeyPair,
    Signature,
};
use rand_core_10::UnwrapErr;

use super::*;

fn signed_deployment(pair: &KeyPair, name: &[u8], sequence: u64) -> Vec<u8> {
    signed_deployment_with_policy(
        pair,
        name,
        sequence,
        charlotte_launch::placement::PlacementPolicy::singleton(),
    )
}

fn signed_deployment_with_policy(
    pair: &KeyPair,
    name: &[u8],
    sequence: u64,
    placement: charlotte_launch::placement::PlacementPolicy,
) -> Vec<u8> {
    let fields = charlotte_launch::deployment::DescriptorFields {
        sequence,
        node_key: 0,
        artifact_digest: [sequence as u8; 32],
        artifact_name: name,
        stack_pages_per_thread: charlotte_launch::DEFAULT_USER_STACK_PAGES as u16,
        max_threads: charlotte_launch::DEFAULT_USER_MAX_THREADS as u16,
        shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
        placement,
        object_key: name,
        grants: &[],
    };
    let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let mut bytes = vec![0; charlotte_launch::deployment::encoded_len(&fields).unwrap()];
    charlotte_launch::deployment::encode_unsigned(&fields, public_key, &mut bytes).unwrap();
    let signature: Signature =
        pair.sk.sign(charlotte_launch::deployment::signature_digest(&bytes).unwrap(), None);
    let signature: &[u8; charlotte_launch::deployment::SIGNATURE_LEN] =
        signature.as_ref().try_into().unwrap();
    assert!(charlotte_launch::deployment::set_signature(&mut bytes, signature));
    bytes
}

fn signed_release(pair: &KeyPair, name: &[u8], sequence: u64, descriptors: &[&[u8]]) -> Vec<u8> {
    let fields = charlotte_launch::release::ReleaseFields {
        sequence,
        release_name: name,
        descriptors,
    };
    let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let mut bytes = vec![0; charlotte_launch::release::encoded_len(&fields).unwrap()];
    charlotte_launch::release::encode_unsigned(&fields, public_key, &mut bytes).unwrap();
    let signature: Signature =
        pair.sk.sign(charlotte_launch::release::signature_digest(&bytes).unwrap(), None);
    let signature: &[u8; charlotte_launch::release::SIGNATURE_LEN] =
        signature.as_ref().try_into().unwrap();
    assert!(charlotte_launch::release::set_signature(&mut bytes, signature));
    bytes
}

fn signed_shutdown(pair: &KeyPair, sequence: u64, target_node: u64) -> Vec<u8> {
    let fields = charlotte_launch::shutdown::ShutdownFields {
        sequence,
        target_node,
        not_before_unix_seconds: 1_788_600_000,
        expires_unix_seconds: 1_788_600_300,
        node_grace_ms: 30_000,
        phase_grace_ms: 2_000,
        reason: charlotte_launch::shutdown::REASON_POWER_OFF,
    };
    let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let mut bytes = vec![0; charlotte_launch::shutdown::ENCODED_LEN];
    charlotte_launch::shutdown::encode_unsigned(&fields, public_key, &mut bytes).unwrap();
    let signature: Signature =
        pair.sk.sign(charlotte_launch::shutdown::signature_digest(&bytes).unwrap(), None);
    assert!(charlotte_launch::shutdown::set_signature(
        &mut bytes,
        signature.as_ref().try_into().unwrap()
    ));
    bytes
}

fn signed_ingress_policy(
    pair: &KeyPair,
    cluster_id: [u8; 32],
    sequence: u64,
    address: [u8; 4],
) -> Vec<u8> {
    let assignments = [charlotte_launch::ingress::ServiceBinding {
        service: charlotte_launch::ingress::ServiceId::tcp_v4(address, 443),
        backend_name: Some(b"orders"),
    }];
    let fields = charlotte_launch::ingress_policy::PolicyFields {
        sequence,
        not_before_unix_seconds: 1_788_600_000,
        expires_unix_seconds: 1_788_600_300,
        cluster_id,
        assignments: &assignments,
    };
    let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let mut bytes = vec![0; charlotte_launch::ingress_policy::encoded_len(&fields).unwrap()];
    charlotte_launch::ingress_policy::encode_unsigned(&fields, public_key, &mut bytes).unwrap();
    let signature: Signature =
        pair.sk.sign(charlotte_launch::ingress_policy::signature_digest(&bytes).unwrap(), None);
    assert!(charlotte_launch::ingress_policy::set_signature(
        &mut bytes,
        signature.as_ref().try_into().unwrap()
    ));
    bytes
}

fn signed_operational_bundle(
    release: &[u8],
    operational: &KeyPair,
    bundle_sequence: u64,
    envelope_sequence: u64,
    profile: &[u8],
) -> Vec<u8> {
    let operational_public: &[u8; 32] = operational.pk.as_ref().try_into().unwrap();
    let recipient_private = [0x71; 32];
    let recipient_public =
        charlotte_launch::operations::recipient_public_key(&recipient_private).unwrap();
    let cluster_id = [0x11; 32];
    let fields = charlotte_launch::operations::EnvelopeFields {
        sequence: envelope_sequence,
        expires_unix_seconds: 2_000_000_000,
        profile_kind: charlotte_launch::operations::PROFILE_KIND_KAFKA,
        cluster_id,
        release_digest: charlotte_launch::sha256::digest(release),
        profile_name: b"kafka/orders/transactional",
    };
    let mut envelope =
        vec![0; charlotte_launch::operations::encoded_len(&fields, profile.len()).unwrap()];
    let mut rng = UnwrapErr(getrandom::SysRng);
    charlotte_launch::operations::seal_unsigned(
        &fields,
        profile,
        &recipient_public,
        operational_public,
        &mut rng,
        &mut envelope,
    )
    .unwrap();
    let signature: Signature = operational
        .sk
        .sign(charlotte_launch::operations::signature_digest(&envelope).unwrap(), None);
    assert!(charlotte_launch::operations::set_signature(
        &mut envelope,
        signature.as_ref().try_into().unwrap()
    ));
    let bindings = [charlotte_launch::operations_bundle::BindingFields {
        target_artifact: b"kafka",
        object_key: b"operations/orders-kafka.cops",
        envelope: &envelope,
    }];
    let fields = charlotte_launch::operations_bundle::BundleFields {
        sequence: bundle_sequence,
        cluster_id,
        release,
        bindings: &bindings,
    };
    let mut bundle = vec![0; charlotte_launch::operations_bundle::encoded_len(&fields).unwrap()];
    charlotte_launch::operations_bundle::encode_unsigned(
        &fields,
        operational_public,
        &recipient_public,
        &mut bundle,
    )
    .unwrap();
    let binding_signature: Signature = operational.sk.sign(
        charlotte_launch::operations_bundle::binding_signature_digest(&bundle, 0).unwrap(),
        None,
    );
    assert!(charlotte_launch::operations_bundle::set_binding_signature(
        &mut bundle,
        0,
        binding_signature.as_ref().try_into().unwrap()
    ));
    let signature: Signature = operational
        .sk
        .sign(charlotte_launch::operations_bundle::signature_digest(&bundle).unwrap(), None);
    assert!(charlotte_launch::operations_bundle::set_signature(
        &mut bundle,
        signature.as_ref().try_into().unwrap()
    ));
    bundle
}

fn i64_result(bytes: Vec<u8>) -> i64 {
    i64::from_le_bytes(bytes.try_into().unwrap())
}

#[test]
fn shutdown_intent_is_replay_fenced_and_survives_snapshot() {
    let pair = KeyPair::from_seed([0x55; 32].into());
    let key: [u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let catalog = NameCatalog::new_with_deployment_key(key);
    let first = signed_shutdown(&pair, 4, 0x1234);
    assert_eq!(i64_result(catalog.apply_with_result(1, &encode_shutdown(&first).unwrap())), 1);
    assert_eq!(i64_result(catalog.apply_with_result(1, &encode_shutdown(&first).unwrap())), 1);

    let stale = signed_shutdown(&pair, 3, 0x1234);
    assert_eq!(
        i64_result(catalog.apply_with_result(1, &encode_shutdown(&stale).unwrap())),
        crate::clusterctl::ERR_STALE_DESCRIPTOR
    );
    let next = signed_shutdown(&pair, 5, 0x1234);
    assert_eq!(i64_result(catalog.apply_with_result(1, &encode_shutdown(&next).unwrap())), 2);
    assert_eq!(catalog.ingress_draining_nodes(), vec![(0x1234, 2)]);

    let query = catalog.query(&encode_shutdown_query(0x1234));
    assert_eq!(decode_shutdown_result(&query).unwrap().envelope, next);
    let restored = NameCatalog::new_with_deployment_key(key);
    restored.restore(&catalog.snapshot());
    assert_eq!(restored.shutdown_intent(0x1234), catalog.shutdown_intent(0x1234));
    assert_eq!(restored.ingress_draining_nodes(), vec![(0x1234, 2)]);
}

#[test]
fn ingress_policy_is_replay_fenced_and_survives_snapshot() {
    let deployment = KeyPair::from_seed([0x56; 32].into());
    let operations = KeyPair::from_seed([0x57; 32].into());
    let deployment_key = deployment.pk.as_ref().try_into().unwrap();
    let operations_key = operations.pk.as_ref().try_into().unwrap();
    let cluster_id = [0x58; 32];
    let catalog =
        NameCatalog::new_with_control_plane_trust(deployment_key, operations_key, cluster_id);
    let first = signed_ingress_policy(&operations, cluster_id, 4, [10, 0, 2, 42]);
    assert_eq!(
        i64_result(catalog.apply_with_result(1, &encode_ingress_policy(&first).unwrap())),
        1
    );
    assert_eq!(
        i64_result(catalog.apply_with_result(1, &encode_ingress_policy(&first).unwrap())),
        1
    );
    let conflict = signed_ingress_policy(&operations, cluster_id, 4, [10, 0, 2, 43]);
    assert_eq!(
        i64_result(catalog.apply_with_result(1, &encode_ingress_policy(&conflict).unwrap())),
        crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR
    );
    let stale = signed_ingress_policy(&operations, cluster_id, 3, [10, 0, 2, 43]);
    assert_eq!(
        i64_result(catalog.apply_with_result(1, &encode_ingress_policy(&stale).unwrap())),
        crate::clusterctl::ERR_STALE_DESCRIPTOR
    );
    let next = signed_ingress_policy(&operations, cluster_id, 5, [10, 0, 2, 44]);
    assert_eq!(i64_result(catalog.apply_with_result(1, &encode_ingress_policy(&next).unwrap())), 2);
    let query = catalog.query(&encode_ingress_policy_query());
    assert_eq!(decode_ingress_policy_result(&query).unwrap().envelope, next);

    let restored =
        NameCatalog::new_with_control_plane_trust(deployment_key, operations_key, cluster_id);
    restored.restore(&catalog.snapshot());
    assert_eq!(restored.ingress_policy(), catalog.ingress_policy());
}

#[test]
fn node_capacity_is_replay_fenced_and_survives_snapshot() {
    let catalog = NameCatalog::new();
    let sample = |boot_nonce, epoch, free_frames| NodeCapacityEntry {
        node_key: 0x1234,
        boot_nonce,
        epoch,
        free_frames,
        usable_frames: 1000,
        cpu_load_permille: 250,
    };
    catalog.apply_with_result(1, &encode_node_capacity(&sample(7, 5, 600)));
    assert_eq!(catalog.node_capacity(0x1234), Some(sample(7, 5, 600)));

    // Within one boot, a lower epoch is a stale reorder and is ignored.
    catalog.apply_with_result(1, &encode_node_capacity(&sample(7, 4, 100)));
    assert_eq!(catalog.node_capacity(0x1234).unwrap().free_frames, 600);

    // A new boot supersedes the previous boot even with a smaller epoch.
    catalog.apply_with_result(1, &encode_node_capacity(&sample(9, 1, 700)));
    assert_eq!(catalog.node_capacity(0x1234), Some(sample(9, 1, 700)));

    // Out-of-range samples are rejected outright.
    catalog.apply_with_result(1, &encode_node_capacity(&sample(9, 2, 2000)));
    assert_eq!(catalog.node_capacity(0x1234).unwrap().free_frames, 700);

    let view = catalog.node_capacity_view();
    assert_eq!(view.get(&0x1234).unwrap().free_frames, 700);

    let restored = NameCatalog::new();
    restored.restore(&catalog.snapshot());
    assert_eq!(restored.node_capacity(0x1234), catalog.node_capacity(0x1234));
    assert_eq!(restored.node_capacity_view().get(&0x1234).unwrap().free_frames, 700);
}

#[test]
fn deployment_generation_survives_activation_and_snapshot() {
    let catalog = NameCatalog::new();
    assert_eq!(
        catalog.apply_with_result(
            1,
            &encode_deploy(b"orders", 17, 0x89ab_cdef, &[0x5a; 32], b"descriptor")
        ),
        1u64.to_le_bytes()
    );
    let prepared = catalog
        .apply_with_result(1, &encode_register_deployment(b"orders", b"charlotte:89abcdef", 1));
    let service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"orders", service_generation));

    let entry = catalog.lookup(b"orders").unwrap();
    assert_eq!(entry.deployment_generation, 1);
    assert_eq!(crate::node_identity::key_from_name(&entry.node), Some(0x89ab_cdef));

    let restored = NameCatalog::new();
    restored.restore(&catalog.snapshot());
    assert_eq!(restored.lookup(b"orders"), Some(entry));
}

#[test]
fn ingress_placement_requires_the_exact_ready_deployment_generation() {
    let catalog = NameCatalog::new();
    let first = catalog.apply_with_result(
        1,
        &encode_deploy(b"orders", 17, 0x89ab_cdef, &[0x5a; 32], b"descriptor-v1"),
    );
    assert_eq!(u64::from_le_bytes(first.try_into().unwrap()), 1);
    assert_eq!(
        catalog.ingress_placement(b"orders"),
        Some(IngressPlacement {
            deployment_generation: 1,
            service_generation: 0,
            ready_nodes: vec![],
        })
    );

    let prepared = catalog
        .apply_with_result(1, &encode_register_deployment(b"orders", b"charlotte:89abcdef", 1));
    let first_service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"orders", first_service_generation));
    assert_eq!(
        catalog.ingress_placement(b"orders"),
        Some(IngressPlacement {
            deployment_generation: 1,
            service_generation: first_service_generation,
            ready_nodes: vec![0x89ab_cdef],
        })
    );

    let second = catalog.apply_with_result(
        1,
        &encode_deploy(b"orders", 18, 0x1234_abcd, &[0x6b; 32], b"descriptor-v2"),
    );
    assert_eq!(u64::from_le_bytes(second.try_into().unwrap()), 2);
    assert_eq!(
        catalog.ingress_placement(b"orders"),
        Some(IngressPlacement {
            deployment_generation: 2,
            service_generation: first_service_generation,
            ready_nodes: vec![],
        })
    );

    let prepared = catalog
        .apply_with_result(1, &encode_register_deployment(b"orders", b"charlotte:1234abcd", 2));
    let second_service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"orders", second_service_generation));
    assert_eq!(
        catalog.ingress_placement(b"orders"),
        Some(IngressPlacement {
            deployment_generation: 2,
            service_generation: second_service_generation,
            ready_nodes: vec![0x1234_abcd],
        })
    );
}

#[test]
fn replica_set_readiness_is_per_node_generation_fenced_and_snapshotted() {
    let pair = KeyPair::from_seed([0x63; 32].into());
    let key: [u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let descriptor = signed_deployment_with_policy(
        &pair,
        b"orders",
        1,
        charlotte_launch::placement::PlacementPolicy {
            replicas: 2,
            max_instances_per_node: 1,
            min_distinct_nodes: 2,
            flags: charlotte_launch::placement::SPREAD_REPLICAS,
            affinity_group: 0,
            anti_affinity_group: 0,
        },
    );
    let release = signed_release(&pair, b"orders-v1", 1, &[&descriptor]);
    let catalog = NameCatalog::new_with_deployment_key(key);
    let command = encode_release_replicas(&release, &[vec![0x1111_1111, 0x2222_2222]])
        .expect("replica command");
    assert_eq!(catalog.apply_with_result(1, &command), 1i64.to_le_bytes());
    assert_eq!(
        catalog.deployment(b"orders").unwrap().replica_nodes,
        vec![0x1111_1111, 0x2222_2222]
    );

    let first = catalog
        .apply_with_result(1, &encode_register_deployment(b"orders", b"charlotte:11111111", 1));
    let first = u64::from_le_bytes(first.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"orders", first));
    let second = catalog
        .apply_with_result(1, &encode_register_deployment(b"orders", b"charlotte:22222222", 1));
    let second = u64::from_le_bytes(second.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"orders", second));
    assert_eq!(
        catalog.ingress_placement(b"orders"),
        Some(IngressPlacement {
            deployment_generation: 1,
            service_generation: second,
            ready_nodes: vec![0x1111_1111, 0x2222_2222],
        })
    );

    let restored = NameCatalog::new_with_deployment_key(key);
    restored.restore(&catalog.snapshot());
    assert_eq!(restored.ingress_placement(b"orders"), catalog.ingress_placement(b"orders"));
    assert_eq!(
        restored.apply_with_result(
            1,
            &encode_unregister_generation(b"orders", b"charlotte:11111111", first),
        ),
        first.to_le_bytes()
    );
    assert_eq!(restored.ingress_placement(b"orders").unwrap().ready_nodes, vec![0x2222_2222]);
    let reassigned = restored
        .apply_with_result(1, &encode_reassign(b"orders", 1, &[0x2222_2222, 0x3333_3333]).unwrap());
    assert_eq!(u64::from_le_bytes(reassigned.try_into().unwrap()), 2);
    assert_eq!(
        restored.ingress_placement(b"orders"),
        Some(IngressPlacement {
            deployment_generation: 2,
            service_generation: second,
            ready_nodes: vec![],
        })
    );
    assert_eq!(
            restored.apply_with_result(
                1,
                &encode_register_deployment(b"orders", b"charlotte:33333333", 1),
            ),
            0u64.to_le_bytes()
        );
}

#[test]
fn ordinary_registration_has_no_deployment_generation() {
    let catalog = NameCatalog::new();
    let prepared = catalog.apply_with_result(1, &encode_register(b"dns", b"charlotte:1234abcd"));
    let service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"dns", service_generation));
    assert_eq!(catalog.lookup(b"dns").unwrap().deployment_generation, 0);
}

#[test]
fn consensus_domain_reset_discards_the_standalone_catalog() {
    let catalog = NameCatalog::new();
    let prepared = catalog.apply_with_result(1, &encode_register(b"dns", b"charlotte:1234abcd"));
    let service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"dns", service_generation));
    catalog.apply(1, &encode_deploy(b"orders", 17, 0x1234_abcd, &[0x5a; 32], b"descriptor"));
    assert!(catalog.lookup(b"dns").is_some());
    assert!(catalog.deployment(b"orders").is_some());

    catalog.reset();

    assert!(catalog.lookup(b"dns").is_none());
    assert!(catalog.deployment(b"orders").is_none());
    assert_eq!(catalog.registered_count(), 0);
    assert_eq!(catalog.deployment_count(), 0);
    assert!(catalog.cluster_key().is_none());
}

#[test]
fn exact_deployment_retry_is_idempotent() {
    let catalog = NameCatalog::new();
    let command = encode_deploy(b"orders", 17, 0x1234_abcd, &[0x5a; 32], b"descriptor");
    let first = catalog.apply_with_result(1, &command);
    let retry = catalog.apply_with_result(2, &command);
    assert_eq!(u64::from_le_bytes(first.try_into().unwrap()), 1);
    assert_eq!(u64::from_le_bytes(retry.try_into().unwrap()), 1);

    let replacement = encode_deploy(b"orders", 17, 0x89ab_cdef, &[0x5a; 32], b"descriptor");
    let next = catalog.apply_with_result(3, &replacement);
    assert_eq!(u64::from_le_bytes(next.try_into().unwrap()), 2);
}

#[test]
fn deployment_capacity_rejects_new_keys_and_allows_replacements() {
    let catalog = NameCatalog::new();
    {
        let mut deployments = catalog.deployments.lock();
        for index in 0..MAX_DEPLOYMENTS {
            deployments.insert(
                alloc::format!("fill-{index}").into_bytes(),
                DeploymentEntry {
                    object_id: index as u64,
                    node_key: 0,
                    replica_nodes: Vec::new(),
                    generation: 1,
                    artifact_digest: [0; 32],
                    descriptor: Vec::new(),
                },
            );
        }
    }

    let overflow =
        catalog.apply_with_result(1, &encode_deploy(b"overflow", 1, 1, &[0; 32], b"descriptor"));
    assert_eq!(overflow, crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec());

    let replacement =
        catalog.apply_with_result(2, &encode_deploy(b"fill-0", 2, 2, &[1; 32], b"d2"));
    assert_ne!(replacement, crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec());
    assert_eq!(catalog.deployment(b"fill-0").unwrap().object_id, 2);
}

#[test]
fn signed_release_is_atomic_idempotent_and_snapshot_persistent() {
    let pair = KeyPair::generate();
    let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let catalog = NameCatalog::new();
    assert_eq!(i64_result(catalog.apply_with_result(1, &encode_set_cluster_key(public_key))), 1);

    let receive_v1 = signed_deployment(&pair, b"receive", 1);
    let publish_v1 = signed_deployment(&pair, b"publish", 1);
    let release_v1 =
        signed_release(&pair, b"orders", 1, &[receive_v1.as_slice(), publish_v1.as_slice()]);
    let command_v1 = encode_release(&release_v1, &[0x1111, 0x2222]).unwrap();
    let relay = crate::rrelease::Request {
        session: 3,
        request_id: 9,
        caller: b"charlotte:1234abcd".to_vec(),
        envelope: release_v1.clone(),
    };
    let encoded_relay = crate::rrelease::encode_request(&relay).unwrap();
    assert_eq!(
        crate::rrelease::decode_request(
            &[&[crate::rrelease::TAG_REQUEST], encoded_relay.as_slice()].concat()
        ),
        Some(relay)
    );
    let relay_reply = crate::rrelease::encode_reply(3, 9, 1);
    assert_eq!(
        crate::rrelease::decode_reply(
            &[&[crate::rrelease::TAG_REPLY], relay_reply.as_slice()].concat()
        ),
        Some((3, 9, 1))
    );
    assert_eq!(i64_result(catalog.apply_with_result(1, &command_v1)), 1);
    assert_eq!(i64_result(catalog.apply_with_result(2, &command_v1)), 1);
    assert_eq!(catalog.deployment(b"receive").unwrap().node_key, 0x1111);
    assert_eq!(catalog.deployment(b"publish").unwrap().node_key, 0x2222);

    let receive_v2 = signed_deployment(&pair, b"receive", 2);
    let publish_v2 = signed_deployment(&pair, b"publish", 2);
    let release_v2 =
        signed_release(&pair, b"orders", 2, &[receive_v2.as_slice(), publish_v2.as_slice()]);
    assert_eq!(
        i64_result(
            catalog.apply_with_result(3, &encode_release(&release_v2, &[0x3333, 0x4444]).unwrap(),)
        ),
        2
    );

    let receive_v3 = signed_deployment(&pair, b"receive", 3);
    let stale_release =
        signed_release(&pair, b"orders", 3, &[receive_v3.as_slice(), publish_v1.as_slice()]);
    assert_eq!(
        i64_result(
            catalog
                .apply_with_result(4, &encode_release(&stale_release, &[0x5555, 0x6666]).unwrap(),)
        ),
        crate::clusterctl::ERR_STALE_DESCRIPTOR
    );
    let receive = catalog.deployment(b"receive").unwrap();
    assert_eq!(receive.node_key, 0x3333);
    assert_eq!(charlotte_launch::deployment::decode(&receive.descriptor).unwrap().sequence, 2);

    let restored = NameCatalog::new();
    restored.restore(&catalog.snapshot());
    assert_eq!(restored.release(b"orders"), catalog.release(b"orders"));
    assert_eq!(restored.deployment(b"receive"), catalog.deployment(b"receive"));
    assert_eq!(restored.deployment(b"publish"), catalog.deployment(b"publish"));
}

#[test]
fn operational_bindings_are_atomic_fenced_and_snapshot_persistent() {
    let release_pair = KeyPair::generate();
    let operational_pair = KeyPair::generate();
    let release_public: &[u8; 32] = release_pair.pk.as_ref().try_into().unwrap();
    let catalog = NameCatalog::new();
    assert_eq!(
        i64_result(catalog.apply_with_result(1, &encode_set_cluster_key(release_public))),
        1
    );

    let kafka_v1 = signed_deployment(&release_pair, b"kafka", 1);
    let release_v1 = signed_release(&release_pair, b"orders", 1, &[kafka_v1.as_slice()]);
    let bundle_v1 = signed_operational_bundle(&release_v1, &operational_pair, 1, 1, b"profile-v1");
    let command_v1 = encode_release_with_operations(&bundle_v1, &[0x1111]).unwrap();
    assert!(command_v1.len() <= catten_graft::types::MAX_COMMAND_BYTES);
    assert_eq!(i64_result(catalog.apply_with_result(2, &command_v1)), 1);
    assert_eq!(i64_result(catalog.apply_with_result(3, &command_v1)), 1);

    let binding = catalog.operational_binding(b"kafka/orders/transactional").unwrap();
    assert_eq!(binding.generation, 1);
    assert_eq!(binding.sequence, 1);
    assert_eq!(binding.target_artifact, b"kafka");
    assert_eq!(binding.release_digest, charlotte_launch::sha256::digest(&release_v1));
    assert_eq!(catalog.release(b"orders").unwrap().operations_sequence, 1);

    let restored = NameCatalog::new();
    restored.restore(&catalog.snapshot());
    assert_eq!(restored.operational_binding(b"kafka/orders/transactional"), Some(binding.clone()));
    assert_eq!(restored.release(b"orders"), catalog.release(b"orders"));

    let bundle_v2 = signed_operational_bundle(&release_v1, &operational_pair, 2, 2, b"profile-v2");
    let command_v2 = encode_release_with_operations(&bundle_v2, &[0x1111]).unwrap();
    assert_eq!(i64_result(catalog.apply_with_result(4, &command_v2)), 1);
    let binding_v2 = catalog.operational_binding(b"kafka/orders/transactional").unwrap();
    assert_eq!(binding_v2.generation, 2);
    assert_eq!(binding_v2.sequence, 2);
    assert_eq!(
        i64_result(catalog.apply_with_result(5, &command_v1)),
        crate::clusterctl::ERR_STALE_DESCRIPTOR
    );
    let conflicting_v2 =
        signed_operational_bundle(&release_v1, &operational_pair, 2, 3, b"conflict");
    assert_eq!(
        i64_result(catalog.apply_with_result(
            6,
            &encode_release_with_operations(&conflicting_v2, &[0x1111]).unwrap(),
        )),
        crate::clusterctl::ERR_CONFLICTING_DESCRIPTOR
    );

    let kafka_v2 = signed_deployment(&release_pair, b"kafka", 2);
    let release_v2 = signed_release(&release_pair, b"orders", 2, &[kafka_v2.as_slice()]);
    assert_eq!(
        i64_result(catalog.apply_with_result(7, &encode_release(&release_v2, &[0x2222]).unwrap(),)),
        2
    );
    assert!(catalog.operational_binding(b"kafka/orders/transactional").is_none());

    let bundle_v3 = signed_operational_bundle(&release_v2, &operational_pair, 3, 3, b"profile-v3");
    assert_eq!(
        i64_result(
            catalog.apply_with_result(
                8,
                &encode_release_with_operations(&bundle_v3, &[0x2222]).unwrap(),
            )
        ),
        2
    );
    let binding_v3 = catalog.operational_binding(b"kafka/orders/transactional").unwrap();
    assert_eq!(binding_v3.generation, 4);
    assert_eq!(binding_v3.sequence, 3);
    assert_eq!(binding_v3.release_digest, charlotte_launch::sha256::digest(&release_v2));
}

#[test]
fn rollout_status_and_remote_registration_are_canonical() {
    let status = crate::clusterctl::RolloutStatus {
        state: crate::clusterctl::ROLLOUT_READY,
        deployment_generation: 7,
        service_generation: 11,
        node_key: 0x1234_abcd,
        desired_replicas: 3,
        ready_replicas: 3,
    };
    assert_eq!(crate::clusterctl::RolloutStatus::decode(&status.encode()), Some(status));

    let frame = crate::rregister::encode_request(b"charlotte:1234abcd", b"orders", 7);
    assert_eq!(
        crate::rregister::decode_request(
            &[&[crate::rregister::TAG_REQUEST], frame.as_slice()].concat()
        ),
        Some((b"charlotte:1234abcd".to_vec(), b"orders".to_vec(), 7))
    );

    let request = crate::rdeploy::Request {
        session: 3,
        request_id: 9,
        caller: b"charlotte:1234abcd".to_vec(),
        artifact: b"orders".to_vec(),
        object_id: 17,
        node_key: 0,
        digest: [0x5a; 32],
        descriptor: b"signed descriptor".to_vec(),
    };
    let encoded = crate::rdeploy::encode_request(&request).unwrap();
    assert_eq!(
        crate::rdeploy::decode_request(
            &[&[crate::rdeploy::TAG_REQUEST], encoded.as_slice()].concat()
        ),
        Some(request)
    );
    let reply = crate::rdeploy::encode_reply(3, 9, 21);
    assert_eq!(
        crate::rdeploy::decode_reply(&[&[crate::rdeploy::TAG_REPLY], reply.as_slice()].concat()),
        Some((3, 9, 21))
    );
}
