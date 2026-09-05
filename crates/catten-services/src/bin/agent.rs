//! Node-side reconciler for cluster deployments.
//!
//! Desired deployments live in the replicated name catalog. Each agent
//! enumerates them, launches every assignment for its node, publishes ready
//! application endpoints to the distributed catalog, and retires only the
//! exact artifact generation that moved away. Executable bytes come from the
//! node's separately provisioned S3 connector; no object-store secret enters
//! an application descriptor.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    vec,
    vec::Vec,
};

use catten_rt::{
    Context,
    ManifestValue,
    config,
    manifest_key,
    owned::{
        Connection,
        ConnectionRef,
        DeployedArtifact,
        OwnedMemory,
        launch_artifact,
        launch_operational_connector,
        launch_scoped_artifact_named,
    },
};
use catten_services::{
    dns,
    net,
    node_identity,
    objstore,
    s3_client::Client as S3Client,
    sleep_ms,
    sleep_ms_or_shutdown,
    time,
    try_registered_name_bytes_owned,
    wait_for_local_ready_or_shutdown,
    wait_for_registered_name_or_shutdown_owned,
    wait_for_registered_name_owned,
};

catten_rt::entry!(main);

const STAGE_IDENTITY: u32 = 2;
const STAGE_SERVING: u32 = 6;
const STAGE_RETIRED: u32 = 7;
const STAGE_DRAINING_APPLICATIONS: u32 = 8;
const STAGE_DRAINING_CONNECTORS: u32 = 9;
const STAGE_SHUTDOWN_READY: u32 = 10;
const STAGE_FAIL: u32 = 0xdead;
const RETIREMENT_POLL_MS: u64 = 10;

struct DeploymentInfo {
    generation: u64,
    object_id: u64,
    node_key: u64,
    artifact_digest: [u8; 32],
    descriptor: Vec<u8>,
}

struct ActiveDeployment {
    name: Vec<u8>,
    generation: u64,
    domain: DeployedArtifact,
    published: bool,
    retiring: bool,
}

struct OperationalBinding {
    generation: u64,
    bundle_sequence: u64,
    sequence: u64,
    expires_unix_seconds: u64,
    profile_kind: u16,
    release_name: Vec<u8>,
    profile_name: Vec<u8>,
    target_artifact: Vec<u8>,
    object_key: Vec<u8>,
    release_digest: [u8; 32],
    bundle_digest: [u8; 32],
    envelope_digest: [u8; 32],
    recipient_key_id: [u8; 16],
    signing_key_id: [u8; 16],
    authorization_signature: [u8; 64],
}

struct ActiveOperational {
    profile_name: Vec<u8>,
    binding_generation: u64,
    deployment_generation: u64,
    target_artifact: Vec<u8>,
    domain: DeployedArtifact,
    retiring: bool,
}

fn fail(stage: u32) -> ! {
    catten_rt::logln!("[agent] fatal stage={:#x}", stage);
    config::write_u32_release(charlotte_launch::agent_status::STAGE, stage);
    catten_rt::domain_abort()
}

fn memory_from_bytes(bytes: &[u8]) -> Option<OwnedMemory> {
    let memory = OwnedMemory::allocate(bytes.len().div_ceil(4096).max(1)).ok()?;
    let mut mapping = memory.map_writable().ok()?;
    mapping.as_mut_slice().get_mut(..bytes.len())?.copy_from_slice(bytes);
    mapping.unmap().ok()
}

fn lookup(names: ConnectionRef<'_>, service: u64) -> Option<Connection> {
    wait_for_registered_name_owned(names, service).map(|(_, connection)| connection)
}

fn manifest_admission_trust(ctx: &Context) -> Option<charlotte_launch::trust::AdmissionTrust> {
    match ctx.manifest_value(charlotte_launch::ADMISSION_TRUST_MANIFEST_KEY) {
        Some(ManifestValue::Bytes(bytes)) => charlotte_launch::trust::AdmissionTrust::decode(bytes),
        _ => None,
    }
}

