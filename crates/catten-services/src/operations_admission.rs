//! Pure leader-side admission for encrypted operational bundles.
//!
//! Keeping signature/context/expiry verification in one testable function
//! prevents the HTTP ingress, follower relay, and Raft command construction
//! from drifting into three subtly different policies.

extern crate alloc;

use alloc::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    vec,
    vec::Vec,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    Expired,
    Invalid,
    TooLarge,
    WrongCluster,
    WrongOperationsKey,
    WrongRecipient,
    WrongReleaseKey,
    /// Too few eligible nodes exist for the signed replica policy.
    UnsatisfiablePlacement,
    /// Enough nodes exist, but none can host the declared resource demand.
    InsufficientCapacity,
}

/// Sentinel for a node that has not reported CPU load yet.
pub const CPU_LOAD_UNKNOWN: u16 = u16::MAX;

/// Coarse per-node pressure sample used by the deterministic resolver.
///
/// The leader fills this from committed capacity reports; an absent node is
/// treated as unknown (neutral), never as exhausted, so mixed-version clusters
/// keep making progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeCapacity {
    pub free_frames: u64,
    pub usable_frames: u64,
    /// Recent CPU occupancy in permille, or [`CPU_LOAD_UNKNOWN`].
    pub cpu_load_permille: u16,
}

impl Default for NodeCapacity {
    fn default() -> Self {
        Self {
            free_frames: 0,
            usable_frames: 0,
            cpu_load_permille: CPU_LOAD_UNKNOWN,
        }
    }
}

pub type NodeCapacityView = BTreeMap<u64, NodeCapacity>;

/// Free frames below one sixteenth of usable memory exclude a node from new
/// placements. Ranking prefers ample, then unknown or moderate, then low.
fn memory_bucket_of(sample: &NodeCapacity) -> u8 {
    const RESERVE_DIVISOR: u64 = 16;
    if sample.usable_frames == 0 {
        return 2;
    }
    let free = sample.free_frames.min(sample.usable_frames);
    let reserve = (sample.usable_frames / RESERVE_DIVISOR).max(1);
    if free < reserve {
        0
    } else if free < sample.usable_frames / 8 {
        1
    } else if free < sample.usable_frames / 4 {
        2
    } else {
        3
    }
}

/// Soft CPU-load bucket. Heavy occupancy ranks last but never excludes a
/// node, so placement still proceeds when every node is busy.
fn cpu_bucket_of(sample: &NodeCapacity) -> u8 {
    match sample.cpu_load_permille {
        CPU_LOAD_UNKNOWN => 2,
        load if load >= 950 => 0,
        load if load >= 750 => 1,
        load if load >= 500 => 2,
        _ => 3,
    }
}

fn capacity_bucket(capacity: &NodeCapacityView, node: u64) -> u8 {
    capacity.get(&node).map_or(2, memory_bucket_of)
}

fn cpu_bucket(capacity: &NodeCapacityView, node: u64) -> u8 {
    capacity.get(&node).map_or(2, cpu_bucket_of)
}

/// Whether a fresh sample is different enough from the committed one to
/// justify a Raft command.
///
/// Workloads drift continuously, so committing every five-second sample would
/// grow the log without improving placement. A report is committed when the
/// memory or CPU bucket changes, when usable memory changes, or when free
/// frames move by more than one sixteenth of usable memory. An absent previous
/// sample is always committed, so a cold cluster seeds its table in one
/// interval.
pub fn capacity_sample_changed(previous: Option<&NodeCapacity>, next: &NodeCapacity) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    if previous.usable_frames != next.usable_frames {
        return true;
    }
    if memory_bucket_of(previous) != memory_bucket_of(next)
        || cpu_bucket_of(previous) != cpu_bucket_of(next)
    {
        return true;
    }
    previous.free_frames.abs_diff(next.free_frames) > (next.usable_frames / 16).max(1)
}

/// Combined pressure rank: the worse of memory and CPU pressure.
fn pressure_bucket(capacity: &NodeCapacityView, node: u64) -> u8 {
    capacity_bucket(capacity, node).min(cpu_bucket(capacity, node))
}

/// Fixed pages the loader maps for every domain besides its stack and heap:
/// config, status, input, the default completion queue, and four shard rings.
const DOMAIN_RUNTIME_PAGES: u64 = 8;

