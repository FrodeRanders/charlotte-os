//! Catalog snapshot serialization and restore.
//!
//! Extracted from the catalog module. The codec walks the catalog's private
//! registries directly, so it stays a child module of `name_catalog`.

use catten_graft::state_machine::SnapshotError;

use super::*;

impl NameCatalog {
    pub(super) fn snapshot_bytes(&self) -> Vec<u8> {
        let entries = self.entries.lock();
        let releases = self.releases.lock();
        let deployments = self.deployments.lock();
        let deployment_replicas = self.deployment_replicas.lock();
        let operational_bindings = self.operational_bindings.lock();
        let shutdown_intents = self.shutdown_intents.lock();
        let node_capacity = self.node_capacity.lock();
        let ingress_policy = self.ingress_policy.lock();
        let mut size = 8 + 4; // magic + entry count
        for (name, entry) in entries.iter() {
            size += 4 + name.len() + 4 + entry.node.len() + 8 + 1 + 8;
        }
        // The sole snapshot grammar includes every current catalog section.
        size += 4;
        for (artifact, entry) in deployments.iter() {
            size += 4
                + artifact.len()
                + 8
                + 4
                + entry.descriptor.len()
                + 2
                + entry.replica_nodes.len() * 10;
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
        size += 4;
        for _ in node_capacity.values() {
            size += 8 + 8 + 8 + 8 + 8 + 2 + 16;
        }
        let mut buf = Vec::with_capacity(size);
        buf.extend_from_slice(&CATALOG_MAGIC_V18.to_le_bytes());
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
            buf.extend_from_slice(&entry.generation.to_le_bytes());
            buf.extend_from_slice(&(entry.descriptor.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.descriptor);
            buf.extend_from_slice(&(entry.replica_nodes.len() as u16).to_le_bytes());
            for (node, variant) in entry.replica_nodes.iter().zip(&entry.replica_variants) {
                buf.extend_from_slice(&node.to_le_bytes());
                buf.extend_from_slice(&variant.to_le_bytes());
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
        buf.extend_from_slice(&(node_capacity.len() as u32).to_le_bytes());
        for entry in node_capacity.values() {
            buf.extend_from_slice(&entry.node_key.to_le_bytes());
            buf.extend_from_slice(&entry.boot_nonce.to_le_bytes());
            buf.extend_from_slice(&entry.epoch.to_le_bytes());
            buf.extend_from_slice(&entry.free_frames.to_le_bytes());
            buf.extend_from_slice(&entry.usable_frames.to_le_bytes());
            buf.extend_from_slice(&entry.cpu_load_permille.to_le_bytes());
            buf.extend_from_slice(&entry.target.encode().unwrap_or([0; 16]));
        }
        buf
    }

    pub(super) fn restore_bytes(&self, data: &[u8]) -> Result<(), SnapshotError> {
        if data.len() < 12 {
            return Err(SnapshotError::Malformed);
        }
        let magic = u64::from_le_bytes(data[0..8].try_into().ok().unwrap_or_default());
        if magic != CATALOG_MAGIC_V18 {
            return Err(SnapshotError::Unsupported);
        }
        let count = u32::from_le_bytes(data[8..12].try_into().ok().unwrap_or_default()) as usize;
        if count > MAX_ENTRIES {
            return Err(SnapshotError::Malformed);
        }
        let mut pos = 12;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let Some((name, after_name)) = take_len_bytes(data, pos) else {
                return Err(SnapshotError::Malformed);
            };
            let Some((node, after_node)) = take_len_bytes(data, after_name) else {
                return Err(SnapshotError::Malformed);
            };
            let (generation, after_generation) = {
                let Some(bytes) = data.get(after_node..after_node.saturating_add(8)) else {
                    return Err(SnapshotError::Malformed);
                };
                let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
                    return Err(SnapshotError::Malformed);
                };
                (u64::from_le_bytes(bytes), after_node + 8)
            };
            let (active, after_entry) = {
                let Some(active) = data.get(after_generation) else {
                    return Err(SnapshotError::Malformed);
                };
                if *active > 1 {
                    return Err(SnapshotError::Malformed);
                }
                (*active != 0, after_generation + 1)
            };
            let (deployment_generation, after_entry) = {
                let Some((generation, after_generation)) = read_u64(data, after_entry) else {
                    return Err(SnapshotError::Malformed);
                };
                (generation, after_generation)
            };
            if generation == 0 || name.is_empty() || entries.contains_key(name) {
                return Err(SnapshotError::Malformed);
            }
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

        let mut deployments = BTreeMap::new();
        {
            let Some(bytes) = data.get(pos..pos.saturating_add(4)) else {
                return Err(SnapshotError::Malformed);
            };
            let Ok(bytes) = <[u8; 4]>::try_from(bytes) else {
                return Err(SnapshotError::Malformed);
            };
            let deploy_count = u32::from_le_bytes(bytes) as usize;
            pos += 4;
            if deploy_count > MAX_DEPLOYMENTS {
                return Err(SnapshotError::Malformed);
            }
            for _ in 0..deploy_count {
                let Some((artifact, after_artifact)) = take_len_bytes(data, pos) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((generation, after_entry)) = read_u64(data, after_artifact) else {
                    return Err(SnapshotError::Malformed);
                };
                let (descriptor, after_entry) = {
                    let Some((descriptor, after_descriptor)) = take_len_bytes(data, after_entry)
                    else {
                        return Err(SnapshotError::Malformed);
                    };
                    if descriptor.len() > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN {
                        return Err(SnapshotError::Malformed);
                    }
                    (descriptor.to_vec(), after_descriptor)
                };
                let (replica_nodes, replica_variants, after_entry) = {
                    let signed = charlotte_launch::deployment::decode(&descriptor)
                        .ok_or(SnapshotError::Malformed)?;
                    let Some((replica_count, mut position)) = read_u16(data, after_entry) else {
                        return Err(SnapshotError::Malformed);
                    };
                    let mut nodes = Vec::with_capacity(
                        usize::from(replica_count).min(data.len().saturating_sub(position)),
                    );
                    let mut seen = BTreeSet::new();
                    let mut variants = Vec::new();
                    for _ in 0..replica_count {
                        let Some((node, next)) = read_u64(data, position) else {
                            return Err(SnapshotError::Malformed);
                        };
                        if node == 0
                            || nodes.last().is_some_and(|previous| *previous >= node)
                            || !seen.insert(node)
                        {
                            return Err(SnapshotError::Malformed);
                        }
                        let (variant, next) =
                            read_u16(data, next).ok_or(SnapshotError::Malformed)?;
                        signed.variant(usize::from(variant)).ok_or(SnapshotError::Malformed)?;
                        nodes.push(node);
                        variants.push(variant);
                        position = next;
                    }
                    if signed.artifact_name != artifact {
                        return Err(SnapshotError::Malformed);
                    }
                    (nodes, variants, position)
                };
                if generation == 0 || artifact.is_empty() || deployments.contains_key(artifact) {
                    return Err(SnapshotError::Malformed);
                }
                deployments.insert(
                    artifact.to_vec(),
                    DeploymentEntry {
                        replica_nodes,
                        replica_variants,
                        generation,
                        descriptor,
                    },
                );
                pos = after_entry;
            }
        }

        let mut releases = BTreeMap::new();
        {
            let Some(bytes) = data.get(pos..pos.saturating_add(4)) else {
                return Err(SnapshotError::Malformed);
            };
            let Ok(bytes) = <[u8; 4]>::try_from(bytes) else {
                return Err(SnapshotError::Malformed);
            };
            let release_count = u32::from_le_bytes(bytes) as usize;
            pos += 4;
            if release_count > MAX_RELEASES {
                return Err(SnapshotError::Malformed);
            }
            for _ in 0..release_count {
                let Some((name, after_name)) = take_len_bytes(data, pos) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((generation, after_generation)) = read_u64(data, after_name) else {
                    return Err(SnapshotError::Malformed);
                };
                let (operations_sequence, operations_bundle_digest, after_operations) = {
                    let Some((operations_sequence, after_operations_sequence)) =
                        read_u64(data, after_generation)
                    else {
                        return Err(SnapshotError::Malformed);
                    };
                    let Some(operations_bundle_digest) = data
                        .get(after_operations_sequence..after_operations_sequence + 32)
                        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                    else {
                        return Err(SnapshotError::Malformed);
                    };
                    (operations_sequence, operations_bundle_digest, after_operations_sequence + 32)
                };
                let Some((envelope, after_envelope)) = take_len_bytes(data, after_operations)
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(decoded) = charlotte_launch::release::decode(envelope) else {
                    return Err(SnapshotError::Malformed);
                };
                if decoded.release_name != name || generation == 0 || releases.contains_key(name) {
                    return Err(SnapshotError::Malformed);
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
        {
            let Some(binding_count) = data
                .get(pos..pos.saturating_add(4))
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
            else {
                return Err(SnapshotError::Malformed);
            };
            pos += 4;
            if binding_count as usize > MAX_OPERATIONAL_BINDINGS {
                return Err(SnapshotError::Malformed);
            }
            for _ in 0..binding_count {
                let Some((profile_name, after_profile)) = take_len_bytes(data, pos) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((generation, after_generation)) = read_u64(data, after_profile) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(active) = data.get(after_generation) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((release_name, after_release_name)) =
                    take_len_bytes(data, after_generation + 1)
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(release_digest) = data
                    .get(after_release_name..after_release_name + 32)
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((bundle_sequence, after_bundle_sequence)) =
                    read_u64(data, after_release_name + 32)
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(bundle_digest) = data
                    .get(after_bundle_sequence..after_bundle_sequence + 32)
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((target_artifact, after_target)) =
                    take_len_bytes(data, after_bundle_sequence + 32)
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((object_key, after_object)) = take_len_bytes(data, after_target) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(envelope_digest) = data
                    .get(after_object..after_object + 32)
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((profile_kind, after_kind)) = read_u16(data, after_object + 32) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((sequence, after_sequence)) = read_u64(data, after_kind) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((expires_unix_seconds, after_expiry)) = read_u64(data, after_sequence)
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(recipient_key_id) = data
                    .get(after_expiry..after_expiry + 16)
                    .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(signing_key_id) = data
                    .get(after_expiry + 16..after_expiry + 32)
                    .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
                else {
                    return Err(SnapshotError::Malformed);
                };
                let (authorization_signature, after_binding) = {
                    let Some(signature) = data
                        .get(after_expiry + 32..after_expiry + 96)
                        .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
                    else {
                        return Err(SnapshotError::Malformed);
                    };
                    (signature, after_expiry + 96)
                };
                if *active > 1
                    || generation == 0
                    || bundle_sequence == 0
                    || sequence == 0
                    || expires_unix_seconds == 0
                    || !charlotte_launch::operations::valid_profile_name(profile_name)
                    || !charlotte_launch::deployment::valid_artifact_name(target_artifact)
                    || !charlotte_launch::operations_bundle::valid_object_key(object_key)
                    || !charlotte_launch::operations::valid_profile_kind(profile_kind)
                {
                    return Err(SnapshotError::Malformed);
                }
                let Some(release_entry) = releases.get(release_name) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(release_envelope) =
                    charlotte_launch::release::decode(&release_entry.envelope)
                else {
                    return Err(SnapshotError::Malformed);
                };
                if release_digest != charlotte_launch::sha256::digest(&release_entry.envelope)
                    || !release_contains_artifact(&release_envelope, target_artifact)
                    || (*active != 0
                        && (bundle_sequence != release_entry.operations_sequence
                            || bundle_digest != release_entry.operations_bundle_digest))
                {
                    return Err(SnapshotError::Malformed);
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
                    return Err(SnapshotError::Malformed);
                }
                pos = after_binding;
            }
        }

        let mut shutdown_intents = BTreeMap::new();
        {
            let Some(intent_count) = data
                .get(pos..pos.saturating_add(4))
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
            else {
                return Err(SnapshotError::Malformed);
            };
            pos += 4;
            if intent_count as usize > MAX_SHUTDOWN_INTENTS {
                return Err(SnapshotError::Malformed);
            }
            for _ in 0..intent_count {
                let Some((node_key, after_node)) = read_u64(data, pos) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((generation, after_generation)) = read_u64(data, after_node) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((envelope, after_envelope)) = take_len_bytes(data, after_generation)
                else {
                    return Err(SnapshotError::Malformed);
                };
                let Some(fields) = charlotte_launch::shutdown::decode(envelope) else {
                    return Err(SnapshotError::Malformed);
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
                    return Err(SnapshotError::Malformed);
                }
                pos = after_envelope;
            }
        }

        let mut deployment_replicas = BTreeMap::new();
        {
            let Some(replica_name_count) = data
                .get(pos..pos.saturating_add(4))
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
            else {
                return Err(SnapshotError::Malformed);
            };
            pos += 4;
            if replica_name_count as usize > MAX_DEPLOYMENTS {
                return Err(SnapshotError::Malformed);
            }
            for _ in 0..replica_name_count {
                let Some((name, after_name)) = take_len_bytes(data, pos) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((replica_count, mut position)) = read_u16(data, after_name) else {
                    return Err(SnapshotError::Malformed);
                };
                let mut replicas = BTreeMap::new();
                for _ in 0..replica_count {
                    let Some((node, after_node)) = take_len_bytes(data, position) else {
                        return Err(SnapshotError::Malformed);
                    };
                    let Some((generation, after_generation)) = read_u64(data, after_node) else {
                        return Err(SnapshotError::Malformed);
                    };
                    let Some(active) = data.get(after_generation) else {
                        return Err(SnapshotError::Malformed);
                    };
                    let Some((deployment_generation, after_entry)) =
                        read_u64(data, after_generation + 1)
                    else {
                        return Err(SnapshotError::Malformed);
                    };
                    if *active > 1
                        || generation == 0
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
                        return Err(SnapshotError::Malformed);
                    }
                    position = after_entry;
                }
                if deployment_replicas.insert(name.to_vec(), replicas).is_some() {
                    return Err(SnapshotError::Malformed);
                }
                pos = position;
            }
        }

        let ingress_policy = {
            let Some(present) = data.get(pos) else {
                return Err(SnapshotError::Malformed);
            };
            if *present > 1 {
                return Err(SnapshotError::Malformed);
            }
            pos += 1;
            if *present == 0 {
                None
            } else {
                let Some((generation, after_generation)) = read_u64(data, pos) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((envelope, after_envelope)) = take_len_bytes(data, after_generation)
                else {
                    return Err(SnapshotError::Malformed);
                };
                if generation == 0
                    || charlotte_launch::ingress_policy::verify(
                        envelope,
                        &self.cluster_id,
                        &self.bootstrap_operations_key,
                    ) != charlotte_launch::ingress_policy::VerifyOutcome::Valid
                {
                    return Err(SnapshotError::Malformed);
                }
                pos = after_envelope;
                Some(IngressPolicyEntry {
                    generation,
                    envelope: envelope.to_vec(),
                })
            }
        };

        let mut cluster_key = None;
        let mut cluster_key_generation = 0u64;

        {
            let Some(present) = data.get(pos) else {
                return Err(SnapshotError::Malformed);
            };
            let (generation, key_start) = {
                let Some((generation, after_generation)) = read_u64(data, pos + 1) else {
                    return Err(SnapshotError::Malformed);
                };
                (generation, after_generation)
            };
            let Some(key) = data.get(key_start..key_start + 32) else {
                return Err(SnapshotError::Malformed);
            };
            if *present > 1 || (*present == 0 && (generation != 0 || key != [0; 32])) {
                return Err(SnapshotError::Malformed);
            }
            if *present != 0 {
                let Ok(key) = <[u8; 32]>::try_from(key) else {
                    return Err(SnapshotError::Malformed);
                };
                cluster_key = Some(key);
                if generation == 0 {
                    return Err(SnapshotError::Malformed);
                }
                cluster_key_generation = generation;
            }
            pos = key_start + 32;
        }

        let mut node_capacity = BTreeMap::new();
        {
            let Some(sample_count) = data
                .get(pos..pos.saturating_add(4))
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
            else {
                return Err(SnapshotError::Malformed);
            };
            pos += 4;
            if sample_count as usize > MAX_NODE_CAPACITY_ENTRIES {
                return Err(SnapshotError::Malformed);
            }
            for _ in 0..sample_count {
                let Some((node_key, after_node)) = read_u64(data, pos) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((boot_nonce, after_nonce)) = read_u64(data, after_node) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((epoch, after_epoch)) = read_u64(data, after_nonce) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((free_frames, after_free)) = read_u64(data, after_epoch) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((usable_frames, after_usable)) = read_u64(data, after_free) else {
                    return Err(SnapshotError::Malformed);
                };
                let Some((cpu_load_permille, after_entry)) = read_u16(data, after_usable) else {
                    return Err(SnapshotError::Malformed);
                };
                let target = charlotte_launch::execution_target::ExecutionTarget::decode(
                    data.get(after_entry..after_entry + 16).ok_or(SnapshotError::Malformed)?,
                )
                .ok_or(SnapshotError::Malformed)?;
                let after_entry = after_entry + 16;
                if node_key == 0
                    || boot_nonce == 0
                    || cpu_load_permille > 1000
                    || epoch == 0
                    || usable_frames == 0
                    || free_frames > usable_frames
                    || node_capacity
                        .insert(
                            node_key,
                            NodeCapacityEntry {
                                target,
                                node_key,
                                boot_nonce,
                                epoch,
                                free_frames,
                                usable_frames,
                                cpu_load_permille,
                            },
                        )
                        .is_some()
                {
                    return Err(SnapshotError::Malformed);
                }
                pos = after_entry;
            }
        }

        if pos != data.len() {
            return Err(SnapshotError::Malformed);
        }

        // Publish the whole snapshot only after every section parsed; a
        // truncated or corrupt snapshot leaves the previous state intact
        // rather than a mix of old and new collections.
        *self.entries.lock() = entries;
        *self.deployments.lock() = deployments;
        *self.releases.lock() = releases;
        *self.operational_bindings.lock() = operational_bindings;
        *self.shutdown_intents.lock() = shutdown_intents;
        *self.deployment_replicas.lock() = deployment_replicas;
        *self.node_capacity.lock() = node_capacity;
        *self.ingress_policy.lock() = ingress_policy;
        *self.cluster_key.lock() = cluster_key;
        *self.cluster_key_generation.lock() = cluster_key_generation;
        Ok(())
    }
}