fn deployment_names(dns_connection: ConnectionRef<'_>) -> Vec<Vec<u8>> {
    let Some(reply) =
        dns_connection.call(dns::OP_DEPLOY_LIST, 0).ok().and_then(|call| call.wait().ok())
    else {
        return Vec::new();
    };
    let Ok(len) = usize::try_from(reply.result) else {
        return Vec::new();
    };
    let Some(memory) = reply.memory else {
        return Vec::new();
    };
    let Ok(mapping) = memory.map_read_only() else {
        return Vec::new();
    };
    let Some(bytes) = mapping.as_slice().get(..len) else {
        return Vec::new();
    };
    let Some(count_bytes) = bytes.get(..2) else {
        return Vec::new();
    };
    let count = usize::from(u16::from_le_bytes(count_bytes.try_into().unwrap_or_default()));
    let mut offset = 2;
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let Some(name_len) = bytes.get(offset).copied().map(usize::from) else {
            return Vec::new();
        };
        let start = offset + 1;
        let Some(name) = bytes.get(start..start.saturating_add(name_len)) else {
            return Vec::new();
        };
        if name.is_empty() || name.len() > charlotte_launch::deployment::MAX_ARTIFACT_NAME_LEN {
            return Vec::new();
        }
        names.push(name.to_vec());
        offset = start + name_len;
    }
    if offset == bytes.len() {
        names
    } else {
        Vec::new()
    }
}

fn operational_bindings(dns_connection: ConnectionRef<'_>) -> Vec<OperationalBinding> {
    let Some(reply) =
        dns_connection.call(dns::OP_OPERATIONAL_LIST, 0).ok().and_then(|call| call.wait().ok())
    else {
        return Vec::new();
    };
    let Ok(len) = usize::try_from(reply.result) else {
        return Vec::new();
    };
    let Some(memory) = reply.memory else {
        return Vec::new();
    };
    let Ok(mapping) = memory.map_read_only() else {
        return Vec::new();
    };
    let Some(bytes) = mapping.as_slice().get(..len) else {
        return Vec::new();
    };
    let Some(expected_count) = bytes
        .get(10..12)
        .and_then(|count| <[u8; 2]>::try_from(count).ok())
        .map(u16::from_le_bytes)
        .map(usize::from)
    else {
        return Vec::new();
    };
    let Some(bindings) = charlotte_launch::operations_pickup::decode_catalog_list(bytes) else {
        return Vec::new();
    };
    let result = bindings
        .map(|binding| OperationalBinding {
            generation: binding.generation,
            bundle_sequence: binding.bundle_sequence,
            sequence: binding.sequence,
            expires_unix_seconds: binding.expires_unix_seconds,
            profile_kind: binding.profile_kind,
            release_name: binding.release_name.to_vec(),
            profile_name: binding.profile_name.to_vec(),
            target_artifact: binding.target_artifact.to_vec(),
            object_key: binding.object_key.to_vec(),
            release_digest: binding.release_digest,
            bundle_digest: binding.bundle_digest,
            envelope_digest: binding.envelope_digest,
            recipient_key_id: binding.recipient_key_id,
            signing_key_id: binding.signing_key_id,
            authorization_signature: binding.authorization_signature,
        })
        .collect::<Vec<_>>();
    if result.len() == expected_count {
        result
    } else {
        Vec::new()
    }
}

fn query_deployment(dns_connection: ConnectionRef<'_>, name: &[u8]) -> Option<DeploymentInfo> {
    let request = memory_from_bytes(name)?;
    let reply = dns_connection
        .call_move(dns::OP_DEPLOY_QUERY_NAMED, name.len() as u64, request)
        .ok()?
        .wait()
        .ok()?;
    let len = usize::try_from(reply.result).ok()?;
    let memory = reply.memory?;
    let mapping = memory.map_read_only().ok()?;
    decode_deployment(mapping.as_slice().get(..len)?)
}

fn query_release(dns_connection: ConnectionRef<'_>, release_name: &[u8]) -> Option<Vec<u8>> {
    let request = memory_from_bytes(release_name)?;
    let reply = dns_connection
        .call_move(dns::OP_RELEASE_QUERY_NAMED, release_name.len() as u64, request)
        .ok()?
        .wait()
        .ok()?;
    let len = usize::try_from(reply.result).ok()?;
    if !(charlotte_launch::release::HEADER_LEN..=charlotte_launch::release::MAX_RELEASE_LEN)
        .contains(&len)
    {
        return None;
    }
    let memory = reply.memory?;
    let mapping = memory.map_read_only().ok()?;
    let bytes = mapping.as_slice().get(..len)?;
    charlotte_launch::release::decode(bytes)?;
    Some(bytes.to_vec())
}

