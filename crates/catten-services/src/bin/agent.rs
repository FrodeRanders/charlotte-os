//! Cluster deploy agent: the node-side "picker-upper" of the server-class
//! cluster vision (manual Chapter 17).
//!
//! Each agent:
//!
//! 1. Uploads the demo artifact (a MAC-signed payload blob) to its node-local object store -- the
//!    sketch's stand-in for "software lives in the interchangeable cluster object store".
//! 2. Polls the replicated deployment manifest (the dns catalog) for assignments addressed to its
//!    node key.
//! 3. For each assignment: verifies the cluster signature over the deployment record, picks the
//!    payload up from the object store, verifies the payload signature, registers the deployed name
//!    in the distributed name service, and serves it.
//! 4. When the cluster re-assigns the artifact elsewhere (migration), the agent stops serving and
//!    exits; the dns endpoint-close watch unregisters its generation with fencing, so the new
//!    generation is never clobbered.
//!
//! The signatures are placeholder FNV-1a MACs keyed with the shared
//! deployment secret (real cryptography and the blank-start key ceremony are
//! future work). The node key and poll interval arrive in the launch
//! manifest, written by the kernel self-test.
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
const STAGE_UPLOADED: u32 = 4;
const STAGE_SERVING: u32 = 6;
const STAGE_RETIRED: u32 = 7;
const STAGE_FAIL: u32 = 0xdead;

/// The deployed artifact's name bytes ("greet", matching `deploy::NAME`).
const GREET_NAME: &[u8] = b"greet";

const DATA_VADDR: usize = 0x0000_0000_2000_0000;
const SIZE_VADDR: usize = 0x0000_0000_0070_0000;
const REPLY_SPINS: u64 = 50_000_000;

/// A replicated deployment record as decoded from `OP_DEPLOY_QUERY`.
struct DeploymentInfo {
    generation: u64,
    object_id: u64,
    node_key: u64,
    mac: u64,
}

fn fail(stage: u32) -> ! {
    config::write::<u32>(0, stage);
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

/// Upload `bytes` to the object store at `object_id` (create-at, set size,
/// move-write, flush). `ERR_EXISTS` is success: the artifact is already in
/// the store.
fn upload_artifact(obj_conn: u64, object_id: u64, bytes: &[u8]) -> Result<(), ()> {
    let create = ipc_scalar_call(obj_conn, objstore::OP_CREATE_AT, object_id);
    if create == 0 {
        return Err(());
    }
    let created = catten_services::spin_reply(create).0;
    if created != objstore::ERR_OK && created != objstore::ERR_EXISTS {
        return Err(());
    }
    if created == objstore::ERR_EXISTS {
        return Ok(());
    }

    let size_cap = memory_alloc(1);
    if size_cap == 0 || memory_map(size_cap, SIZE_VADDR, true) != 0 {
        return Err(());
    }
    unsafe {
        (SIZE_VADDR as *mut u64).write_unaligned(bytes.len() as u64);
    }
    memory_unmap(size_cap);
    let set_size =
        ipc_scalar_call_borrow_read(obj_conn, objstore::OP_SET_SIZE, object_id, size_cap);
    if set_size == 0 || catten_services::spin_reply(set_size).0 != objstore::ERR_OK {
        return Err(());
    }
    memory_close(size_cap);

    let data = memory_alloc(1);
    if data == 0 || memory_map(data, DATA_VADDR, true) != 0 {
        return Err(());
    }
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), DATA_VADDR as *mut u8, bytes.len());
    }
    memory_unmap(data);
    let write = ipc_scalar_call_move(obj_conn, objstore::OP_WRITE, object_id, data);
    if write == 0 || catten_services::spin_reply(write).0 != objstore::ERR_OK {
        return Err(());
    }
    let flush = ipc_scalar_call(obj_conn, objstore::OP_FLUSH, 0);
    if flush == 0 || catten_services::spin_reply(flush).0 != objstore::ERR_OK {
        return Err(());
    }
    Ok(())
}

/// Blocking query of the replicated deployment manifest for `packed_name`
/// (packed LE). Used by the pre-serve polling loop, where nothing invokes the
/// agent yet, so blocking on the dns reply cannot deadlock.
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

