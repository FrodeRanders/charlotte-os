//! Cluster deploy agent: the node-side "picker-upper" of the server-class
//! cluster vision (manual Chapter 19).
//!
//! Each agent:
//!
//! 1. Polls the replicated deployment manifest for assignments to this node.
//! 2. Fetches the pinned object-store bytes, verifies their SHA-256 and CLS2
//!    name-bound cluster signature, and passes the memory object to the
//!    privileged deployment syscall.
//! 3. The kernel loads that exact ELF in a fresh address space; the service
//!    registers locally and the agent publishes it to the distributed catalog.
//! 4. On reassignment the agent retires and reclaims the spawned domain. The
//!    resulting endpoint close drives generation-fenced distributed removal.
#![no_std]
#![no_main]

extern crate alloc;

catten_rt::entry!(main);

use catten_rt::{
    Context,
    ManifestValue,
    config,
    manifest_key,
};
use catten_services::{
    deploy,
    dns,
    net,
    node_identity,
    ns,
    objstore,
    wait_reply,
};
use catten_syscall::*;

/// Status-page stage markers (offset 0).
const STAGE_IDENTITY: u32 = 2;
const STAGE_SERVING: u32 = 6;
const STAGE_RETIRED: u32 = 7;
const STAGE_FAIL: u32 = 0xdead;

const DATA_VADDR: usize = 0x0000_0000_2000_0000;
const REPLY_SPINS: u64 = 50_000_000;

/// A replicated deployment record as decoded from `OP_DEPLOY_QUERY`
/// (`[generation][object_id][node_key][artifact_sha256]`; Raft establishes
/// the authoritative generation and the digest pins its exact bytes).
struct DeploymentInfo {
    generation: u64,
    object_id: u64,
    node_key: u64,
    artifact_digest: [u8; 32],
}

fn fail(stage: u32) -> ! {
    config::write_u32_release(0, stage);
    unsafe { thread_exit() }
}

fn lookup(ns_connection: u64, name: u64) -> u64 {
    let lookup = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_LOOKUP,
        name,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if lookup == 0 {
        return 0;
    }
    let (generation, connection) = catten_services::spin_reply(lookup);
    if generation < 1 || connection == 0 {
        0
    } else {
        connection
    }
}

/// The cluster's public key as committed by the key ceremony, read through
/// the local dns replica (replicated state).
fn read_cluster_key(dns_conn: u64) -> Option<[u8; 32]> {
    let call = ipc_scalar_call(dns_conn, dns::OP_KEY, 0);
    if call == 0 {
        return None;
    }
    let (status, size, _returned_connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if memory == 0 || (status as i64) < 0 || size < 32 {
        if memory != 0 {
            memory_close(memory);
        }
        return None;
    }
    if memory_map(memory, DATA_VADDR, false) != 0 {
        memory_close(memory);
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((DATA_VADDR + index) as *const u8) };
    }
    memory_unmap(memory);
    memory_close(memory);
    Some(key)
}

/// The build-time cluster public key, as the kernel wrote it into the launch
/// manifest when it spawned this service.
fn manifest_cluster_key(ctx: &Context) -> Option<[u8; 32]> {
    match ctx.manifest_value(charlotte_launch::CLUSTER_KEY_MANIFEST_KEY) {
        Some(ManifestValue::Bytes(bytes)) => <[u8; 32]>::try_from(bytes).ok(),
        _ => None,
    }
}

/// This node's cluster key: the FNV-1a of its NIC MAC (truncated to 32 bits,
/// matching the node-name suffix the dns derives for the cluster members).
fn local_node_key(ns_connection: u64) -> Option<u64> {
    let net_lookup = ipc_scalar_call(ns_connection, ns::OP_LOOKUP, net::NAME);
    if net_lookup == 0 {
        return None;
    }
    let (net_generation, net_conn) = unsafe { wait_reply(net_lookup, REPLY_SPINS) };
    if net_generation < 1 || net_conn == 0 {
        return None;
    }
    let status_call = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status_call == 0 {
        return None;
    }
    let (status, _) = unsafe { wait_reply(status_call, REPLY_SPINS) };
    let (link, local_mac) = charlotte_protocol_net::decode_status(status);
    if link == 0 {
        return None;
    }
    Some(node_identity::fnv1a(&local_mac) & 0xffff_ffff)
}