fn decode_deployment(bytes: &[u8]) -> Option<DeploymentInfo> {
    if bytes.len() < 56 {
        return None;
    }
    let descriptor = if bytes.len() == 56 {
        Vec::new()
    } else {
        let descriptor_len =
            usize::try_from(u32::from_le_bytes(bytes.get(56..60)?.try_into().ok()?)).ok()?;
        if descriptor_len == 0
            || descriptor_len > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
            || bytes.len() != 60 + descriptor_len
        {
            return None;
        }
        bytes[60..].to_vec()
    };
    Some(DeploymentInfo {
        generation: u64::from_le_bytes(bytes[0..8].try_into().ok()?),
        object_id: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        node_key: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
        artifact_digest: bytes[24..56].try_into().ok()?,
        descriptor,
    })
}

fn fetch_from_local_store(
    names: ConnectionRef<'_>,
    object_id: u64,
    expected_digest: &[u8; 32],
    cluster_key: &[u8; 32],
    artifact_name: &[u8],
) -> Option<(OwnedMemory, usize)> {
    let connection = lookup(names, objstore::NAME)?;
    let reply = connection.as_ref().call(objstore::OP_READ, object_id).ok()?.wait().ok()?;
    let len = usize::try_from(reply.result).ok()?;
    let memory = reply.memory?;
    if len == 0 || len > memory.len() {
        return None;
    }
    let mapping = memory.map_read_only().ok()?;
    let bytes = mapping.as_slice().get(..len)?;
    if charlotte_launch::sha256::digest(bytes) != *expected_digest
        || charlotte_launch::signature_note::verify_elf_for_name(bytes, cluster_key, artifact_name)
            != charlotte_launch::signature_note::VerifyOutcome::Valid
    {
        return None;
    }
    let memory = mapping.unmap().ok()?;
    Some((memory, len))
}

fn fetch_from_central_store(
    names: ConnectionRef<'_>,
    descriptor: &charlotte_launch::deployment::DeploymentDescriptor<'_>,
    cluster_key: &[u8; 32],
) -> Option<Vec<u8>> {
    catten_rt::logln!(
        "[agent] fetching {:?} from S3 key {:?}",
        core::str::from_utf8(descriptor.artifact_name).unwrap_or("<invalid>"),
        core::str::from_utf8(descriptor.object_key).unwrap_or("<invalid>")
    );
    let artifact = fetch_s3_object(
        names,
        descriptor.object_key,
        charlotte_launch::MAX_ARTIFACT_ELF_SIZE,
        &descriptor.artifact_digest,
    )?;
    if charlotte_launch::signature_note::verify_elf_for_name(
        &artifact,
        cluster_key,
        descriptor.artifact_name,
    ) != charlotte_launch::signature_note::VerifyOutcome::Valid
    {
        catten_rt::logln!("[agent] fetched artifact failed identity validation");
        return None;
    }
    catten_rt::logln!("[agent] fetched and verified {} bytes", artifact.len());
    Some(artifact)
}

fn fetch_s3_object(
    names: ConnectionRef<'_>,
    key: &[u8],
    max_len: usize,
    expected_digest: &[u8; 32],
) -> Option<Vec<u8>> {
    let (_, connection) = try_registered_name_bytes_owned(names, b"s3")?;
    let client = S3Client::new(connection.as_ref());
    let (mut get, info) = client.get(charlotte_protocol_s3::ObjectRequest::get(key)).ok()?;
    let expected_len = usize::try_from(info.content_length).ok()?;
    if info.status != 200 || expected_len == 0 || expected_len > max_len {
        return None;
    }
    let mut bytes = Vec::with_capacity(expected_len);
    while let Some(chunk) = get.read().ok()? {
        let (memory, len) = chunk.into_parts();
        let mapping = memory.map_read_only().ok()?;
        bytes.extend_from_slice(mapping.as_slice().get(..len)?);
        if bytes.len() > expected_len {
            return None;
        }
    }
    get.close().ok()?;
    (bytes.len() == expected_len && charlotte_launch::sha256::digest(&bytes) == *expected_digest)
        .then_some(bytes)
}

fn trusted_unix_seconds(names: ConnectionRef<'_>) -> Option<u64> {
    let time = lookup(names, time::NAME)?;
    let reply = time.as_ref().call(time::OP_UNIX_SECONDS, 0).ok()?.wait().ok()?;
    u64::try_from(reply.result).ok().filter(|seconds| *seconds != 0)
}