/// Read the artifact at `object_id` from the object store and verify both the
/// payload MAC and the payload bytes.
fn fetch_and_verify(obj_conn: u64, object_id: u64) -> Result<(), ()> {
    let read = ipc_scalar_call(obj_conn, objstore::OP_READ, object_id);
    if read == 0 {
        return Err(());
    }
    let (status, size, returned_connection, returned_memory) = ipc_reply_wait_with_memory(read);
    ipc_close(read);
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    let expected = 8 + deploy::GREET_PAYLOAD.len();
    if status != 0 || size as usize != expected || returned_memory == 0 {
        if returned_memory != 0 {
            memory_close(returned_memory);
        }
        return Err(());
    }
    if memory_map(returned_memory, DATA_VADDR, false) != 0 {
        memory_close(returned_memory);
        return Err(());
    }
    let stored_mac = unsafe { core::ptr::read_volatile(DATA_VADDR as *const u64) };
    let mut payload = [0u8; 32];
    for (index, byte) in payload.iter_mut().enumerate().take(deploy::GREET_PAYLOAD.len()) {
        *byte = unsafe { core::ptr::read_volatile((DATA_VADDR + 8 + index) as *const u8) };
    }
    memory_unmap(returned_memory);
    memory_close(returned_memory);
    if deploy::payload_mac(&payload[..deploy::GREET_PAYLOAD.len()]) != stored_mac
        || payload[..deploy::GREET_PAYLOAD.len()] != *deploy::GREET_PAYLOAD
    {
        return Err(());
    }
    Ok(())
}

/// Register the deployed name locally and in the distributed catalog, bind the
/// endpoint, and serve `OP_GET` until the cluster re-assigns the artifact.
///
/// The serve loop never blocks on the dns: the publish reply and the
/// retirement query are polled, so a caller's `OP_GET` is always answered
/// even while the catalog entry is still replicating. (A blocking query here
/// would deadlock: the dns waits for our reply inside `invoke_local` while we
/// wait for its query reply.)
fn serve(dns_conn: u64, ns_connection: u64, my_node_key: u64, poll_ms: u64, generation: u64) -> ! {
    let endpoint = ipc_endpoint_create(deploy::INTERFACE, deploy::VERSION, 8);
    if endpoint == 0 {
        fail(STAGE_FAIL);
    }
    let register = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        deploy::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    let local = if register == 0 {
        None
    } else {
        let (generation, _) = unsafe { wait_reply(register, REPLY_SPINS) };
        (generation >= 1).then_some(generation)
    };
    if local.is_none() {
        fail(STAGE_FAIL);
    }
    if ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        fail(STAGE_FAIL);
    }

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
        // Drain and reply to pending calls first, unconditionally.
        loop {
            let message = ipc_recv(endpoint);
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() }
            }
            match message.opcode {
                deploy::OP_GET => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, deploy::GREET_VALUE as i64);
                    }
                }
                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
            }
        }

        // Publish progress: has the catalog registration committed?
        if !published {
            let (status, result, _returned_connection) = ipc_reply_poll(publish);
            if status == 0 {
                ipc_close(publish);
                if result >= 1 {
                    published = true;
                    config::write::<u64>(8, generation);
                    config::write::<u32>(0, STAGE_SERVING);
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
                let still_mine = entry.is_some_and(|entry| {
                    entry.node_key == my_node_key
                        && entry.mac == dns::deploy_mac(GREET_NAME, entry.object_id, entry.node_key)
                });
                if !still_mine {
                    config::write::<u32>(0, STAGE_RETIRED);
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

/// Decode a `OP_DEPLOY_QUERY` reply page (32 bytes, `[generation][object_id]
/// [node_key][mac]`), mapped at `DATA_VADDR`. Returns `None` when the memory
/// cap is absent (the query errored or found nothing).
fn decode_deployment(memory: u64) -> Option<DeploymentInfo> {
    if memory == 0 {
        return None;
    }
    if memory_map(memory, DATA_VADDR, false) != 0 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((DATA_VADDR + index) as *const u8) };
    }
    memory_unmap(memory);
    Some(DeploymentInfo {
        generation: u64::from_le_bytes(bytes[0..8].try_into().ok()?),
        object_id: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        node_key: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
        mac: u64::from_le_bytes(bytes[24..32].try_into().ok()?),
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
    config::write::<u32>(0, STAGE_IDENTITY);

    // "Software lives in the object store": upload the signed artifact to the
    // (node-local, for now) store. `GREET_NAME` is the deployed name the
    // cluster will assign.
    if upload_artifact(obj_conn, dns::artifact_object_id(GREET_NAME), &deploy::artifact_bytes())
        .is_err()
    {
        fail(STAGE_FAIL);
    }
    config::write::<u32>(0, STAGE_UPLOADED);

    loop {
        if let Some(entry) = query_deployment(dns_conn, deploy::NAME) {
            // The assignment is a cluster decision: verify its signature
            // against the shared deployment secret before acting.
            let mac_ok = entry.mac == dns::deploy_mac(GREET_NAME, entry.object_id, entry.node_key);
            if entry.node_key == my_node_key
                && mac_ok
                && fetch_and_verify(obj_conn, entry.object_id).is_ok()
            {
                serve(dns_conn, ns_connection, my_node_key, poll_ms, entry.generation);
            }
        }
        cq_wait_timeout(1, poll_ms, 0);
    }
}
