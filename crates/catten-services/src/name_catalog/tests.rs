//! Catalog host tests.

use alloc::vec;

use ed25519_compact::{
    KeyPair,
    Signature,
};
use rand_core_10::UnwrapErr;

use super::*;
fn fixture_pair() -> KeyPair {
    KeyPair::from_seed([0x37; 32].into())
}
fn fixture_profiles() -> crate::operations_admission::NodeCapacityView {
    [
        1,
        2,
        3,
        4,
        9,
        0x1234,
        0x1111,
        0x2222,
        0x3333,
        0x4444,
        0x5555,
        0x6666,
        0x1111_1111,
        0x2222_2222,
        0x3333_3333,
        0x89ab_cdef,
        0x1234_abcd,
    ]
    .into_iter()
    .map(|node| {
        (
            node,
            crate::operations_admission::NodeCapacity {
                target: Some(charlotte_launch::execution_target::ExecutionTarget::baseline(
                    charlotte_launch::execution_target::Architecture::Aarch64,
                )),
                ..Default::default()
            },
        )
    })
    .collect()
}
fn seed_catalog(catalog: Arc<NameCatalog>) -> Arc<NameCatalog> {
    for (node, capacity) in fixture_profiles() {
        catalog.apply(
            1,
            &encode_node_capacity(&NodeCapacityEntry {
                target: capacity.target.unwrap(),
                node_key: node,
                boot_nonce: 1,
                epoch: 1,
                free_frames: 10000,
                usable_frames: 20000,
                cpu_load_permille: 0,
            }),
        );
    }
    catalog
}
fn fixture_catalog() -> Arc<NameCatalog> {
    seed_catalog(NameCatalog::new_with_deployment_key(
        fixture_pair().pk.as_ref().try_into().unwrap(),
    ))
}
fn encode_release(bytes: &[u8], nodes: &[u64]) -> Option<Vec<u8>> {
    super::encode_release(bytes, nodes, &fixture_profiles())
}
fn encode_release_replicas(bytes: &[u8], nodes: &[Vec<u64>]) -> Option<Vec<u8>> {
    super::encode_release_replicas(bytes, nodes, &fixture_profiles())
}
fn encode_release_with_operations(bytes: &[u8], nodes: &[u64]) -> Option<Vec<u8>> {
    super::encode_release_with_operations(bytes, nodes, &fixture_profiles())
}
fn deploy_fixture(name: &[u8], node: u64, sequence: u64) -> Vec<u8> {
    let descriptor = signed_deployment(&fixture_pair(), name, sequence);
    super::encode_deploy(&descriptor, node, 0)
}

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
        variants: &[charlotte_launch::deployment::ArtifactVariant {
            target: charlotte_launch::execution_target::ExecutionTarget::baseline(
                charlotte_launch::execution_target::Architecture::Aarch64,
            ),
            priority: 0,
            artifact_digest: [sequence as u8; 32],
            object_key: name,
        }],
        sequence,
        node_key: 0,

        artifact_name: name,
        stack_pages_per_thread: charlotte_launch::DEFAULT_USER_STACK_PAGES as u16,
        max_threads: charlotte_launch::DEFAULT_USER_MAX_THREADS as u16,
        shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
        placement,

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
    let catalog = seed_catalog(NameCatalog::new_with_deployment_key(key));
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
    let restored = seed_catalog(NameCatalog::new_with_deployment_key(key));
    restored.restore(&catalog.snapshot()).unwrap();
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
    let catalog = seed_catalog(NameCatalog::new_with_control_plane_trust(
        deployment_key,
        operations_key,
        cluster_id,
    ));
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

    let restored = seed_catalog(NameCatalog::new_with_control_plane_trust(
        deployment_key,
        operations_key,
        cluster_id,
    ));
    restored.restore(&catalog.snapshot()).unwrap();
    assert_eq!(restored.ingress_policy(), catalog.ingress_policy());
}