fn launch(
    names: ConnectionRef<'_>,
    name: &[u8],
    entry: &DeploymentInfo,
    trust: &charlotte_launch::trust::AdmissionTrust,
    my_node_key: u64,
) -> Option<ActiveDeployment> {
    if entry.descriptor.is_empty() {
        let (artifact, artifact_len) = fetch_from_local_store(
            names,
            entry.object_id,
            &entry.artifact_digest,
            &trust.artifact_key,
            name,
        )?;
        let domain = launch_artifact(artifact, artifact_len, name).ok()?;
        catten_rt::logln!(
            "[agent] launched legacy {:?} generation={} asid={}",
            core::str::from_utf8(name).unwrap_or("<invalid>"),
            entry.generation,
            domain.asid()
        );
        return Some(ActiveDeployment {
            name: name.to_vec(),
            generation: entry.generation,
            domain,
            published: false,
            retiring: false,
        });
    }
    if charlotte_launch::deployment::verify(&entry.descriptor, &trust.deployment_key)
        != charlotte_launch::deployment::VerifyOutcome::Valid
    {
        return None;
    }
    let descriptor = charlotte_launch::deployment::decode(&entry.descriptor)?;
    if descriptor.artifact_name != name
        || (descriptor.node_key != 0 && descriptor.node_key != my_node_key)
        || descriptor.artifact_digest != entry.artifact_digest
    {
        return None;
    }
    let artifact = fetch_from_central_store(names, &descriptor, &trust.artifact_key)?;
    let artifact_memory = memory_from_bytes(&artifact)?;
    let descriptor_memory = memory_from_bytes(&entry.descriptor)?;
    let domain = launch_scoped_artifact_named(
        artifact_memory,
        artifact.len(),
        name,
        descriptor_memory,
        entry.descriptor.len(),
    )
    .ok()?;
    catten_rt::logln!(
        "[agent] launched {:?} generation={} asid={}",
        core::str::from_utf8(name).unwrap_or("<invalid>"),
        entry.generation,
        domain.asid()
    );
    Some(ActiveDeployment {
        name: name.to_vec(),
        generation: entry.generation,
        domain,
        published: false,
        retiring: false,
    })
}

fn launch_operational(
    names: ConnectionRef<'_>,
    dns_connection: ConnectionRef<'_>,
    binding: &OperationalBinding,
    deployment: &DeploymentInfo,
    trust: &charlotte_launch::trust::AdmissionTrust,
    my_node_key: u64,
) -> Option<ActiveOperational> {
    if deployment.descriptor.is_empty()
        || deployment.node_key != my_node_key
        || deployment.artifact_digest == [0; 32]
    {
        return None;
    }
    let descriptor = charlotte_launch::deployment::decode(&deployment.descriptor)?;
    if descriptor.artifact_name != binding.target_artifact
        || descriptor.artifact_digest != deployment.artifact_digest
    {
        return None;
    }
    if charlotte_launch::deployment::verify(&deployment.descriptor, &trust.deployment_key)
        != charlotte_launch::deployment::VerifyOutcome::Valid
    {
        return None;
    }
    let artifact = fetch_from_central_store(names, &descriptor, &trust.artifact_key)?;
    let envelope = fetch_s3_object(
        names,
        &binding.object_key,
        charlotte_launch::operations::MAX_ENVELOPE_LEN,
        &binding.envelope_digest,
    )?;
    let release = query_release(dns_connection, &binding.release_name)?;
    let now_unix_seconds = trusted_unix_seconds(names)?;
    let wire_binding = charlotte_launch::operations_pickup::CatalogBinding {
        generation: binding.generation,
        bundle_sequence: binding.bundle_sequence,
        sequence: binding.sequence,
        expires_unix_seconds: binding.expires_unix_seconds,
        profile_kind: binding.profile_kind,
        release_name: &binding.release_name,
        profile_name: &binding.profile_name,
        target_artifact: &binding.target_artifact,
        object_key: &binding.object_key,
        release_digest: binding.release_digest,
        bundle_digest: binding.bundle_digest,
        envelope_digest: binding.envelope_digest,
        recipient_key_id: binding.recipient_key_id,
        signing_key_id: binding.signing_key_id,
        authorization_signature: binding.authorization_signature,
    };
    let pickup = charlotte_launch::operations_pickup::Pickup {
        binding: wire_binding,
        now_unix_seconds,
        release: &release,
        artifact: &artifact,
        descriptor: &deployment.descriptor,
        envelope: &envelope,
    };
    let mut package = vec![0; pickup.encoded_len()?];
    let package_len = pickup.encode(&mut package)?;
    let package = memory_from_bytes(&package)?;
    let domain =
        launch_operational_connector(package, package_len, &binding.target_artifact).ok()?;
    catten_rt::logln!(
        "[agent] launched operational profile={:?} target={:?} generation={} asid={}",
        core::str::from_utf8(&binding.profile_name).unwrap_or("<invalid>"),
        core::str::from_utf8(&binding.target_artifact).unwrap_or("<invalid>"),
        binding.generation,
        domain.asid()
    );
    Some(ActiveOperational {
        profile_name: binding.profile_name.clone(),
        binding_generation: binding.generation,
        deployment_generation: deployment.generation,
        target_artifact: binding.target_artifact.clone(),
        domain,
        retiring: false,
    })
}

