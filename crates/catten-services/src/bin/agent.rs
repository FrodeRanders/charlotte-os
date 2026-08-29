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

use alloc::vec::Vec;

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
    try_registered_name_bytes_owned,
    wait_for_registered_name_owned,
};

catten_rt::entry!(main);

const STAGE_IDENTITY: u32 = 2;
const STAGE_SERVING: u32 = 6;
const STAGE_RETIRED: u32 = 7;
const STAGE_FAIL: u32 = 0xdead;

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

fn read_cluster_key(dns_connection: ConnectionRef<'_>) -> Option<[u8; 32]> {
    let reply = dns_connection.call(dns::OP_KEY, 0).ok()?.wait().ok()?;
    if reply.result < 32 {
        return None;
    }
    let memory = reply.memory?;
    let mapping = memory.map_read_only().ok()?;
    mapping.as_slice().get(..32)?.try_into().ok()
}

fn manifest_cluster_key(ctx: &Context) -> Option<[u8; 32]> {
    match ctx.manifest_value(charlotte_launch::CLUSTER_KEY_MANIFEST_KEY) {
        Some(ManifestValue::Bytes(bytes)) => <[u8; 32]>::try_from(bytes).ok(),
        _ => None,
    }
}

fn local_node_key(names: ConnectionRef<'_>) -> Option<u64> {
    let connection = lookup(names, net::NAME)?;
    let reply = connection.as_ref().call(net::OP_STATUS, 0).ok()?.wait().ok()?;
    let (link, mac) = charlotte_protocol_net::decode_status(reply.result);
    (link != 0).then_some(node_identity::fnv1a(&mac) & 0xffff_ffff)
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
    let (_, connection) = try_registered_name_bytes_owned(names, b"s3")?;
    let client = S3Client::new(connection.as_ref());
    let request = charlotte_protocol_s3::ObjectRequest::get(descriptor.object_key);
    let (mut get, info) = client.get(request).ok()?;
    let expected_len = usize::try_from(info.content_length).ok()?;
    if info.status != 200
        || expected_len == 0
        || expected_len > charlotte_launch::MAX_ARTIFACT_ELF_SIZE
    {
        return None;
    }
    let mut artifact = Vec::with_capacity(expected_len);
    while let Some(chunk) = get.read().ok()? {
        let (memory, len) = chunk.into_parts();
        let mapping = memory.map_read_only().ok()?;
        artifact.extend_from_slice(mapping.as_slice().get(..len)?);
        if artifact.len() > expected_len {
            return None;
        }
    }
    get.close().ok()?;
    if artifact.len() != expected_len
        || charlotte_launch::sha256::digest(&artifact) != descriptor.artifact_digest
        || charlotte_launch::signature_note::verify_elf_for_name(
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

fn launch(
    names: ConnectionRef<'_>,
    name: &[u8],
    entry: &DeploymentInfo,
    cluster_key: &[u8; 32],
    my_node_key: u64,
) -> Option<ActiveDeployment> {
    if entry.descriptor.is_empty() {
        let (artifact, artifact_len) = fetch_from_local_store(
            names,
            entry.object_id,
            &entry.artifact_digest,
            cluster_key,
            name,
        )?;
        let domain = launch_artifact(artifact, artifact_len, name).ok()?;
        return Some(ActiveDeployment {
            name: name.to_vec(),
            generation: entry.generation,
            domain,
            published: false,
            retiring: false,
        });
    }
    if charlotte_launch::deployment::verify(&entry.descriptor, cluster_key)
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
    let artifact = fetch_from_central_store(names, &descriptor, cluster_key)?;
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

fn main(ctx: Context) -> ! {
    let names = ctx.bootstrap_connection().unwrap_or_else(|| fail(STAGE_FAIL));
    let poll_ms = match ctx.manifest_value(manifest_key(b"poll-ms")) {
        Some(ManifestValue::Unsigned(ms)) => ms,
        _ => 500,
    };
    let dns_connection = lookup(names, dns::NAME).unwrap_or_else(|| fail(STAGE_FAIL));
    let my_node_key = local_node_key(names).unwrap_or_else(|| fail(STAGE_FAIL));
    config::write::<u64>(charlotte_launch::agent_status::NODE_KEY, my_node_key);
    config::write_u32_release(charlotte_launch::agent_status::STAGE, STAGE_IDENTITY);

    let cluster_key = manifest_cluster_key(&ctx).unwrap_or_else(|| fail(STAGE_FAIL));
    if read_cluster_key(dns_connection.as_ref()).is_some_and(|key| key != cluster_key) {
        fail(STAGE_FAIL);
    }

    let mut active: Vec<ActiveDeployment> = Vec::new();
    loop {
        let desired_names = deployment_names(dns_connection.as_ref());

        for running in &mut active {
            let still_desired = query_deployment(dns_connection.as_ref(), &running.name)
                .is_some_and(|entry| {
                    entry.node_key == my_node_key && entry.generation == running.generation
                });
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

        for name in desired_names {
            if active.iter().any(|running| running.name == name) {
                continue;
            }
            if let Some(entry) = query_deployment(dns_connection.as_ref(), &name)
                && entry.node_key == my_node_key
                && let Some(running) = launch(names, &name, &entry, &cluster_key, my_node_key)
            {
                active.push(running);
            }
        }
        sleep_ms(poll_ms);
    }
}