#[test]
fn node_capacity_is_replay_fenced_and_survives_snapshot() {
    let catalog = fixture_catalog();
    let sample = |boot_nonce, epoch, free_frames| NodeCapacityEntry {
        target: charlotte_launch::execution_target::ExecutionTarget::baseline(
            charlotte_launch::execution_target::Architecture::Aarch64,
        ),
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

    let restored = fixture_catalog();
    restored.restore(&catalog.snapshot()).unwrap();
    assert_eq!(restored.node_capacity(0x1234), catalog.node_capacity(0x1234));
    assert_eq!(restored.node_capacity_view().get(&0x1234).unwrap().free_frames, 700);
}

#[test]
fn deployment_generation_survives_activation_and_snapshot() {
    let catalog = fixture_catalog();
    assert_eq!(
        catalog.apply_with_result(1, &deploy_fixture(b"orders", 0x89ab_cdef, 1)),
        1u64.to_le_bytes()
    );
    let prepared = catalog
        .apply_with_result(1, &encode_register_deployment(b"orders", b"charlotte:89abcdef", 1));
    let service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"orders", service_generation));

    let entry = catalog.lookup(b"orders").unwrap();
    assert_eq!(entry.deployment_generation, 1);
    assert_eq!(crate::node_identity::key_from_name(&entry.node), Some(0x89ab_cdef));

    let restored = fixture_catalog();
    restored.restore(&catalog.snapshot()).unwrap();
    assert_eq!(restored.lookup(b"orders"), Some(entry));
}

#[test]
fn ingress_placement_requires_the_exact_ready_deployment_generation() {
    let catalog = fixture_catalog();
    let first = catalog.apply_with_result(1, &deploy_fixture(b"orders", 0x89ab_cdef, 1));
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

    let second = catalog.apply_with_result(1, &deploy_fixture(b"orders", 0x1234_abcd, 2));
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
    let catalog = seed_catalog(NameCatalog::new_with_deployment_key(key));
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

    let restored = seed_catalog(NameCatalog::new_with_deployment_key(key));
    restored.restore(&catalog.snapshot()).unwrap();
    assert_eq!(restored.ingress_placement(b"orders"), catalog.ingress_placement(b"orders"));
    assert_eq!(
        restored.apply_with_result(
            1,
            &encode_unregister_generation(b"orders", b"charlotte:11111111", first),
        ),
        first.to_le_bytes()
    );
    assert_eq!(restored.ingress_placement(b"orders").unwrap().ready_nodes, vec![0x2222_2222]);
    let reassigned = restored.apply_with_result(
        1,
        &encode_reassign(
            b"orders",
            1,
            &[0x2222_2222, 0x3333_3333],
            &descriptor,
            &fixture_profiles(),
        )
        .unwrap(),
    );
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
    let catalog = fixture_catalog();
    let prepared = catalog.apply_with_result(1, &encode_register(b"dns", b"charlotte:1234abcd"));
    let service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"dns", service_generation));
    assert_eq!(catalog.lookup(b"dns").unwrap().deployment_generation, 0);
}

#[test]
fn consensus_domain_reset_discards_the_standalone_catalog() {
    let catalog = fixture_catalog();
    let prepared = catalog.apply_with_result(1, &encode_register(b"dns", b"charlotte:1234abcd"));
    let service_generation = u64::from_le_bytes(prepared.try_into().unwrap());
    catalog.apply(1, &encode_activate(b"dns", service_generation));
    catalog.apply(1, &deploy_fixture(b"orders", 0x1234_abcd, 1));
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
    let catalog = fixture_catalog();
    let command = deploy_fixture(b"orders", 0x1234_abcd, 1);
    let first = catalog.apply_with_result(1, &command);
    let retry = catalog.apply_with_result(2, &command);
    assert_eq!(u64::from_le_bytes(first.try_into().unwrap()), 1);
    assert_eq!(u64::from_le_bytes(retry.try_into().unwrap()), 1);

    let replacement = deploy_fixture(b"orders", 0x89ab_cdef, 1);
    let next = catalog.apply_with_result(3, &replacement);
    assert_eq!(u64::from_le_bytes(next.try_into().unwrap()), 2);
}