fn publish_if_ready(
    names: ConnectionRef<'_>,
    dns_connection: ConnectionRef<'_>,
    active: &mut ActiveDeployment,
) {
    if active.published || try_registered_name_bytes_owned(names, &active.name).is_none() {
        return;
    }
    let mut request = Vec::with_capacity(8 + active.name.len());
    request.extend_from_slice(&active.generation.to_le_bytes());
    request.extend_from_slice(&active.name);
    let Some(name_memory) = memory_from_bytes(&request) else {
        return;
    };
    let Some(reply) = dns_connection
        .call_move(dns::OP_REGISTER_DEPLOYMENT_NAMED, request.len() as u64, name_memory)
        .ok()
        .and_then(|call| call.wait().ok())
    else {
        return;
    };
    if reply.result >= 1 {
        catten_rt::logln!(
            "[agent] published {:?} deployment_generation={} service_generation={}",
            core::str::from_utf8(&active.name).unwrap_or("<invalid>"),
            active.generation,
            reply.result
        );
        active.published = true;
        config::write::<u64>(charlotte_launch::agent_status::SERVED_GENERATION, active.generation);
        config::write_u32_release(charlotte_launch::agent_status::STAGE, STAGE_SERVING);
    }
}

fn drain_for_node_shutdown(
    active: &mut Vec<ActiveDeployment>,
    operational: &mut Vec<ActiveOperational>,
    request: catten_rt::ShutdownRequest,
) -> catten_rt::ShutdownRequest {
    catten_rt::logln!("[agent] node shutdown requested deadline_ms={}", request.deadline_ms);
    config::write_u32_release(charlotte_launch::agent_status::STAGE, STAGE_DRAINING_APPLICATIONS);
    for running in active.iter_mut() {
        running.retiring = true;
    }
    for running in operational.iter_mut() {
        running.retiring = true;
    }

    let mut draining_connectors = false;
    loop {
        let mut index = 0;
        while index < active.len() {
            match active[index].domain.poll_node_shutdown(request.deadline_ms) {
                Ok(true) => {
                    active.swap_remove(index);
                    continue;
                }
                Ok(false) => {}
                Err(_) => fail(STAGE_FAIL),
            }
            index += 1;
        }

        if active.is_empty() {
            if !draining_connectors {
                catten_rt::logln!("[agent] application domains drained; draining connectors");
                config::write_u32_release(
                    charlotte_launch::agent_status::STAGE,
                    STAGE_DRAINING_CONNECTORS,
                );
                draining_connectors = true;
            }
            let mut index = 0;
            while index < operational.len() {
                match operational[index].domain.poll_node_shutdown(request.deadline_ms) {
                    Ok(true) => {
                        operational.swap_remove(index);
                        continue;
                    }
                    Ok(false) => {}
                    Err(_) => fail(STAGE_FAIL),
                }
                index += 1;
            }
        }

        if active.is_empty() && operational.is_empty() {
            catten_rt::logln!("[agent] child domains drained for node shutdown");
            config::write_u32_release(charlotte_launch::agent_status::STAGE, STAGE_SHUTDOWN_READY);
            return request;
        }
        sleep_ms(RETIREMENT_POLL_MS);
    }
}