/// Blocking query of the replicated deployment manifest for `packed_name`
/// (packed LE). Used by the polling loop, where nothing invokes the agent
/// yet, so blocking on the dns reply cannot deadlock.
fn query_deployment(dns_conn: u64, packed_name: u64) -> Option<DeploymentInfo> {
    let call = ipc_scalar_call(dns_conn, dns::OP_DEPLOY_QUERY, packed_name);
    if call == 0 {
        return None;
    }
    let (_status, _size, _returned_connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    let entry = decode_deployment(memory);
    if memory != 0 {
        memory_close(memory);
    }
    entry
}

/// Read the artifact at `object_id` from the object store, verify that it is
/// exactly the deployed artifact (SHA-256 identity) and that its signature
/// note validates against the cluster's public key.
fn fetch_and_verify(
    obj_conn: u64,
    object_id: u64,
    expected_digest: &[u8; 32],
    cluster_key: &[u8; 32],
) -> Result<(u64, usize), ()> {
    let read = ipc_scalar_call(obj_conn, objstore::OP_READ, object_id);
    if read == 0 {
        return Err(());
    }
    let (status, size, returned_connection, returned_memory) = ipc_reply_wait_with_memory(read);
    ipc_close(read);
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    let len = usize::try_from(size).map_err(|_| ())?;
    if status != 0 || returned_memory == 0 || len == 0 || len > memory_size(returned_memory) {
        if returned_memory != 0 {
            memory_close(returned_memory);
        }
        return Err(());
    }
    if memory_map(returned_memory, DATA_VADDR, false) != 0 {
        memory_close(returned_memory);
        return Err(());
    }
    let mut artifact = alloc::vec::Vec::with_capacity(len);
    for index in 0..len {
        artifact.push(unsafe { core::ptr::read_volatile((DATA_VADDR + index) as *const u8) });
    }
    memory_unmap(returned_memory);
    // The artifact is the note-signed `greet` ELF: it must be exactly the
    // artifact this agent is built to serve, and its signature note must
    // validate against the cluster's public key.
    if charlotte_launch::sha256::digest(&artifact) != *expected_digest {
        memory_close(returned_memory);
        return Err(());
    }
    if charlotte_launch::signature_note::verify_elf_for_name(
        &artifact,
        cluster_key,
        b"greet",
    )
        != charlotte_launch::signature_note::VerifyOutcome::Valid
    {
        memory_close(returned_memory);
        return Err(());
    }
    Ok((returned_memory, len))
}

/// Publish the independently running artifact domain, then supervise it until
/// the assignment moves elsewhere.
fn supervise(
    dns_conn: u64,
    ns_connection: u64,
    my_node_key: u64,
    poll_ms: u64,
    generation: u64,
) -> ! {
    // A blocking node-name lookup is the startup synchronization: it proves
    // that the spawned ELF, not this agent, created and registered its endpoint.
    let service_connection = lookup(ns_connection, deploy::NAME);
    if service_connection == 0 {
        fail(STAGE_FAIL);
    }
    ipc_close(service_connection);

    // Publish through the dns. The reply is deferred until the catalog entry
    // has replicated (locally through the leader, or relayed to it); poll it
    // while serving rather than blocking on it.
    let publish = ipc_scalar_call(dns_conn, dns::OP_REGISTER, deploy::NAME);
    if publish == 0 {
        fail(STAGE_FAIL);
    }
    // Polling deployment query; the retirement decision is made from its
    // reply without ever blocking the serve loop.
    let mut deploy_query = ipc_scalar_call(dns_conn, dns::OP_DEPLOY_QUERY, deploy::NAME);
    if deploy_query == 0 {
        fail(STAGE_FAIL);
    }

    let mut published = false;
    loop {
        cq_wait_timeout(1, poll_ms, 0);
        // Publish progress: has the catalog registration committed?
        if !published {
            let (status, result, _returned_connection) = ipc_reply_poll(publish);
            if status == 0 {
                ipc_close(publish);
                if result >= 1 {
                    published = true;
                    config::write::<u64>(8, generation);
                    config::write_u32_release(0, STAGE_SERVING);
                } else {
                    fail(STAGE_FAIL);
                }
            }
        }

        // Retirement check: is the artifact still assigned to this node? The
        // query is polled, never blocked on.
        if published {
            let (status, _size, _returned_connection, memory) =
                ipc_reply_poll_with_memory(deploy_query);
            if status == 0 {
                ipc_close(deploy_query);
                let entry = decode_deployment(memory);
                if memory != 0 {
                    memory_close(memory);
                }
                let still_mine = entry.is_some_and(|entry| entry.node_key == my_node_key);
                if !still_mine {
                    loop {
                        let retirement = retire_artifact();
                        match retirement {
                            0 => break,
                            1 => cq_wait_timeout(1, poll_ms.min(25), 0),
                            _ => fail(STAGE_FAIL),
                        };
                    }
                    config::write_u32_release(0, STAGE_RETIRED);
                    unsafe { thread_exit() }
                }
                deploy_query = ipc_scalar_call(dns_conn, dns::OP_DEPLOY_QUERY, deploy::NAME);
                if deploy_query == 0 {
                    fail(STAGE_FAIL);
                }
            }
        }
    }
}

/// Decode a `OP_DEPLOY_QUERY` reply page (56 bytes, `[generation][object_id]
/// [node_key][artifact_sha256]`), mapped at `DATA_VADDR`. Returns `None` when the memory
/// cap is absent (the query errored or found nothing).
fn decode_deployment(memory: u64) -> Option<DeploymentInfo> {
    if memory == 0 {
        return None;
    }
    if memory_map(memory, DATA_VADDR, false) != 0 {
        return None;
    }
    if memory_size(memory) < 56 {
        return None;
    }
    let mut bytes = [0u8; 56];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((DATA_VADDR + index) as *const u8) };
    }
    memory_unmap(memory);
    Some(DeploymentInfo {
        generation: u64::from_le_bytes(bytes[0..8].try_into().ok()?),
        object_id: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        node_key: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
        artifact_digest: bytes[24..56].try_into().ok()?,
    })
}

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_cap().unwrap_or_else(|| fail(STAGE_FAIL));
    let poll_ms = match ctx.manifest_value(manifest_key(b"poll-ms")) {
        Some(ManifestValue::Unsigned(ms)) => ms,
        _ => 500,
    };

    let obj_conn = lookup(ns_connection, objstore::NAME);
    let dns_conn = lookup(ns_connection, dns::NAME);
    if obj_conn == 0 || dns_conn == 0 {
        fail(STAGE_FAIL);
    }

    // This node's cluster key, derived the same way the dns derives its node
    // identity (from the NIC MAC). Publish it so the kernel verifier can tell
    // which cluster node this guest is.
    let my_node_key = match local_node_key(ns_connection) {
        Some(key) => key,
        None => fail(STAGE_FAIL),
    };
    config::write::<u64>(16, my_node_key);
    config::write_u32_release(0, STAGE_IDENTITY);

    // The cluster's public key: prefer the key committed by the ceremony
    // (obtained from the cluster), else the build-time copy the kernel
    // handed us in the launch manifest. The kernel pre-stages the signed
    // artifact into the object store; this agent only fetches, verifies, and
    // serves.
    let cluster_key = manifest_cluster_key(&ctx).unwrap_or_else(|| fail(STAGE_FAIL));
    if read_cluster_key(dns_conn).is_some_and(|replicated| replicated != cluster_key) {
        // Replicated state may distribute the bootstrap anchor, but it may
        // not replace it without an authenticated key-rotation protocol.
        fail(STAGE_FAIL);
    }

    loop {
        if let Some(entry) = query_deployment(dns_conn, deploy::NAME) {
            // The assignment is a cluster decision (committed by consensus).
            // If it names this node, the artifact must validate against the
            // cluster's public key before it is served.
            if entry.node_key == my_node_key
                && let Ok((artifact_cap, artifact_size)) = fetch_and_verify(
                    obj_conn,
                    entry.object_id,
                    &entry.artifact_digest,
                    &cluster_key,
                )
            {
                if spawn_artifact(artifact_cap, artifact_size, deploy::NAME) == 0 {
                    fail(STAGE_FAIL);
                }
                supervise(dns_conn, ns_connection, my_node_key, poll_ms, entry.generation);
            }
        }
        cq_wait_timeout(1, poll_ms, 0);
    }
}