/// Worst-case resident memory one descriptor implies on a node: every thread's
/// stack budget plus the fixed runtime pages.
///
/// The heap is demand-committed rather than reserved, so it is not part of the
/// demand; an explicit signed heap request arrives with a future descriptor
/// version, and storage or processor requests need their own node sensors.
fn descriptor_memory_demand(
    descriptor: &charlotte_launch::deployment::DeploymentDescriptor<'_>,
) -> u64 {
    u64::from(descriptor.max_threads)
        .saturating_mul(u64::from(descriptor.stack_pages_per_thread))
        .saturating_add(DOMAIN_RUNTIME_PAGES)
}

/// Whether a node's committed free frames cover `demand_frames` plus its own
/// free-frame reserve. Unknown capacity is optimistic so mixed-version
/// clusters keep making progress.
fn node_can_host(capacity: &NodeCapacityView, node: u64, demand_frames: u64) -> bool {
    let Some(sample) = capacity.get(&node) else {
        return true;
    };
    if sample.usable_frames == 0 {
        return true;
    }
    let reserve = (sample.usable_frames / 16).max(1);
    sample.free_frames.min(sample.usable_frames) >= demand_frames.saturating_add(reserve)
}

/// Resolve each signed component policy to a concrete, unique node set.
/// Fixed singletons retain the historical leader placement. Replica choices
/// use a stable per-artifact ranking; affinity groups share a ranking seed and
/// anti-affinity groups require unused nodes. Components are planned in
/// artifact-name order so release ordering cannot change the result.
pub fn resolve_release_assignments(
    release_bytes: &[u8],
    eligible_nodes: &[u64],
    automatic_node: u64,
) -> Result<Vec<Vec<u64>>, AdmissionError> {
    resolve_release_assignments_with_capacity(
        release_bytes,
        eligible_nodes,
        automatic_node,
        &NodeCapacityView::new(),
    )
}

/// Capacity-aware variant of [`resolve_release_assignments`].
pub fn resolve_release_assignments_with_capacity(
    release_bytes: &[u8],
    eligible_nodes: &[u64],
    automatic_node: u64,
    capacity: &NodeCapacityView,
) -> Result<Vec<Vec<u64>>, AdmissionError> {
    let release =
        charlotte_launch::release::decode(release_bytes).ok_or(AdmissionError::Invalid)?;
    let descriptors = release.descriptors().collect::<Vec<_>>();
    resolve_descriptor_assignments_with_capacity(
        &descriptors,
        eligible_nodes,
        automatic_node,
        capacity,
    )
}

/// Resolve an already decoded, stable descriptor ordering. The controller
/// uses this to recompute desired sets after committed membership changes.
pub fn resolve_descriptor_assignments(
    descriptor_bytes: &[&[u8]],
    eligible_nodes: &[u64],
    automatic_node: u64,
) -> Result<Vec<Vec<u64>>, AdmissionError> {
    resolve_descriptor_assignments_with_capacity(
        descriptor_bytes,
        eligible_nodes,
        automatic_node,
        &NodeCapacityView::new(),
    )
}