#[test]
fn deployment_capacity_rejects_new_keys_and_allows_replacements() {
    let catalog = fixture_catalog();
    {
        let mut deployments = catalog.deployments.lock();
        for index in 0..MAX_DEPLOYMENTS {
            deployments.insert(
                alloc::format!("fill-{index}").into_bytes(),
                DeploymentEntry {
                    replica_nodes: Vec::new(),
                    replica_variants: Vec::new(),
                    generation: 1,

                    descriptor: signed_deployment(
                        &fixture_pair(),
                        alloc::format!("fill-{index}").as_bytes(),
                        1,
                    ),
                },
            );
        }
    }

    let overflow = catalog.apply_with_result(1, &deploy_fixture(b"overflow", 1, 1));
    assert_eq!(overflow, crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec());

    let replacement = catalog.apply_with_result(2, &deploy_fixture(b"fill-0", 2, 2));
    assert_ne!(replacement, crate::clusterctl::ERR_CAPACITY.to_le_bytes().to_vec());
    assert_eq!(catalog.deployment(b"fill-0").unwrap().generation, 2);
}

#[test]
fn signed_release_is_atomic_idempotent_and_snapshot_persistent() {
    let pair = KeyPair::generate();
    let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
    let catalog = fixture_catalog();
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
    assert_eq!(catalog.deployment(b"receive").unwrap().replica_nodes, vec![0x1111]);
    assert_eq!(catalog.deployment(b"publish").unwrap().replica_nodes, vec![0x2222]);

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
    assert_eq!(receive.replica_nodes, vec![0x3333]);
    assert_eq!(charlotte_launch::deployment::decode(&receive.descriptor).unwrap().sequence, 2);

    let restored = fixture_catalog();
    restored.restore(&catalog.snapshot()).unwrap();
    assert_eq!(restored.release(b"orders"), catalog.release(b"orders"));
    assert_eq!(restored.deployment(b"receive"), catalog.deployment(b"receive"));
    assert_eq!(restored.deployment(b"publish"), catalog.deployment(b"publish"));
}