fn serve(ctx: &Context) -> catten_rt::ShutdownRequest {
    let names = ctx.bootstrap_connection().unwrap_or_else(|| fail(STAGE_FAIL));
    let poll_ms = match ctx.manifest_value(manifest_key(b"poll-ms")) {
        Some(ManifestValue::Unsigned(ms)) => ms,
        _ => 500,
    };
    let (_, dns_connection) =
        match wait_for_registered_name_or_shutdown_owned(ctx, names, dns::NAME) {
            Ok(found) => found,
            Err(request) => return request,
        };
    let (_, net_connection) =
        match wait_for_registered_name_or_shutdown_owned(ctx, names, net::NAME) {
            Ok(found) => found,
            Err(request) => return request,
        };
    let my_node_key = net_connection
        .as_ref()
        .call(net::OP_STATUS, 0)
        .ok()
        .and_then(|call| call.wait().ok())
        .and_then(|reply| {
            let (link, mac) = charlotte_protocol_net::decode_status(reply.result);
            (link != 0).then_some(node_identity::fnv1a(&mac) & 0xffff_ffff)
        })
        .unwrap_or_else(|| fail(STAGE_FAIL));
    config::write::<u64>(charlotte_launch::agent_status::NODE_KEY, my_node_key);
    config::write_u32_release(charlotte_launch::agent_status::STAGE, STAGE_IDENTITY);

    let trust = manifest_admission_trust(ctx).unwrap_or_else(|| fail(STAGE_FAIL));
    let mut active: Vec<ActiveDeployment> = Vec::new();
    let mut operational: Vec<ActiveOperational> = Vec::new();
    if let Err(request) = wait_for_local_ready_or_shutdown(ctx, names) {
        return request;
    }
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return drain_for_node_shutdown(&mut active, &mut operational, request);
        }
        let desired_names = deployment_names(dns_connection.as_ref());
        let desired_operations = operational_bindings(dns_connection.as_ref());

        for running in &mut active {
            let still_desired =
                query_deployment(dns_connection.as_ref(), &running.name).is_some_and(|entry| {
                    entry.node_key == my_node_key && entry.generation == running.generation
                }) && !desired_operations
                    .iter()
                    .any(|binding| binding.target_artifact == running.name);
            running.retiring |= !still_desired;
            if !running.retiring {
                publish_if_ready(names, dns_connection.as_ref(), running);
            }
        }
        let mut index = 0;
        while index < active.len() {
            if active[index].retiring {
                match active[index].domain.poll_retire() {
                    Ok(true) => {
                        active.swap_remove(index);
                        if active.is_empty() {
                            config::write_u32_release(
                                charlotte_launch::agent_status::STAGE,
                                STAGE_RETIRED,
                            );
                        }
                        continue;
                    }
                    Ok(false) => {}
                    Err(_) => fail(STAGE_FAIL),
                }
            }
            index += 1;
        }

        for running in &mut operational {
            let binding_matches = desired_operations.iter().any(|binding| {
                binding.profile_name == running.profile_name
                    && binding.target_artifact == running.target_artifact
                    && binding.generation == running.binding_generation
            });
            let deployment_matches = query_deployment(
                dns_connection.as_ref(),
                &running.target_artifact,
            )
            .is_some_and(|entry| {
                entry.node_key == my_node_key && entry.generation == running.deployment_generation
            });
            running.retiring |= !binding_matches || !deployment_matches;
        }
        let mut index = 0;
        while index < operational.len() {
            if operational[index].retiring {
                match operational[index].domain.poll_retire() {
                    Ok(true) => {
                        operational.swap_remove(index);
                        continue;
                    }
                    Ok(false) => {}
                    Err(_) => fail(STAGE_FAIL),
                }
            }
            index += 1;
        }

        for binding in &desired_operations {
            if operational
                .iter()
                .any(|running| running.profile_name == binding.profile_name && !running.retiring)
                || active.iter().any(|running| running.name == binding.target_artifact)
            {
                continue;
            }
            if let Some(deployment) =
                query_deployment(dns_connection.as_ref(), &binding.target_artifact)
                && let Some(running) = launch_operational(
                    names,
                    dns_connection.as_ref(),
                    binding,
                    &deployment,
                    &trust,
                    my_node_key,
                )
            {
                operational.push(running);
            }
        }

        for name in desired_names {
            if desired_operations.iter().any(|binding| binding.target_artifact == name) {
                continue;
            }
            if active.iter().any(|running| running.name == name) {
                continue;
            }
            if let Some(entry) = query_deployment(dns_connection.as_ref(), &name)
                && entry.node_key == my_node_key
                && let Some(running) = launch(names, &name, &entry, &trust, my_node_key)
            {
                active.push(running);
            }
        }
        let retirement_in_progress = active.iter().any(|running| running.retiring)
            || operational.iter().any(|running| running.retiring);
        let delay = if retirement_in_progress {
            poll_ms.min(RETIREMENT_POLL_MS)
        } else {
            poll_ms
        };
        if let Err(request) = sleep_ms_or_shutdown(ctx, delay) {
            return drain_for_node_shutdown(&mut active, &mut operational, request);
        }
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}
