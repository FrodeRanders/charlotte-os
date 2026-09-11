//! Catalog snapshot serialization and restore.
//!
//! Extracted from the catalog module. The codec walks the catalog's private
//! registries directly, so it stays a child module of `name_catalog`.

use super::*;

impl NameCatalog {
    pub(super) fn snapshot_bytes(&self) -> Vec<u8> {
        let entries = self.entries.lock();
        let releases = self.releases.lock();
        let deployments = self.deployments.lock();
        let deployment_replicas = self.deployment_replicas.lock();
        let operational_bindings = self.operational_bindings.lock();
        let shutdown_intents = self.shutdown_intents.lock();
        let ingress_policy = self.ingress_policy.lock();
        let mut size = 8 + 4; // magic + entry count
        for (name, entry) in entries.iter() {
            size += 4 + name.len() + 4 + entry.node.len() + 8 + 1 + 8;
        }
        // V7 appends signed deployment descriptors to the manifest records;
        // V8 binds active application registrations to deployment generations;
        // V9 persists atomically admitted signed release envelopes; V10 adds
        // compact encrypted-profile references and their replay fences; V11
        // adds the detached operational authorization for each binding; V12
        // appends node-targeted signed shutdown intents; V13 adds concrete
        // replica sets and per-node deployment readiness; V14 appends the
        // operator-signed cluster ingress policy.
        size += 4;
        for (artifact, entry) in deployments.iter() {
            size += 4
                + artifact.len()
                + 8
                + 8
                + 8
                + 32
                + 4
                + entry.descriptor.len()
                + 2
                + entry.replica_nodes.len() * 8;
        }
        size += 4;
        for (name, entry) in releases.iter() {
            size += 4 + name.len() + 8 + 8 + 32 + 4 + entry.envelope.len();
        }
        size += 4;
        for (name, entry) in operational_bindings.iter() {
            size += 179
                + charlotte_launch::operations_bundle::BINDING_SIGNATURE_LEN
                + name.len()
                + entry.release_name.len()
                + entry.target_artifact.len()
                + entry.object_key.len();
        }
        size += 4;
        for entry in shutdown_intents.values() {
            size += 8 + 8 + 4 + entry.envelope.len();
        }
        size += 4;
        for (name, replicas) in deployment_replicas.iter() {
            size += 4 + name.len() + 2;
            for entry in replicas.values() {
                size += 4 + entry.node.len() + 8 + 1 + 8;
            }
        }
        size += 1;
        if let Some(entry) = ingress_policy.as_ref() {
            size += 8 + 4 + entry.envelope.len();
        }
        size += 1 + 8 + 32;
        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&CATALOG_MAGIC_V14.to_le_bytes());
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (name, entry) in entries.iter() {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name);
            buf.extend_from_slice(&(entry.node.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.node);
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.push(u8::from(entry.active));
            buf.extend_from_slice(&entry.deployment_generation.to_le_bytes());
        }
        buf.extend_from_slice(&(deployments.len() as u32).to_le_bytes());
        for (artifact, entry) in deployments.iter() {
            buf.extend_from_slice(&(artifact.len() as u32).to_le_bytes());
            buf.extend_from_slice(artifact);
            buf.extend_from_slice(&entry.object_id.to_le_bytes());
            buf.extend_from_slice(&entry.node_key.to_le_bytes());
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.extend_from_slice(&entry.artifact_digest);
            buf.extend_from_slice(&(entry.descriptor.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.descriptor);
            buf.extend_from_slice(&(entry.replica_nodes.len() as u16).to_le_bytes());
            for node in &entry.replica_nodes {
                buf.extend_from_slice(&node.to_le_bytes());
            }
        }
        buf.extend_from_slice(&(releases.len() as u32).to_le_bytes());
        for (name, entry) in releases.iter() {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name);
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.extend_from_slice(&entry.operations_sequence.to_le_bytes());
            buf.extend_from_slice(&entry.operations_bundle_digest);
            buf.extend_from_slice(&(entry.envelope.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.envelope);
        }
        buf.extend_from_slice(&(operational_bindings.len() as u32).to_le_bytes());
        for (name, entry) in operational_bindings.iter() {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name);
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.push(u8::from(entry.active));
            buf.extend_from_slice(&(entry.release_name.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.release_name);
            buf.extend_from_slice(&entry.release_digest);
            buf.extend_from_slice(&entry.bundle_sequence.to_le_bytes());
            buf.extend_from_slice(&entry.bundle_digest);
            buf.extend_from_slice(&(entry.target_artifact.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.target_artifact);
            buf.extend_from_slice(&(entry.object_key.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.object_key);
            buf.extend_from_slice(&entry.envelope_digest);
            buf.extend_from_slice(&entry.profile_kind.to_le_bytes());
            buf.extend_from_slice(&entry.sequence.to_le_bytes());
            buf.extend_from_slice(&entry.expires_unix_seconds.to_le_bytes());
            buf.extend_from_slice(&entry.recipient_key_id);
            buf.extend_from_slice(&entry.signing_key_id);
            buf.extend_from_slice(&entry.authorization_signature);
        }
        buf.extend_from_slice(&(shutdown_intents.len() as u32).to_le_bytes());
        for (node_key, entry) in shutdown_intents.iter() {
            buf.extend_from_slice(&node_key.to_le_bytes());
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.extend_from_slice(&(entry.envelope.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.envelope);
        }
        buf.extend_from_slice(&(deployment_replicas.len() as u32).to_le_bytes());
        for (name, replicas) in deployment_replicas.iter() {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name);
            buf.extend_from_slice(&(replicas.len() as u16).to_le_bytes());
            for entry in replicas.values() {
                buf.extend_from_slice(&(entry.node.len() as u32).to_le_bytes());
                buf.extend_from_slice(&entry.node);
                buf.extend_from_slice(&entry.generation.to_le_bytes());
                buf.push(u8::from(entry.active));
                buf.extend_from_slice(&entry.deployment_generation.to_le_bytes());
            }
        }
        if let Some(entry) = ingress_policy.as_ref() {
            buf.push(1);
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.extend_from_slice(&(entry.envelope.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.envelope);
        } else {
            buf.push(0);
        }
        if let Some(key) = *self.cluster_key.lock() {
            buf.push(1);
            buf.extend_from_slice(&self.cluster_key_generation.lock().to_le_bytes());
            buf.extend_from_slice(&key);
        } else {
            buf.push(0);
            buf.extend_from_slice(&0u64.to_le_bytes());
            buf.extend_from_slice(&[0u8; 32]);
        }
        buf
    }

    pub(super) fn restore_bytes(&self, data: &[u8]) {
        if data.len() < 12 {
            return;
        }
        let magic = u64::from_le_bytes(data[0..8].try_into().ok().unwrap_or_default());
        if magic != CATALOG_MAGIC_V1
            && magic != CATALOG_MAGIC_V2
            && magic != CATALOG_MAGIC_V3
            && magic != CATALOG_MAGIC_V4
            && magic != CATALOG_MAGIC_V5
            && magic != CATALOG_MAGIC_V6
            && magic != CATALOG_MAGIC_V7
            && magic != CATALOG_MAGIC_V8
            && magic != CATALOG_MAGIC_V9
            && magic != CATALOG_MAGIC_V10
            && magic != CATALOG_MAGIC_V11
            && magic != CATALOG_MAGIC_V12
            && magic != CATALOG_MAGIC_V13
            && magic != CATALOG_MAGIC_V14
        {
            return;
        }
        let count = u32::from_le_bytes(data[8..12].try_into().ok().unwrap_or_default()) as usize;
        let mut pos = 12;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let Some((name, after_name)) = take_len_bytes(data, pos) else {
                return;
            };
            let Some((node, after_node)) = take_len_bytes(data, after_name) else {
                return;
            };
            let (generation, after_generation) = if magic != CATALOG_MAGIC_V1 {
                let Some(bytes) = data.get(after_node..after_node.saturating_add(8)) else {
                    return;
                };
                let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
                    return;
                };
                (u64::from_le_bytes(bytes), after_node + 8)
            } else {
                (1, after_node)
            };
            let (active, after_entry) = if magic == CATALOG_MAGIC_V3
                || magic == CATALOG_MAGIC_V4
                || magic == CATALOG_MAGIC_V5
                || magic == CATALOG_MAGIC_V6
                || magic == CATALOG_MAGIC_V7
                || magic == CATALOG_MAGIC_V8
                || magic == CATALOG_MAGIC_V9
                || magic == CATALOG_MAGIC_V10
                || magic == CATALOG_MAGIC_V11
                || magic == CATALOG_MAGIC_V12
                || magic == CATALOG_MAGIC_V13
                || magic == CATALOG_MAGIC_V14
            {
                let Some(active) = data.get(after_generation) else {
                    return;
                };
                (*active != 0, after_generation + 1)
            } else {
                (true, after_generation)
            };
            let (deployment_generation, after_entry) = if magic == CATALOG_MAGIC_V8
                || magic == CATALOG_MAGIC_V9
                || magic == CATALOG_MAGIC_V10
                || magic == CATALOG_MAGIC_V11
                || magic == CATALOG_MAGIC_V12
                || magic == CATALOG_MAGIC_V13
                || magic == CATALOG_MAGIC_V14
            {
                let Some((generation, after_generation)) = read_u64(data, after_entry) else {
                    return;
                };
                (generation, after_generation)
            } else {
                (0, after_entry)
            };
            entries.insert(
                name.to_vec(),
                CatalogEntry {
                    node: node.to_vec(),
                    generation,
                    active,
                    deployment_generation,
                },
            );
            pos = after_entry;
        }
        *self.entries.lock() = entries;

        let mut deployments = BTreeMap::new();
        if magic == CATALOG_MAGIC_V4
            || magic == CATALOG_MAGIC_V5
            || magic == CATALOG_MAGIC_V6
            || magic == CATALOG_MAGIC_V7
            || magic == CATALOG_MAGIC_V8
            || magic == CATALOG_MAGIC_V9
            || magic == CATALOG_MAGIC_V10
            || magic == CATALOG_MAGIC_V11
            || magic == CATALOG_MAGIC_V12
            || magic == CATALOG_MAGIC_V13
            || magic == CATALOG_MAGIC_V14
        {
            let Some(bytes) = data.get(pos..pos.saturating_add(4)) else {
                return;
            };
            let Ok(bytes) = <[u8; 4]>::try_from(bytes) else {
                return;
            };
            let deploy_count = u32::from_le_bytes(bytes) as usize;
            pos += 4;
            for _ in 0..deploy_count {
                let Some((artifact, after_artifact)) = take_len_bytes(data, pos) else {
                    return;
                };
                let Some((object_id, after_object)) = read_u64(data, after_artifact) else {
                    return;
                };
                let Some((node_key, after_node)) = read_u64(data, after_object) else {
                    return;
                };
                let Some((generation, after_generation)) = read_u64(data, after_node) else {
                    return;
                };
                // V4 carried a placeholder MAC after the generation; V5 did
                // not. V6 pins the artifact's complete SHA-256 identity.
                let (artifact_digest, after_entry) = if magic == CATALOG_MAGIC_V4 {
                    match read_u64(data, after_generation) {
                        Some((_, after_mac)) => ([0; 32], after_mac),
                        None => return,
                    }
                } else if magic == CATALOG_MAGIC_V6
                    || magic == CATALOG_MAGIC_V7
                    || magic == CATALOG_MAGIC_V8
                    || magic == CATALOG_MAGIC_V9
                    || magic == CATALOG_MAGIC_V10
                    || magic == CATALOG_MAGIC_V11
                    || magic == CATALOG_MAGIC_V12
                    || magic == CATALOG_MAGIC_V13
                    || magic == CATALOG_MAGIC_V14
                {
                    let Some(digest) =
                        data.get(after_generation..after_generation.saturating_add(32))
                    else {
                        return;
                    };
                    let Ok(digest) = <[u8; 32]>::try_from(digest) else {
                        return;
                    };
                    (digest, after_generation + 32)
                } else {
                    ([0; 32], after_generation)
                };
                let (descriptor, after_entry) = if magic == CATALOG_MAGIC_V7
                    || magic == CATALOG_MAGIC_V8
                    || magic == CATALOG_MAGIC_V9
                    || magic == CATALOG_MAGIC_V10
                    || magic == CATALOG_MAGIC_V11
                    || magic == CATALOG_MAGIC_V12
                    || magic == CATALOG_MAGIC_V13
                    || magic == CATALOG_MAGIC_V14
                {
                    let Some((descriptor, after_descriptor)) = take_len_bytes(data, after_entry)
                    else {
                        return;
                    };
                    if descriptor.len() > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN {
                        return;
                    }
                    (descriptor.to_vec(), after_descriptor)
                } else {
                    (Vec::new(), after_entry)
                };
                let (replica_nodes, after_entry) = if magic == CATALOG_MAGIC_V13
                    || magic == CATALOG_MAGIC_V14
                {
                    let Some((replica_count, mut position)) = read_u16(data, after_entry) else {
                        return;
                    };
                    let mut nodes = Vec::with_capacity(usize::from(replica_count));
                    let mut seen = BTreeSet::new();
                    for _ in 0..replica_count {
                        let Some((node, next)) = read_u64(data, position) else {
                            return;
                        };
                        if node == 0 || !seen.insert(node) {
                            return;
                        }
                        nodes.push(node);
                        position = next;
                    }
                    if nodes.first().copied().unwrap_or(0) != node_key {
                        return;
                    }
                    (nodes, position)
                } else {
                    ((node_key != 0).then_some(node_key).into_iter().collect(), after_entry)
                };
                deployments.insert(
                    artifact.to_vec(),
                    DeploymentEntry {
                        object_id,
                        node_key,
                        replica_nodes,
                        generation,
                        artifact_digest,
                        descriptor,
                    },
                );
                pos = after_entry;
            }
        }
        *self.deployments.lock() = deployments;

        let mut releases = BTreeMap::new();
        if magic == CATALOG_MAGIC_V9
            || magic == CATALOG_MAGIC_V10
            || magic == CATALOG_MAGIC_V11
            || magic == CATALOG_MAGIC_V12
            || magic == CATALOG_MAGIC_V13
            || magic == CATALOG_MAGIC_V14
        {
            let Some(bytes) = data.get(pos..pos.saturating_add(4)) else {
                return;
            };
            let Ok(bytes) = <[u8; 4]>::try_from(bytes) else {
                return;
            };
            let release_count = u32::from_le_bytes(bytes) as usize;
            pos += 4;
            for _ in 0..release_count {
                let Some((name, after_name)) = take_len_bytes(data, pos) else {
                    return;
                };
                let Some((generation, after_generation)) = read_u64(data, after_name) else {
                    return;
                };
                let (operations_sequence, operations_bundle_digest, after_operations) = if magic
                    == CATALOG_MAGIC_V10
                    || magic == CATALOG_MAGIC_V11
                    || magic == CATALOG_MAGIC_V12
                    || magic == CATALOG_MAGIC_V13
                    || magic == CATALOG_MAGIC_V14
                {
                    let Some((operations_sequence, after_operations_sequence)) =
                        read_u64(data, after_generation)
                    else {
                        return;
                    };
                    let Some(operations_bundle_digest) = data
                        .get(after_operations_sequence..after_operations_sequence + 32)
                        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                    else {
                        return;
                    };
                    (operations_sequence, operations_bundle_digest, after_operations_sequence + 32)
                } else {
                    (0, [0; 32], after_generation)
                };
                let Some((envelope, after_envelope)) = take_len_bytes(data, after_operations)
                else {
                    return;
                };
                let Some(decoded) = charlotte_launch::release::decode(envelope) else {
                    return;
                };
                if decoded.release_name != name || generation == 0 {
                    return;
                }
                releases.insert(
                    name.to_vec(),
                    ReleaseEntry {
                        generation,
                        envelope: envelope.to_vec(),
                        operations_sequence,
                        operations_bundle_digest,
                    },
                );
                pos = after_envelope;
            }
        }
        let mut operational_bindings = BTreeMap::new();
        if magic == CATALOG_MAGIC_V10
            || magic == CATALOG_MAGIC_V11
            || magic == CATALOG_MAGIC_V12
            || magic == CATALOG_MAGIC_V13
            || magic == CATALOG_MAGIC_V14
        {
            let Some(binding_count) = data
                .get(pos..pos.saturating_add(4))
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
            else {
                return;
            };
            pos += 4;
            for _ in 0..binding_count {
                let Some((profile_name, after_profile)) = take_len_bytes(data, pos) else {
                    return;
                };
                let Some((generation, after_generation)) = read_u64(data, after_profile) else {
                    return;
                };
                let Some(active) = data.get(after_generation) else {
                    return;
                };
                let Some((release_name, after_release_name)) =
                    take_len_bytes(data, after_generation + 1)
                else {
                    return;
                };
                let Some(release_digest) = data
                    .get(after_release_name..after_release_name + 32)
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                else {
                    return;
                };
                let Some((bundle_sequence, after_bundle_sequence)) =
                    read_u64(data, after_release_name + 32)
                else {
                    return;
                };
                let Some(bundle_digest) = data
                    .get(after_bundle_sequence..after_bundle_sequence + 32)
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                else {
                    return;
                };
                let Some((target_artifact, after_target)) =
                    take_len_bytes(data, after_bundle_sequence + 32)
                else {
                    return;
                };
                let Some((object_key, after_object)) = take_len_bytes(data, after_target) else {
                    return;
                };
                let Some(envelope_digest) = data
                    .get(after_object..after_object + 32)
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                else {
                    return;
                };
                let Some((profile_kind, after_kind)) = read_u16(data, after_object + 32) else {
                    return;
                };
                let Some((sequence, after_sequence)) = read_u64(data, after_kind) else {
                    return;
                };
                let Some((expires_unix_seconds, after_expiry)) = read_u64(data, after_sequence)
                else {
                    return;
                };
                let Some(recipient_key_id) = data
                    .get(after_expiry..after_expiry + 16)
                    .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
                else {
                    return;
                };
                let Some(signing_key_id) = data
                    .get(after_expiry + 16..after_expiry + 32)
                    .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
                else {
                    return;
                };
                let (authorization_signature, after_binding) = if magic == CATALOG_MAGIC_V11
                    || magic == CATALOG_MAGIC_V12
                    || magic == CATALOG_MAGIC_V13
                    || magic == CATALOG_MAGIC_V14
                {
                    let Some(signature) = data
                        .get(after_expiry + 32..after_expiry + 96)
                        .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
                    else {
                        return;
                    };
                    (signature, after_expiry + 96)
                } else {
                    ([0; 64], after_expiry + 32)
                };
                if generation == 0
                    || bundle_sequence == 0
                    || sequence == 0
                    || expires_unix_seconds == 0
                    || !charlotte_launch::operations::valid_profile_name(profile_name)
                    || !charlotte_launch::deployment::valid_artifact_name(target_artifact)
                    || !charlotte_launch::operations_bundle::valid_object_key(object_key)
                    || !charlotte_launch::operations::valid_profile_kind(profile_kind)
                {
                    return;
                }
                let Some(release_entry) = releases.get(release_name) else {
                    return;
                };
                let Some(release_envelope) =
                    charlotte_launch::release::decode(&release_entry.envelope)
                else {
                    return;
                };
                if release_digest != charlotte_launch::sha256::digest(&release_entry.envelope)
                    || !release_contains_artifact(&release_envelope, target_artifact)
                    || (*active != 0
                        && (bundle_sequence != release_entry.operations_sequence
                            || bundle_digest != release_entry.operations_bundle_digest))
                {
                    return;
                }
                if operational_bindings
                    .insert(
                        profile_name.to_vec(),
                        OperationalBindingEntry {
                            generation,
                            active: *active != 0,
                            release_name: release_name.to_vec(),
                            release_digest,
                            bundle_sequence,
                            bundle_digest,
                            target_artifact: target_artifact.to_vec(),
                            object_key: object_key.to_vec(),
                            envelope_digest,
                            profile_kind,
                            sequence,
                            expires_unix_seconds,
                            recipient_key_id,
                            signing_key_id,
                            authorization_signature,
                        },
                    )
                    .is_some()
                {
                    return;
                }
                pos = after_binding;
            }
        }
        *self.releases.lock() = releases;
        *self.operational_bindings.lock() = operational_bindings;

        let mut shutdown_intents = BTreeMap::new();
        if magic == CATALOG_MAGIC_V12 || magic == CATALOG_MAGIC_V13 || magic == CATALOG_MAGIC_V14 {
            let Some(intent_count) = data
                .get(pos..pos.saturating_add(4))
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
            else {
                return;
            };
            pos += 4;
            for _ in 0..intent_count {
                let Some((node_key, after_node)) = read_u64(data, pos) else {
                    return;
                };
                let Some((generation, after_generation)) = read_u64(data, after_node) else {
                    return;
                };
                let Some((envelope, after_envelope)) = take_len_bytes(data, after_generation)
                else {
                    return;
                };
                let Some(fields) = charlotte_launch::shutdown::decode(envelope) else {
                    return;
                };
                if node_key == 0
                    || generation == 0
                    || fields.target_node != node_key
                    || shutdown_intents
                        .insert(
                            node_key,
                            ShutdownIntentEntry {
                                generation,
                                envelope: envelope.to_vec(),
                            },
                        )
                        .is_some()
                {
                    return;
                }
                pos = after_envelope;
            }
        }
        *self.shutdown_intents.lock() = shutdown_intents;

        let mut deployment_replicas = BTreeMap::new();
        if magic == CATALOG_MAGIC_V13 || magic == CATALOG_MAGIC_V14 {
            let Some(replica_name_count) = data
                .get(pos..pos.saturating_add(4))
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
            else {
                return;
            };
            pos += 4;
            for _ in 0..replica_name_count {
                let Some((name, after_name)) = take_len_bytes(data, pos) else {
                    return;
                };
                let Some((replica_count, mut position)) = read_u16(data, after_name) else {
                    return;
                };
                let mut replicas = BTreeMap::new();
                for _ in 0..replica_count {
                    let Some((node, after_node)) = take_len_bytes(data, position) else {
                        return;
                    };
                    let Some((generation, after_generation)) = read_u64(data, after_node) else {
                        return;
                    };
                    let Some(active) = data.get(after_generation) else {
                        return;
                    };
                    let Some((deployment_generation, after_entry)) =
                        read_u64(data, after_generation + 1)
                    else {
                        return;
                    };
                    if generation == 0
                        || node.is_empty()
                        || replicas
                            .insert(
                                node.to_vec(),
                                CatalogEntry {
                                    node: node.to_vec(),
                                    generation,
                                    active: *active != 0,
                                    deployment_generation,
                                },
                            )
                            .is_some()
                    {
                        return;
                    }
                    position = after_entry;
                }
                if deployment_replicas.insert(name.to_vec(), replicas).is_some() {
                    return;
                }
                pos = position;
            }
        }
        *self.deployment_replicas.lock() = deployment_replicas;

        let ingress_policy = if magic == CATALOG_MAGIC_V14 {
            let Some(present) = data.get(pos) else {
                return;
            };
            pos += 1;
            if *present == 0 {
                None
            } else {
                let Some((generation, after_generation)) = read_u64(data, pos) else {
                    return;
                };
                let Some((envelope, after_envelope)) = take_len_bytes(data, after_generation)
                else {
                    return;
                };
                if generation == 0
                    || charlotte_launch::ingress_policy::verify(
                        envelope,
                        &self.cluster_id,
                        &self.bootstrap_operations_key,
                    ) != charlotte_launch::ingress_policy::VerifyOutcome::Valid
                {
                    return;
                }
                pos = after_envelope;
                Some(IngressPolicyEntry {
                    generation,
                    envelope: envelope.to_vec(),
                })
            }
        } else {
            None
        };
        *self.ingress_policy.lock() = ingress_policy;

        *self.cluster_key.lock() = None;
        *self.cluster_key_generation.lock() = 0;

        if magic == CATALOG_MAGIC_V5
            || magic == CATALOG_MAGIC_V6
            || magic == CATALOG_MAGIC_V7
            || magic == CATALOG_MAGIC_V8
            || magic == CATALOG_MAGIC_V9
            || magic == CATALOG_MAGIC_V10
            || magic == CATALOG_MAGIC_V11
            || magic == CATALOG_MAGIC_V12
            || magic == CATALOG_MAGIC_V13
            || magic == CATALOG_MAGIC_V14
        {
            let Some(present) = data.get(pos) else {
                return;
            };
            let (generation, key_start) = if magic == CATALOG_MAGIC_V6
                || magic == CATALOG_MAGIC_V7
                || magic == CATALOG_MAGIC_V8
                || magic == CATALOG_MAGIC_V9
                || magic == CATALOG_MAGIC_V10
                || magic == CATALOG_MAGIC_V11
                || magic == CATALOG_MAGIC_V12
                || magic == CATALOG_MAGIC_V13
                || magic == CATALOG_MAGIC_V14
            {
                let Some((generation, after_generation)) = read_u64(data, pos + 1) else {
                    return;
                };
                (generation, after_generation)
            } else {
                (u64::from(*present != 0), pos + 1)
            };
            let Some(key) = data.get(key_start..key_start + 32) else {
                return;
            };
            if *present != 0 {
                let Ok(key) = <[u8; 32]>::try_from(key) else {
                    return;
                };
                *self.cluster_key.lock() = Some(key);
                *self.cluster_key_generation.lock() = generation.max(1);
            }
        }
    }
}