#[test]
fn operational_bindings_are_atomic_fenced_and_snapshot_persistent() {
    let release_pair = KeyPair::generate();
    let operational_pair = KeyPair::generate();
    let release_public: &[u8; 32] = release_pair.pk.as_ref().try_into().unwrap();
    let catalog = fixture_catalog();
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

    let restored = fixture_catalog();
    restored.restore(&catalog.snapshot()).unwrap();
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
        descriptor: signed_deployment(&fixture_pair(), b"orders", 1),
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

#[test]
fn retired_catalog_snapshots_reject_without_changing_live_state() {
    let catalog = fixture_catalog();
    catalog.apply(1, &encode_register(b"live", b"charlotte:1234abcd"));
    let before = catalog.snapshot();
    let current = fixture_catalog().snapshot();
    // Historical magics, including the original CATALOGM tag.
    for tag in b"M23456789ABCDEFG" {
        let mut retired = current.clone();
        let magic = 0x4341_5441_4c4f_4700u64 | u64::from(*tag);
        retired[..8].copy_from_slice(&magic.to_le_bytes());
        assert_eq!(
            catalog.restore(&retired),
            Err(catten_graft::state_machine::SnapshotError::Unsupported)
        );
        assert_eq!(catalog.snapshot(), before);
    }
    catalog.restore(&current).unwrap();
    assert_eq!(catalog.snapshot(), current);
}

#[test]
fn truncated_or_extended_catalog_snapshots_do_not_publish_any_section() {
    let pair = KeyPair::generate();
    let source = fixture_catalog();
    let public_key = pair.pk.as_ref().try_into().unwrap();
    source.apply(1, &encode_set_cluster_key(public_key));
    source.apply(1, &encode_register(b"source", b"charlotte:89abcdef"));
    let descriptor = signed_deployment(&pair, b"orders", 1);
    let release = signed_release(&pair, b"release", 1, &[&descriptor]);
    source.apply(1, &encode_release(&release, &[0x89ab_cdef]).unwrap());
    let snapshot = source.snapshot();
    let target = fixture_catalog();
    target.apply(1, &encode_register(b"keep", b"charlotte:1234abcd"));
    let before = target.snapshot();
    for length in 0..snapshot.len() {
        assert!(target.restore(&snapshot[..length]).is_err(), "accepted prefix length {length}");
        assert_eq!(target.snapshot(), before);
    }
    let mut extended = snapshot.clone();
    extended.push(0);
    assert!(target.restore(&extended).is_err());
    assert_eq!(target.snapshot(), before);
    target.restore(&snapshot).unwrap();
    assert_eq!(target.snapshot(), snapshot);
}

#[test]
fn catalog_snapshot_rejects_duplicate_keys_and_noncanonical_flags() {
    let source = fixture_catalog();
    source.apply(1, &encode_register(b"one", b"charlotte:1234abcd"));
    let snapshot = source.snapshot();
    let entry_end = 12 + 4 + 3 + 4 + b"charlotte:1234abcd".len() + 8 + 1 + 8;
    let mut duplicated = snapshot.clone();
    duplicated[8..12].copy_from_slice(&2u32.to_le_bytes());
    duplicated.splice(entry_end..entry_end, snapshot[12..entry_end].iter().copied());
    let target = fixture_catalog();
    let before = target.snapshot();
    assert!(target.restore(&duplicated).is_err());
    let mut bad_active = snapshot;
    bad_active[entry_end - 9] = 2;
    assert!(target.restore(&bad_active).is_err());
    assert_eq!(target.snapshot(), before);
}

#[test]
fn catalog_snapshot_rejects_missing_sections_and_unbounded_counts() {
    let target = fixture_catalog();
    let before = target.snapshot();
    for (offset, bound) in [
        (8, MAX_ENTRIES),
        (12, MAX_DEPLOYMENTS),
        (16, MAX_RELEASES),
        (20, MAX_OPERATIONAL_BINDINGS),
        (24, MAX_SHUTDOWN_INTENTS),
        (28, MAX_DEPLOYMENTS),
    ] {
        let mut bytes = before.clone();
        bytes[offset..offset + 4].copy_from_slice(&((bound + 1) as u32).to_le_bytes());
        assert!(target.restore(&bytes).is_err());
    }
    for offset in [32, 33] {
        // ingress presence and cluster-key presence
        let mut bytes = before.clone();
        bytes[offset] = 2;
        assert!(target.restore(&bytes).is_err());
    }
    let mut bytes = before.clone();
    bytes[34..42].copy_from_slice(&1u64.to_le_bytes()); // absent key with a generation
    assert!(target.restore(&bytes).is_err());
    assert_eq!(target.snapshot(), before);
}

#[test]
fn deployment_query_requires_complete_descriptor_and_replica_sections() {
    let entry = DeploymentEntry {
        replica_nodes: vec![4, 9],
        replica_variants: vec![0, 0],
        generation: 3,

        descriptor: signed_deployment(&fixture_pair(), b"orders", 1),
    };
    let bytes = encode_deployment_result(&entry).unwrap();
    assert_eq!(decode_deployment_result(&bytes), Some(entry));
    for length in 0..bytes.len() {
        assert!(decode_deployment_result(&bytes[..length]).is_none());
    }
    let mut extended = bytes;
    extended.push(0);
    assert!(decode_deployment_result(&extended).is_none());
}

#[test]
fn deployment_query_rejects_invalid_generation_and_replica_pins() {
    let mut entry = DeploymentEntry {
        replica_nodes: vec![4, 9],
        replica_variants: vec![0, 0],
        generation: 3,

        descriptor: signed_deployment(&fixture_pair(), b"orders", 1),
    };
    let bytes = encode_deployment_result(&entry).unwrap();
    for (offset, value) in [
        (8, 0),
        (22 + entry.descriptor.len(), 0),
        (32 + entry.descriptor.len(), 4),
        (32 + entry.descriptor.len(), 3),
    ] {
        let mut malformed = bytes.clone();
        malformed[offset..offset + 8].copy_from_slice(&(value as u64).to_le_bytes());
        assert!(decode_deployment_result(&malformed).is_none());
    }
    entry.replica_nodes.clear();
    assert!(encode_deployment_result(&entry).is_none());
    entry.replica_variants.clear();
    assert_eq!(decode_deployment_result(&encode_deployment_result(&entry).unwrap()), Some(entry));
}

#[test]
fn mixed_architecture_rollout_pins_each_binary_and_preserves_it_through_restore() {
    use charlotte_launch::{
        deployment::{
            self,
            ArtifactVariant,
            DescriptorFields,
        },
        execution_target::{
            AARCH64_LSE,
            Architecture,
            ExecutionTarget,
        },
    };
    let pair = fixture_pair();
    let arm = ExecutionTarget::baseline(Architecture::Aarch64);
    let x86 = ExecutionTarget::baseline(Architecture::X86_64);
    let variants = [
        ArtifactVariant {
            target: arm,
            priority: 0,
            artifact_digest: [1; 32],
            object_key: b"arm/orders.elf",
        },
        ArtifactVariant {
            target: x86,
            priority: 0,
            artifact_digest: [2; 32],
            object_key: b"x86/orders.elf",
        },
        ArtifactVariant {
            target: ExecutionTarget {
                features: AARCH64_LSE,
                ..arm
            },
            priority: 1,
            artifact_digest: [3; 32],
            object_key: b"arm/orders-lse.elf",
        },
    ];
    let fields = DescriptorFields {
        sequence: 1,
        node_key: 0,
        variants: &variants,
        artifact_name: b"orders",
        stack_pages_per_thread: 4,
        max_threads: 1,
        shutdown_grace_ms: 5000,
        placement: charlotte_launch::placement::PlacementPolicy {
            replicas: 2,
            min_distinct_nodes: 2,
            ..charlotte_launch::placement::PlacementPolicy::singleton()
        },
        grants: &[],
    };
    let mut descriptor = vec![0; deployment::encoded_len(&fields).unwrap()];
    deployment::encode_unsigned(&fields, pair.pk.as_ref().try_into().unwrap(), &mut descriptor)
        .unwrap();
    let signature = pair.sk.sign(deployment::signature_digest(&descriptor).unwrap(), None);
    assert!(deployment::set_signature(&mut descriptor, signature.as_ref().try_into().unwrap()));
    let release = signed_release(&pair, b"mixed", 1, &[&descriptor]);
    let catalog = NameCatalog::new_with_deployment_key(pair.pk.as_ref().try_into().unwrap());
    for (node, target) in [(1, arm), (2, x86)] {
        catalog.apply(
            1,
            &encode_node_capacity(&NodeCapacityEntry {
                node_key: node,
                target,
                boot_nonce: 1,
                epoch: 1,
                free_frames: 10000,
                usable_frames: 20000,
                cpu_load_permille: 0,
            }),
        );
    }
    let capacity = catalog.node_capacity_view();
    let assignments = crate::operations_admission::resolve_release_assignments_with_capacity(
        &release,
        &[1, 2],
        2,
        &capacity,
    )
    .unwrap();
    let command = super::encode_release_replicas(&release, &assignments, &capacity).unwrap();
    assert_eq!(catalog.apply_with_result(1, &command), 1i64.to_le_bytes());
    let entry = catalog.deployment(b"orders").unwrap();
    assert_eq!(entry.replica_nodes, vec![1, 2]);
    assert_eq!(entry.replica_variants, vec![0, 1]);
    assert_eq!(
        decode_deployment_result(&encode_deployment_result(&entry).unwrap()),
        Some(entry.clone())
    );
    let restored = NameCatalog::new_with_deployment_key(pair.pk.as_ref().try_into().unwrap());
    restored.restore(&catalog.snapshot()).unwrap();
    assert_eq!(restored.deployment(b"orders"), Some(entry.clone()));
    // New CPU availability changes preference, never a committed assignment.
    restored.apply(
        1,
        &encode_node_capacity(&NodeCapacityEntry {
            node_key: 1,
            target: variants[2].target,
            boot_nonce: 2,
            epoch: 1,
            free_frames: 10000,
            usable_frames: 20000,
            cpu_load_permille: 0,
        }),
    );
    assert_eq!(restored.deployment(b"orders"), Some(entry));
    assert!(
        !restored
            .deployment(b"orders")
            .unwrap()
            .has_incompatible_pin(&restored.node_capacity_view())
    );
    let optimized =
        super::encode_reassign(b"orders", 1, &[1, 2], &descriptor, &restored.node_capacity_view())
            .unwrap();
    assert_eq!(restored.apply_with_result(1, &optimized), 2u64.to_le_bytes());
    assert_eq!(restored.deployment(b"orders").unwrap().replica_variants, vec![2, 1]);
    restored.apply(
        1,
        &encode_node_capacity(&NodeCapacityEntry {
            node_key: 1,
            target: arm,
            boot_nonce: 3,
            epoch: 1,
            free_frames: 10000,
            usable_frames: 20000,
            cpu_load_permille: 0,
        }),
    );
    assert!(
        restored
            .deployment(b"orders")
            .unwrap()
            .has_incompatible_pin(&restored.node_capacity_view())
    );
    let repair =
        super::encode_reassign(b"orders", 2, &[1, 2], &descriptor, &restored.node_capacity_view())
            .unwrap();
    assert_eq!(restored.apply_with_result(1, &repair), 3u64.to_le_bytes());
    assert_eq!(restored.deployment(b"orders").unwrap().replica_variants, vec![0, 1]);
    let mut incompatible = command.clone();
    *incompatible.last_mut().unwrap() = 255;
    let before = restored.snapshot();
    assert_eq!(
        restored.apply_with_result(1, &incompatible),
        crate::clusterctl::ERR_INCOMPATIBLE_TARGET.to_le_bytes()
    );
    assert_eq!(restored.snapshot(), before);
}

#[test]
fn cpu_profile_is_incarnation_bound_and_unknown_profiles_cannot_place() {
    use charlotte_launch::execution_target::{
        Architecture,
        ExecutionTarget,
    };
    let catalog = fixture_catalog();
    let original = catalog.node_capacity(1).unwrap();
    let changed = NodeCapacityEntry {
        target: ExecutionTarget::baseline(Architecture::X86_64),
        epoch: 2,
        ..original
    };
    catalog.apply(1, &encode_node_capacity(&changed));
    assert_eq!(catalog.node_capacity(1), Some(original));
    catalog.apply(
        1,
        &encode_node_capacity(&NodeCapacityEntry {
            boot_nonce: 2,
            ..changed
        }),
    );
    assert_eq!(catalog.node_capacity(1).unwrap().target, changed.target);
    let descriptor = signed_deployment(&fixture_pair(), b"orders", 1);
    let release = signed_release(&fixture_pair(), b"arm-only", 1, &[&descriptor]);
    let view = catalog.node_capacity_view();
    assert_eq!(
        crate::operations_admission::resolve_release_assignments_with_capacity(
            &release,
            &[1],
            1,
            &view
        ),
        Err(crate::operations_admission::AdmissionError::InsufficientCompatibleCapacity)
    );
    assert_eq!(
        crate::operations_admission::resolve_release_assignments_with_capacity(
            &release,
            &[99],
            99,
            &view
        ),
        Err(crate::operations_admission::AdmissionError::InsufficientCompatibleCapacity)
    );
    let mut variant = descriptor.clone();
    variant[24..32].copy_from_slice(&1u64.to_le_bytes());
    assert_eq!(
        crate::operations_admission::resolve_descriptor_assignments_with_capacity(
            &[&variant],
            &[1, 2],
            1,
            &view
        ),
        Err(crate::operations_admission::AdmissionError::InsufficientCompatibleCapacity)
    );
    assert_eq!(
        crate::operations_admission::resolve_descriptor_assignments_with_capacity(
            &[&variant],
            &[2],
            1,
            &view
        ),
        Err(crate::operations_admission::AdmissionError::UnsatisfiablePlacement)
    );
    assert_eq!(
        catalog.apply_with_result(1, &super::encode_deploy(&[], 2, 0)),
        crate::clusterctl::ERR_UNTRUSTED_DESCRIPTOR.to_le_bytes()
    );
}

#[test]
fn planner_filters_cpu_features_abi_and_every_node_before_ranking() {
    use charlotte_launch::{
        deployment,
        execution_target::{
            AARCH64_LSE,
            Architecture,
            ExecutionTarget,
        },
        placement::{
            EVERY_ELIGIBLE_NODE,
            PlacementPolicy,
        },
    };

    use crate::operations_admission::{
        AdmissionError,
        resolve_descriptor_assignments_with_capacity,
    };
    let catalog = fixture_catalog();
    let descriptor = signed_deployment(&fixture_pair(), b"orders", 1);
    let target_offset = deployment::HEADER_LEN + b"orders".len();
    let mut demanding = descriptor.clone();
    demanding[target_offset + 8..target_offset + 16].copy_from_slice(&AARCH64_LSE.to_le_bytes());
    let mut capacity = catalog.node_capacity_view();
    assert_eq!(
        resolve_descriptor_assignments_with_capacity(&[&demanding], &[1, 2], 1, &capacity),
        Err(AdmissionError::InsufficientCompatibleCapacity)
    );
    capacity.get_mut(&2).unwrap().target = Some(ExecutionTarget {
        features: AARCH64_LSE,
        ..ExecutionTarget::baseline(Architecture::Aarch64)
    });
    assert_eq!(
        resolve_descriptor_assignments_with_capacity(&[&demanding], &[1, 2], 1, &capacity).unwrap(),
        vec![vec![2]]
    );
    capacity.get_mut(&2).unwrap().target.as_mut().unwrap().runtime_abi = 2;
    assert_eq!(
        resolve_descriptor_assignments_with_capacity(&[&demanding], &[1, 2], 1, &capacity),
        Err(AdmissionError::InsufficientCompatibleCapacity)
    );
    let every = signed_deployment_with_policy(
        &fixture_pair(),
        b"orders",
        1,
        PlacementPolicy {
            flags: EVERY_ELIGIBLE_NODE,
            replicas: 0,
            ..PlacementPolicy::singleton()
        },
    );
    capacity.get_mut(&1).unwrap().target = Some(ExecutionTarget::baseline(Architecture::X86_64));
    capacity.get_mut(&2).unwrap().target = Some(ExecutionTarget::baseline(Architecture::Aarch64));
    assert_eq!(
        resolve_descriptor_assignments_with_capacity(&[&every], &[1, 2, 99], 1, &capacity).unwrap(),
        vec![vec![2]]
    );
}

#[test]
fn descriptor_only_command_rejects_retired_tag_and_invalid_pins_without_mutation() {
    let catalog = fixture_catalog();
    let descriptor = signed_deployment(&fixture_pair(), b"orders", 1);
    let command = super::encode_deploy(&descriptor, 4, 0);
    let before = catalog.snapshot();
    let mut retired = command.clone();
    retired[0] = 0x05;
    assert!(catalog.apply_with_result(1, &retired).is_empty());
    for length in 0..command.len() {
        catalog.apply(1, &command[..length]);
        assert_eq!(catalog.snapshot(), before);
    }
    let mut extended = command.clone();
    extended.push(0);
    assert!(catalog.apply_with_result(1, &extended).is_empty());
    for (node, variant) in [(0, 0), (4, 1)] {
        assert_eq!(
            catalog.apply_with_result(1, &super::encode_deploy(&descriptor, node, variant)),
            crate::clusterctl::ERR_INCOMPATIBLE_TARGET.to_le_bytes()
        );
    }
    let mut tampered = command.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert_eq!(
        catalog.apply_with_result(1, &tampered),
        crate::clusterctl::ERR_UNTRUSTED_DESCRIPTOR.to_le_bytes()
    );
    assert_eq!(catalog.snapshot(), before);
    assert_eq!(catalog.apply_with_result(1, &command), 1u64.to_le_bytes());
    assert_eq!(catalog.apply_with_result(2, &command), 1u64.to_le_bytes());
}

#[test]
fn query_tag_and_signed_variant_indices_reject_legacy_or_malformed_results() {
    let entry = DeploymentEntry {
        generation: 1,
        descriptor: signed_deployment(&fixture_pair(), b"orders", 1),
        replica_nodes: vec![4],
        replica_variants: vec![0],
    };
    let current = encode_deployment_result(&entry).unwrap();
    let mut legacy = vec![0; 56];
    legacy[..8].copy_from_slice(&1u64.to_le_bytes());
    legacy.extend_from_slice(&current[16..]);
    assert!(decode_deployment_result(&legacy).is_none());
    let mut unknown_variant = current.clone();
    let len = unknown_variant.len();
    unknown_variant[len - 2..].copy_from_slice(&1u16.to_le_bytes());
    assert!(decode_deployment_result(&unknown_variant).is_none());
    assert_eq!(decode_deployment_result(&current), Some(entry));
}

#[test]
fn descriptor_relay_rejects_retired_tags_truncation_and_extra_bytes() {
    let request = crate::rdeploy::Request {
        session: 3,
        request_id: 9,
        caller: b"charlotte:1234abcd".to_vec(),
        descriptor: signed_deployment(&fixture_pair(), b"orders", 1),
    };
    let payload = crate::rdeploy::encode_request(&request).unwrap();
    let frame = [&[crate::rdeploy::TAG_REQUEST], payload.as_slice()].concat();
    for length in 0..frame.len() {
        assert!(crate::rdeploy::decode_request(&frame[..length]).is_none());
    }
    let mut retired = frame.clone();
    retired[0] = 0x17;
    assert!(crate::rdeploy::decode_request(&retired).is_none());
    let mut extended = frame.clone();
    extended.push(0);
    assert!(crate::rdeploy::decode_request(&extended).is_none());
    assert_eq!(crate::rdeploy::decode_request(&frame), Some(request));
    let reply = crate::rdeploy::encode_reply(3, 9, 1);
    assert!(crate::rdeploy::decode_reply(&[&[0x18], reply.as_slice()].concat()).is_none());
}

#[test]
fn snapshot_descriptor_name_binding_rejects_before_any_live_publication() {
    let source = fixture_catalog();
    source.apply(1, &deploy_fixture(b"orders", 4, 1));
    let mut corrupted = source.snapshot();
    // Empty registrations, then one deployment's six-byte map key.
    assert_eq!(&corrupted[20..26], b"orders");
    corrupted[20..26].copy_from_slice(b"otherx");
    let target = fixture_catalog();
    target.apply(1, &encode_register(b"keep", b"charlotte:1234abcd"));
    let before = target.snapshot();
    assert_eq!(
        target.restore(&corrupted),
        Err(catten_graft::state_machine::SnapshotError::Malformed)
    );
    assert_eq!(target.snapshot(), before);
}