/// Capacity-aware variant of [`resolve_descriptor_assignments`].
///
/// Nodes below the free-frame reserve are excluded from new placements, and
/// ranking prefers the least-pressured bucket before the stable hash order.
/// Unknown nodes stay eligible and rank as moderate.
pub fn resolve_descriptor_assignments_with_capacity(
    descriptor_bytes: &[&[u8]],
    eligible_nodes: &[u64],
    automatic_node: u64,
    capacity: &NodeCapacityView,
) -> Result<Vec<Vec<u64>>, AdmissionError> {
    let mut eligible = eligible_nodes.iter().copied().filter(|node| *node != 0).collect::<Vec<_>>();
    eligible.sort_unstable();
    eligible.dedup();
    let descriptors = descriptor_bytes
        .iter()
        .map(|bytes| charlotte_launch::deployment::decode(bytes).ok_or(AdmissionError::Invalid))
        .collect::<Result<Vec<_>, _>>()?;
    let mut order = (0..descriptors.len()).collect::<Vec<_>>();
    order.sort_unstable_by(|left, right| {
        descriptors[*left]
            .artifact_name
            .cmp(descriptors[*right].artifact_name)
            .then_with(|| left.cmp(right))
    });
    let mut anti_affinity: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    let mut assignments = vec![None; descriptors.len()];
    for index in order {
        let descriptor = descriptors[index];
        if descriptor.node_key != 0 {
            assignments[index] = Some(vec![descriptor.node_key]);
            continue;
        }
        let policy = descriptor.placement;
        policy.validate_shape().map_err(|_| AdmissionError::Invalid)?;
        // Feasible targets for this component's declared memory reservation.
        let demand = descriptor_memory_demand(&descriptor);
        let candidates = eligible
            .iter()
            .copied()
            .filter(|node| {
                capacity_bucket(capacity, *node) > 0 && node_can_host(capacity, *node, demand)
            })
            .collect::<Vec<_>>();
        if policy == charlotte_launch::placement::PlacementPolicy::singleton() {
            let selected = if automatic_node != 0 && candidates.contains(&automatic_node) {
                automatic_node
            } else {
                candidates
                    .iter()
                    .copied()
                    .max_by_key(|node| {
                        let mut score_input = descriptor.artifact_name.to_vec();
                        score_input.extend_from_slice(&node.to_le_bytes());
                        (
                            pressure_bucket(capacity, *node),
                            charlotte_launch::fnv1a(&score_input),
                            core::cmp::Reverse(*node),
                        )
                    })
                    .ok_or(
                        if eligible.is_empty() {
                            AdmissionError::UnsatisfiablePlacement
                        } else {
                            AdmissionError::InsufficientCapacity
                        },
                    )?
            };
            assignments[index] = Some(vec![selected]);
            continue;
        }
        let every = policy.flags & charlotte_launch::placement::EVERY_ELIGIBLE_NODE != 0;
        if every && candidates.is_empty() {
            return Err(if eligible.is_empty() {
                AdmissionError::UnsatisfiablePlacement
            } else {
                AdmissionError::InsufficientCapacity
            });
        }
        let wanted = if every {
            candidates.len()
        } else {
            usize::from(policy.replicas)
        };
        if wanted == 0 || wanted > eligible.len() {
            return Err(AdmissionError::UnsatisfiablePlacement);
        }
        if wanted > candidates.len() {
            return Err(AdmissionError::InsufficientCapacity);
        }
        let seed = if policy.flags & charlotte_launch::placement::COLOCATE_AFFINITY_GROUP != 0 {
            policy.affinity_group.to_le_bytes().to_vec()
        } else {
            descriptor.artifact_name.to_vec()
        };
        let used = anti_affinity.get(&policy.anti_affinity_group);
        let mut ranked = candidates
            .iter()
            .copied()
            .filter(|node| {
                policy.anti_affinity_group == 0 || !used.is_some_and(|nodes| nodes.contains(node))
            })
            .collect::<Vec<_>>();
        if ranked.len() < wanted {
            return Err(AdmissionError::UnsatisfiablePlacement);
        }
        ranked.sort_unstable_by_key(|node| {
            let mut score_input = seed.clone();
            score_input.extend_from_slice(&node.to_le_bytes());
            (
                core::cmp::Reverse(pressure_bucket(capacity, *node)),
                core::cmp::Reverse(charlotte_launch::fnv1a(&score_input)),
                *node,
            )
        });
        let mut selected = ranked.into_iter().take(wanted).collect::<Vec<_>>();
        selected.sort_unstable();
        if policy.anti_affinity_group != 0 {
            anti_affinity
                .entry(policy.anti_affinity_group)
                .or_default()
                .extend(selected.iter().copied());
        }
        assignments[index] = Some(selected);
    }
    assignments.into_iter().collect::<Option<Vec<_>>>().ok_or(AdmissionError::Invalid)
}

/// Verify both authorities and all operational context, resolve automatic
/// component placement, and return the compact command suitable for Raft.
///
/// This function must be called by the current leader immediately before
/// submission. `now_unix_seconds` must come from the trusted cluster time
/// service; callers fail closed when UTC is unavailable.
pub fn verify_and_encode(
    bundle_bytes: &[u8],
    trust: &charlotte_launch::trust::AdmissionTrust,
    now_unix_seconds: u64,
    eligible_nodes: &[u64],
    automatic_node: u64,
) -> Result<Vec<u8>, AdmissionError> {
    verify_and_encode_with_capacity(
        bundle_bytes,
        trust,
        now_unix_seconds,
        eligible_nodes,
        automatic_node,
        &NodeCapacityView::new(),
    )
}

/// Capacity-aware variant of [`verify_and_encode`].
pub fn verify_and_encode_with_capacity(
    bundle_bytes: &[u8],
    trust: &charlotte_launch::trust::AdmissionTrust,
    now_unix_seconds: u64,
    eligible_nodes: &[u64],
    automatic_node: u64,
    capacity: &NodeCapacityView,
) -> Result<Vec<u8>, AdmissionError> {
    use charlotte_launch::operations_bundle::VerifyOutcome;

    if bundle_bytes.len() > charlotte_launch::operations_bundle::MAX_BUNDLE_LEN {
        return Err(AdmissionError::TooLarge);
    }
    let outcome = charlotte_launch::operations_bundle::verify(
        bundle_bytes,
        &trust.deployment_key,
        &trust.operations_key,
        &trust.recipient_key,
        &trust.cluster_id,
        now_unix_seconds,
    );
    match outcome {
        VerifyOutcome::Valid => {}
        VerifyOutcome::Expired => return Err(AdmissionError::Expired),
        VerifyOutcome::Invalid => return Err(AdmissionError::Invalid),
        VerifyOutcome::WrongCluster => return Err(AdmissionError::WrongCluster),
        VerifyOutcome::WrongOperationalKey => return Err(AdmissionError::WrongOperationsKey),
        VerifyOutcome::WrongRecipient => return Err(AdmissionError::WrongRecipient),
        VerifyOutcome::WrongReleaseKey => return Err(AdmissionError::WrongReleaseKey),
    }
    let bundle =
        charlotte_launch::operations_bundle::decode(bundle_bytes).ok_or(AdmissionError::Invalid)?;
    let assignments = resolve_release_assignments_with_capacity(
        bundle.release,
        eligible_nodes,
        automatic_node,
        capacity,
    )?;
    crate::name_catalog::encode_release_replicas_with_operations(bundle_bytes, &assignments)
        .ok_or(AdmissionError::TooLarge)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use catten_graft::state_machine::StateMachine;
    use charlotte_launch::{
        deployment::{
            self,
            DescriptorFields,
        },
        operations::{
            self,
            EnvelopeFields,
            PROFILE_KIND_KAFKA,
        },
        operations_bundle::{
            self,
            BindingFields,
            BundleFields,
        },
        release::{
            self,
            ReleaseFields,
        },
        trust::AdmissionTrust,
    };
    use ed25519_compact::{
        KeyPair,
        Signature,
    };
    use rand_core_10::UnwrapErr;

    use super::*;
    use crate::{
        name_catalog::NameCatalog,
        roperations,
    };

    struct Fixture {
        bundle: Vec<u8>,
        trust: AdmissionTrust,
    }

    fn sign_deployment(bytes: &mut [u8], pair: &KeyPair) {
        let signature: Signature = pair.sk.sign(deployment::signature_digest(bytes).unwrap(), None);
        assert!(deployment::set_signature(bytes, signature.as_ref().try_into().unwrap()));
    }

    fn policy_release(
        pair: &KeyPair,
        components: &[(&[u8], charlotte_launch::placement::PlacementPolicy)],
    ) -> Vec<u8> {
        let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
        let descriptors = components
            .iter()
            .map(|(name, placement)| {
                let fields = DescriptorFields {
                    sequence: 1,
                    node_key: 0,
                    artifact_digest: [name[0]; 32],
                    artifact_name: name,
                    stack_pages_per_thread: 4,
                    max_threads: 4,
                    shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
                    placement: *placement,
                    object_key: name,
                    grants: &[],
                };
                let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
                deployment::encode_unsigned(&fields, public, &mut bytes).unwrap();
                sign_deployment(&mut bytes, pair);
                bytes
            })
            .collect::<Vec<_>>();
        let refs = descriptors.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let fields = ReleaseFields {
            sequence: 1,
            release_name: b"placement-test",
            descriptors: &refs,
        };
        let mut release = vec![0; release::encoded_len(&fields).unwrap()];
        release::encode_unsigned(&fields, public, &mut release).unwrap();
        let signature: Signature = pair.sk.sign(release::signature_digest(&release).unwrap(), None);
        assert!(release::set_signature(&mut release, signature.as_ref().try_into().unwrap()));
        release
    }

    fn fixture(profile_len: usize, expiry: u64) -> Fixture {
        let release_pair = KeyPair::from_seed([0x31; 32].into());
        let operational_pair = KeyPair::from_seed([0x42; 32].into());
        let release_public: &[u8; 32] = release_pair.pk.as_ref().try_into().unwrap();
        let operational_public: &[u8; 32] = operational_pair.pk.as_ref().try_into().unwrap();
        let descriptor_fields = DescriptorFields {
            sequence: 1,
            node_key: 0,
            artifact_digest: [0x77; 32],
            artifact_name: b"kafka",
            stack_pages_per_thread: 32,
            max_threads: 16,
            shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
            placement: charlotte_launch::placement::PlacementPolicy::singleton(),
            object_key: b"releases/kafka.elf",
            grants: &[],
        };
        let mut descriptor = vec![0; deployment::encoded_len(&descriptor_fields).unwrap()];
        deployment::encode_unsigned(&descriptor_fields, release_public, &mut descriptor).unwrap();
        sign_deployment(&mut descriptor, &release_pair);
        let descriptors = [descriptor.as_slice()];
        let release_fields = ReleaseFields {
            sequence: 1,
            release_name: b"orders",
            descriptors: &descriptors,
        };
        let mut release = vec![0; release::encoded_len(&release_fields).unwrap()];
        release::encode_unsigned(&release_fields, release_public, &mut release).unwrap();
        let signature: Signature =
            release_pair.sk.sign(release::signature_digest(&release).unwrap(), None);
        assert!(release::set_signature(&mut release, signature.as_ref().try_into().unwrap()));

        let mut rng = UnwrapErr(getrandom::SysRng);
        let (_, recipient_public) = operations::generate_recipient_keypair(&mut rng);
        let cluster_id = [0x11; 32];
        let envelope_fields = EnvelopeFields {
            sequence: 1,
            expires_unix_seconds: expiry,
            profile_kind: PROFILE_KIND_KAFKA,
            cluster_id,
            release_digest: charlotte_launch::sha256::digest(&release),
            profile_name: b"kafka/orders/transactional",
        };
        let profile = vec![0x5a; profile_len];
        let mut envelope =
            vec![0; operations::encoded_len(&envelope_fields, profile.len()).unwrap()];
        operations::seal_unsigned(
            &envelope_fields,
            &profile,
            &recipient_public,
            operational_public,
            &mut rng,
            &mut envelope,
        )
        .unwrap();
        let signature: Signature =
            operational_pair.sk.sign(operations::signature_digest(&envelope).unwrap(), None);
        assert!(operations::set_signature(&mut envelope, signature.as_ref().try_into().unwrap()));
        let bindings = [BindingFields {
            target_artifact: b"kafka",
            object_key: b"operations/orders-kafka.cops",
            envelope: &envelope,
        }];
        let bundle_fields = BundleFields {
            sequence: 1,
            cluster_id,
            release: &release,
            bindings: &bindings,
        };
        let mut bundle = vec![0; operations_bundle::encoded_len(&bundle_fields).unwrap()];
        operations_bundle::encode_unsigned(
            &bundle_fields,
            operational_public,
            &recipient_public,
            &mut bundle,
        )
        .unwrap();
        let binding_signature: Signature = operational_pair
            .sk
            .sign(operations_bundle::binding_signature_digest(&bundle, 0).unwrap(), None);
        assert!(operations_bundle::set_binding_signature(
            &mut bundle,
            0,
            binding_signature.as_ref().try_into().unwrap()
        ));
        let signature: Signature =
            operational_pair.sk.sign(operations_bundle::signature_digest(&bundle).unwrap(), None);
        assert!(operations_bundle::set_signature(
            &mut bundle,
            signature.as_ref().try_into().unwrap()
        ));
        Fixture {
            bundle,
            trust: AdmissionTrust {
                sequence: 1,
                cluster_id,
                artifact_key: [0x22; 32],
                deployment_key: *release_public,
                operations_key: *operational_public,
                recipient_key: recipient_public,
            },
        }
    }

    #[test]
    fn leader_verification_produces_only_compact_replay_fenced_state() {
        let fixture = fixture(128, 2_000_000_000);
        let command =
            verify_and_encode(&fixture.bundle, &fixture.trust, 1_900_000_000, &[0x1234], 0x1234)
                .unwrap();
        assert!(command.len() <= catten_graft::types::MAX_COMMAND_BYTES);
        assert!(!command.windows(32).any(|window| window.iter().all(|byte| *byte == 0x5a)));

        let catalog = NameCatalog::new_with_deployment_key(fixture.trust.deployment_key);
        assert_eq!(catalog.apply_with_result(1, &command), 1i64.to_le_bytes());
        let binding = catalog.operational_binding(b"kafka/orders/transactional").unwrap();
        assert_eq!(binding.target_artifact, b"kafka");
        assert_eq!(binding.object_key, b"operations/orders-kafka.cops");
        assert_eq!(binding.sequence, 1);
    }

    #[test]
    fn planner_resolves_distinct_replicas_and_fails_when_capacity_is_short() {
        let pair = KeyPair::from_seed([0x53; 32].into());
        let policy = charlotte_launch::placement::PlacementPolicy {
            replicas: 3,
            max_instances_per_node: 1,
            min_distinct_nodes: 3,
            flags: charlotte_launch::placement::SPREAD_REPLICAS,
            affinity_group: 0,
            anti_affinity_group: 0,
        };
        let release = policy_release(&pair, &[(b"orders", policy)]);
        let assignments = resolve_release_assignments(&release, &[4, 2, 3, 1], 4).unwrap();
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].len(), 3);
        assert!(assignments[0].windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            resolve_release_assignments(&release, &[1, 2], 1),
            Err(AdmissionError::UnsatisfiablePlacement)
        );
    }

    #[test]
    fn singleton_prefers_the_leader_but_can_leave_a_draining_leader() {
        let pair = KeyPair::from_seed([0x55; 32].into());
        let release = policy_release(
            &pair,
            &[(b"orders", charlotte_launch::placement::PlacementPolicy::singleton())],
        );
        assert_eq!(resolve_release_assignments(&release, &[1, 2, 3], 2).unwrap(), vec![vec![2]]);
        let fallback = resolve_release_assignments(&release, &[1, 3], 2).unwrap();
        assert_eq!(fallback[0].len(), 1);
        assert!(matches!(fallback[0][0], 1 | 3));
    }

    #[test]
    fn capacity_gates_and_ranks_new_placements() {
        let pair = KeyPair::from_seed([0x57; 32].into());
        let policy = charlotte_launch::placement::PlacementPolicy {
            replicas: 1,
            max_instances_per_node: 1,
            min_distinct_nodes: 1,
            flags: charlotte_launch::placement::SPREAD_REPLICAS,
            affinity_group: 0,
            anti_affinity_group: 0,
        };
        let release = policy_release(&pair, &[(b"orders", policy)]);

        // Unknown capacity leaves the historical ranking untouched.
        let baseline = resolve_release_assignments(&release, &[1, 2, 3], 0).unwrap();
        assert_eq!(
            resolve_release_assignments_with_capacity(
                &release,
                &[1, 2, 3],
                0,
                &NodeCapacityView::new(),
            )
            .unwrap(),
            baseline
        );

        // A pressured node loses to ample peers even when its hash wins.
        let ample = NodeCapacity {
            free_frames: 900,
            usable_frames: 1000,
            cpu_load_permille: CPU_LOAD_UNKNOWN,
        };
        let low = NodeCapacity {
            free_frames: 60,
            usable_frames: 1000,
            cpu_load_permille: CPU_LOAD_UNKNOWN,
        };
        let pressured = baseline[0][0];
        let mut capacity = NodeCapacityView::new();
        for node in [1, 2, 3] {
            capacity.insert(node, ample);
        }
        capacity.insert(pressured, low);
        let selected =
            resolve_release_assignments_with_capacity(&release, &[1, 2, 3], 0, &capacity).unwrap();
        assert_ne!(selected[0][0], pressured);

        // Below the reserve, a node is excluded from new placements.
        let exhausted = NodeCapacity {
            free_frames: 10,
            usable_frames: 1000,
            cpu_load_permille: CPU_LOAD_UNKNOWN,
        };
        let mut capacity = NodeCapacityView::new();
        for node in [1, 2, 3] {
            capacity.insert(node, ample);
        }
        capacity.insert(1, exhausted);
        assert_eq!(
            resolve_release_assignments_with_capacity(&release, &[1], 0, &capacity),
            Err(AdmissionError::InsufficientCapacity)
        );
        let selected =
            resolve_release_assignments_with_capacity(&release, &[1, 2, 3], 0, &capacity).unwrap();
        assert_ne!(selected[0][0], 1);
    }

    #[test]
    fn capacity_sample_change_is_hysteretic() {
        let sample = NodeCapacity {
            free_frames: 800,
            usable_frames: 1000,
            cpu_load_permille: 100,
        };
        assert!(capacity_sample_changed(None, &sample));
        assert!(!capacity_sample_changed(Some(&sample), &sample));
        let drifted = NodeCapacity {
            free_frames: 790,
            ..sample
        };
        assert!(!capacity_sample_changed(Some(&sample), &drifted));
        let bucket = NodeCapacity {
            free_frames: 60,
            ..sample
        };
        assert!(capacity_sample_changed(Some(&sample), &bucket));
        let moved = NodeCapacity {
            free_frames: 700,
            ..sample
        };
        assert!(capacity_sample_changed(Some(&sample), &moved));
        let busy = NodeCapacity {
            cpu_load_permille: 800,
            ..sample
        };
        assert!(capacity_sample_changed(Some(&sample), &busy));
        let resized = NodeCapacity {
            usable_frames: 2000,
            ..sample
        };
        assert!(capacity_sample_changed(Some(&sample), &resized));
    }

    #[test]
    fn cpu_pressure_demotes_but_never_excludes() {
        let pair = KeyPair::from_seed([0x5a; 32].into());
        let policy = charlotte_launch::placement::PlacementPolicy {
            replicas: 1,
            max_instances_per_node: 1,
            min_distinct_nodes: 1,
            flags: charlotte_launch::placement::SPREAD_REPLICAS,
            affinity_group: 0,
            anti_affinity_group: 0,
        };
        let release = policy_release(&pair, &[(b"orders", policy)]);
        let baseline = resolve_release_assignments(&release, &[1, 2, 3], 0).unwrap();
        let busy = baseline[0][0];
        let idle = |node: u64| {
            (
                node,
                NodeCapacity {
                    free_frames: 900,
                    usable_frames: 1000,
                    cpu_load_permille: 100,
                },
            )
        };
        let mut capacity = NodeCapacityView::from_iter([1, 2, 3].map(idle));
        capacity.insert(
            busy,
            NodeCapacity {
                free_frames: 900,
                usable_frames: 1000,
                cpu_load_permille: 980,
            },
        );
        let selected =
            resolve_release_assignments_with_capacity(&release, &[1, 2, 3], 0, &capacity).unwrap();
        assert_ne!(selected[0][0], busy);

        // A fully occupied node set still places; CPU is a preference only.
        let occupied = NodeCapacityView::from_iter([1, 2, 3].map(|node| {
            (
                node,
                NodeCapacity {
                    free_frames: 900,
                    usable_frames: 1000,
                    cpu_load_permille: 980,
                },
            )
        }));
        assert_eq!(
            resolve_release_assignments_with_capacity(&release, &[1, 2, 3], 0, &occupied)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn declared_memory_demand_fails_only_on_capacity() {
        let pair = KeyPair::from_seed([0x59; 32].into());
        let policy = charlotte_launch::placement::PlacementPolicy {
            replicas: 1,
            max_instances_per_node: 1,
            min_distinct_nodes: 1,
            flags: charlotte_launch::placement::SPREAD_REPLICAS,
            affinity_group: 0,
            anti_affinity_group: 0,
        };
        let release = policy_release(&pair, &[(b"orders", policy)]);
        // Demand = 4 threads x 4 stack pages + 8 runtime pages = 24 frames.
        let tight = NodeCapacity {
            free_frames: 30,
            usable_frames: 1000,
            cpu_load_permille: CPU_LOAD_UNKNOWN,
        };
        let mut capacity = NodeCapacityView::new();
        capacity.insert(1, tight);
        assert_eq!(
            resolve_release_assignments_with_capacity(&release, &[1], 0, &capacity),
            Err(AdmissionError::InsufficientCapacity)
        );

        let roomy = NodeCapacity {
            free_frames: 200,
            usable_frames: 1000,
            cpu_load_permille: CPU_LOAD_UNKNOWN,
        };
        let mut capacity = NodeCapacityView::new();
        capacity.insert(1, roomy);
        assert_eq!(
            resolve_release_assignments_with_capacity(&release, &[1], 0, &capacity).unwrap(),
            vec![vec![1]]
        );

        // Too few eligible nodes is a spread failure, not a capacity failure.
        assert_eq!(
            resolve_release_assignments_with_capacity(&release, &[0], 0, &capacity),
            Err(AdmissionError::UnsatisfiablePlacement)
        );
    }

    #[test]
    fn planner_honours_affinity_and_anti_affinity_groups() {
        let pair = KeyPair::from_seed([0x54; 32].into());
        let colocated = charlotte_launch::placement::PlacementPolicy {
            replicas: 1,
            max_instances_per_node: 1,
            min_distinct_nodes: 1,
            flags: charlotte_launch::placement::COLOCATE_AFFINITY_GROUP,
            affinity_group: 7,
            anti_affinity_group: 0,
        };
        let release = policy_release(&pair, &[(b"receive", colocated), (b"publish", colocated)]);
        let assignments = resolve_release_assignments(&release, &[1, 2, 3], 1).unwrap();
        assert_eq!(assignments[0], assignments[1]);

        let separated = charlotte_launch::placement::PlacementPolicy {
            flags: 0,
            affinity_group: 0,
            anti_affinity_group: 9,
            ..colocated
        };
        let release = policy_release(&pair, &[(b"receive", separated), (b"publish", separated)]);
        let assignments = resolve_release_assignments(&release, &[1, 2, 3], 1).unwrap();
        assert_ne!(assignments[0], assignments[1]);
        assert_eq!(
            resolve_release_assignments(&release, &[1], 1),
            Err(AdmissionError::UnsatisfiablePlacement)
        );

        let reversed = policy_release(&pair, &[(b"publish", separated), (b"receive", separated)]);
        let reversed_assignments = resolve_release_assignments(&reversed, &[1, 2, 3], 1).unwrap();
        assert_eq!(assignments[0], reversed_assignments[1]);
        assert_eq!(assignments[1], reversed_assignments[0]);
    }

    #[test]
    fn leader_fails_closed_on_expiry_and_wrong_authority_context() {
        let fixture = fixture(32, 2_000_000_000);
        assert_eq!(
            verify_and_encode(&fixture.bundle, &fixture.trust, 2_000_000_001, &[1], 1),
            Err(AdmissionError::Expired)
        );
        let mut wrong = fixture.trust;
        wrong.operations_key = [0x99; 32];
        assert_eq!(
            verify_and_encode(&fixture.bundle, &wrong, 1_900_000_000, &[1], 1),
            Err(AdmissionError::WrongOperationsKey)
        );
        let mut wrong = fixture.trust;
        wrong.cluster_id = [0x88; 32];
        assert_eq!(
            verify_and_encode(&fixture.bundle, &wrong, 1_900_000_000, &[1], 1),
            Err(AdmissionError::WrongCluster)
        );
    }

    #[test]
    fn follower_relay_uses_32_bit_framing_above_64_kib() {
        let fixture = fixture(operations::MAX_PROFILE_LEN, 2_000_000_000);
        assert!(fixture.bundle.len() > usize::from(u16::MAX));
        let request = roperations::Request {
            session: 3,
            request_id: 9,
            caller: b"node-a".to_vec(),
            bundle: fixture.bundle,
        };
        let body = roperations::encode_request(&request).unwrap();
        let mut frame = vec![roperations::TAG_REQUEST];
        frame.extend_from_slice(&body);
        assert_eq!(roperations::decode_request(&frame), Some(request));
    }
}
