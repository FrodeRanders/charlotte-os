//! Cluster administration service: the programmatic "outside" interface to a
//! cluster (manual Chapter 19).
//!
//! `clusterctl` wraps the raw dns manifest ops and the object store behind
//! admin-level operations:
//!
//! 1. `OP_UPLOAD` verifies an offline CLS2 signature and logical identity, then stores the
//!    immutable ELF under its derived cluster-wide id.
//! 2. `OP_DEPLOY` assigns the artifact to a node by submitting a deployment record through the
//!    local dns (the manifest is replicated cluster state).
//! 3. `OP_STATUS` reports the committed deployment record.
//!
//! Artifact names are bare cluster-global names; the node dimension appears
//! only in the deployment record. The private signing key never enters this
//! service; its build-time public anchor constrains the key ceremony.
#![no_std]
#![no_main]

extern crate alloc;

catten_rt::entry!(main);

use catten_rt::{
    Context,
    ManifestValue,
    config,
    owned::{
        ConnectionRef,
        OwnedMemory,
    },
};
use catten_services::{
    clusterctl,
    disco,
    dns,
    name_catalog,
    ns,
    objstore,
    raft,
    wait_reply,
};
use catten_syscall::*;
use charlotte_launch::clusterctl_status as status;
use charlotte_protocol_disco::{
    ROLE_LEADER,
    parse_cluster_answer,
};

const REPLY_SPINS: u64 = 50_000_000;
const STAGE_SERVING: u32 = 6;

fn fail(stage: u32) -> ! {
    config::write::<u32>(status::STAGE, stage);
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

/// Write `bytes` to the object store at `object_id` (create-at, set size,
/// move-write, flush). `ERR_EXISTS` is success: the artifact is already
/// present and is simply overwritten with the new content.
fn store_artifact(obj_conn: u64, object_id: u64, bytes: &[u8]) -> bool {
    let create = ipc_scalar_call(obj_conn, objstore::OP_CREATE_AT, object_id);
    if create == 0 {
        return false;
    }
    let created = catten_services::spin_reply(create).0;
    if created != objstore::ERR_OK && created != objstore::ERR_EXISTS {
        return false;
    }

    let size_cap = memory_alloc(1);
    let (size_vaddr_map_status, size_vaddr_vaddr) = memory_map_any(size_cap, true);
    if size_cap == 0 || size_vaddr_map_status != 0 {
        if size_cap != 0 {
            memory_close(size_cap);
        }
        return false;
    }
    unsafe {
        (size_vaddr_vaddr as *mut u64).write_unaligned(bytes.len() as u64);
    }
    memory_unmap(size_cap);
    let set_size =
        ipc_scalar_call_borrow_read(obj_conn, objstore::OP_SET_SIZE, object_id, size_cap);
    let resized = set_size != 0 && catten_services::spin_reply(set_size).0 == objstore::ERR_OK;
    memory_close(size_cap);
    if !resized {
        return false;
    }

    // The artifact is an ELF (tens of KiB): the data cap must span every
    // page the bytes occupy, not a single page.
    let data = memory_alloc(bytes.len().div_ceil(4096).max(1));
    let (data_vaddr_9_map_status, data_vaddr_9_vaddr) = memory_map_any(data, true);
    if data == 0 || data_vaddr_9_map_status != 0 {
        if data != 0 {
            memory_close(data);
        }
        return false;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), data_vaddr_9_vaddr as *mut u8, bytes.len());
    }
    memory_unmap(data);
    let write = ipc_scalar_call_move(obj_conn, objstore::OP_WRITE, object_id, data);
    if write == 0 {
        memory_close(data);
        return false;
    }
    if catten_services::spin_reply(write).0 != objstore::ERR_OK {
        return false;
    }
    let flush = ipc_scalar_call(obj_conn, objstore::OP_FLUSH, 0);
    if flush == 0 || catten_services::spin_reply(flush).0 != objstore::ERR_OK {
        return false;
    }
    true
}

/// Hash the exact object-store bytes that a deployment generation selects.
/// This prevents a later overwrite at the same name-derived object id from
/// silently changing code under an already committed assignment.
fn stored_artifact_digest(obj_conn: u64, object_id: u64) -> Option<[u8; 32]> {
    let read = ipc_scalar_call(obj_conn, objstore::OP_READ, object_id);
    if read == 0 {
        return None;
    }
    let (status, size, returned_connection, memory) = ipc_reply_wait_with_memory(read);
    ipc_close(read);
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    let len = usize::try_from(size).ok()?;
    if status != 0 || memory == 0 || len == 0 || len > memory_size(memory) {
        if memory != 0 {
            memory_close(memory);
        }
        return None;
    }
    let (data_vaddr_8_map_status, data_vaddr_8_vaddr) = memory_map_any(memory, false);
    if data_vaddr_8_map_status != 0 {
        memory_close(memory);
        return None;
    }
    let mut hasher = charlotte_launch::sha256::Sha256::new();
    for index in 0..len {
        hasher.update(&[unsafe {
            core::ptr::read_volatile((data_vaddr_8_vaddr + index) as *const u8)
        }]);
    }
    memory_unmap(memory);
    memory_close(memory);
    Some(hasher.finalize())
}

fn submit_deployment(
    dns_conn: u64,
    packed_name: u64,
    object_id: u64,
    node_key: u64,
    artifact_digest: &[u8; 32],
    descriptor: &[u8],
) -> i64 {
    let request_len = 56usize.saturating_add(descriptor.len());
    if descriptor.len() > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN || request_len > 4096 {
        return clusterctl::ERR_UPLOAD_FAILED;
    }
    let request = match OwnedMemory::allocate(1) {
        Ok(request) => request,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    let mut mapping = match request.map_writable() {
        Ok(mapping) => mapping,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    let bytes = mapping.as_mut_slice();
    bytes[0..8].copy_from_slice(&object_id.to_le_bytes());
    bytes[8..16].copy_from_slice(&node_key.to_le_bytes());
    bytes[16..48].copy_from_slice(artifact_digest);
    if !descriptor.is_empty() {
        bytes[48..52].copy_from_slice(&dns::DEPLOY_DESCRIPTOR_MAGIC.to_le_bytes());
        bytes[52..56].copy_from_slice(&(descriptor.len() as u32).to_le_bytes());
        bytes[56..request_len].copy_from_slice(descriptor);
    }
    let request = match mapping.unmap() {
        Ok(request) => request,
        Err(_) => return clusterctl::ERR_UPLOAD_FAILED,
    };
    // `dns_conn` is owned by this legacy service loop; this short borrow keeps
    // the newly transferred request memory under the typed ownership API.
    let dns = match unsafe { ConnectionRef::from_raw(dns_conn) } {
        Ok(dns) => dns,
        Err(_) => return clusterctl::ERR_NOT_LEADER,
    };
    match dns.call_move(dns::OP_DEPLOY, packed_name, request) {
        Ok(call) => match call.wait() {
            Ok(reply) => reply.result,
            Err(_) => clusterctl::ERR_NOT_LEADER,
        },
        Err((_request, _error)) => clusterctl::ERR_NOT_LEADER,
    }
}

fn current_deployment(dns_conn: u64, packed_name: u64) -> Option<name_catalog::DeploymentEntry> {
    let dns = unsafe { ConnectionRef::from_raw(dns_conn) }.ok()?;
    let reply = dns.call(dns::OP_DEPLOY_QUERY, packed_name).ok()?.wait().ok()?;
    if reply.result < 56 {
        return None;
    }
    let len = usize::try_from(reply.result).ok()?;
    let memory = reply.memory?;
    if len > memory.len() {
        return None;
    }
    let mapping = memory.map_read_only().ok()?;
    name_catalog::decode_deployment_result(mapping.as_slice().get(..len)?)
}

/// The raw payload attached to an `OP_UPLOAD` call, copied out of the moved
/// memory object. The memory layout is `[payload_len:u64 LE][payload]`.
fn read_payload(message: &catten_syscall::IpcMessage) -> Option<alloc::vec::Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    let capacity = memory_size(message.memory);
    if capacity < 8 {
        memory_close(message.memory);
        return None;
    }
    let (data_vaddr_7_map_status, data_vaddr_7_vaddr) = memory_map_any(message.memory, false);
    if data_vaddr_7_map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let len = unsafe { core::ptr::read_volatile(data_vaddr_7_vaddr as *const u64) } as usize;
    // The payload region is the complete multi-page memory object. Apply the
    // same admission bound as the kernel launch gate so ingress and execution
    // cannot disagree about an otherwise valid artifact.
    if len == 0 || len > charlotte_launch::MAX_ARTIFACT_ELF_SIZE || len > capacity - 8 {
        memory_unmap(message.memory);
        memory_close(message.memory);
        return None;
    }
    let mut payload = alloc::vec::Vec::with_capacity(len);
    for index in 0..len {
        payload.push(unsafe {
            core::ptr::read_volatile((data_vaddr_7_vaddr + 8 + index) as *const u8)
        });
    }
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(payload)
}

fn main(ctx: Context) -> ! {
    let trusted_key = match ctx.manifest_value(charlotte_launch::CLUSTER_KEY_MANIFEST_KEY) {
        Some(ManifestValue::Bytes(bytes)) => match <[u8; 32]>::try_from(bytes) {
            Ok(key) => key,
            Err(_) => fail(0xdea5),
        },
        _ => fail(0xdea5),
    };
    let ns_connection = ctx.bootstrap_cap().unwrap_or_else(|| fail(0xdea0));
    let obj_conn = lookup(ns_connection, objstore::NAME);
    let dns_conn = lookup(ns_connection, dns::NAME);
    if obj_conn == 0 || dns_conn == 0 {
        fail(0xdea1);
    }

    let endpoint = ipc_endpoint_create(clusterctl::INTERFACE, clusterctl::VERSION, 8);
    if endpoint == 0 {
        fail(0xdea2);
    }
    let register = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        clusterctl::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 || unsafe { wait_reply(register, REPLY_SPINS) }.0 < 1 {
        fail(0xdea3);
    }
    if ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        fail(0xdea4);
    }
    config::write::<u32>(status::STAGE, STAGE_SERVING);

    loop {
        cq_wait(1, 0);
        loop {
            let message = ipc_recv(endpoint);
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() }
            }
            match message.opcode {
                clusterctl::OP_UPLOAD => {
                    let name = packed_name(message.arg0);
                    let result = if name.is_empty() {
                        clusterctl::ERR_TOO_LARGE
                    } else {
                        match read_payload(&message) {
                            Some(artifact) => {
                                let object_id = dns::artifact_object_id(&name);
                                // Ingress is a trust boundary: the signature
                                // must bless this exact logical name before
                                // any object-store slot can be modified.
                                let trusted = charlotte_launch::signature_note::verify_elf_for_name(
                                    &artifact,
                                    &trusted_key,
                                    &name,
                                )
                                    == charlotte_launch::signature_note::VerifyOutcome::Valid;
                                if !trusted {
                                    clusterctl::ERR_UNTRUSTED_ARTIFACT
                                } else if store_artifact(obj_conn, object_id, &artifact) {
                                    object_id as i64
                                } else {
                                    clusterctl::ERR_UPLOAD_FAILED
                                }
                            }
                            None => clusterctl::ERR_TOO_LARGE,
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                clusterctl::OP_DEPLOY => {
                    let name = packed_name(message.arg0);
                    let result = if name.is_empty() {
                        clusterctl::ERR_TOO_LARGE
                    } else {
                        // The node key arrives in the attached memory object;
                        // the object id is derived from the artifact name.
                        match read_node_key(&message) {
                            Some(node_key) => {
                                let object_id = dns::artifact_object_id(&name);
                                let Some(artifact_digest) =
                                    stored_artifact_digest(obj_conn, object_id)
                                else {
                                    if message.reply != 0 {
                                        ipc_reply(message.reply, clusterctl::ERR_NOT_FOUND);
                                    }
                                    continue;
                                };
                                submit_deployment(
                                    dns_conn,
                                    message.arg0,
                                    object_id,
                                    node_key,
                                    &artifact_digest,
                                    &[],
                                )
                            }
                            None => clusterctl::ERR_TOO_LARGE,
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                clusterctl::OP_NOTIFY => {
                    let name = packed_name(message.arg0);
                    let result = match read_payload(&message) {
                        Some(descriptor_bytes)
                            if charlotte_launch::deployment::verify(
                                &descriptor_bytes,
                                &trusted_key,
                            ) == charlotte_launch::deployment::VerifyOutcome::Valid =>
                        {
                            match charlotte_launch::deployment::decode(&descriptor_bytes) {
                                Some(descriptor)
                                    if descriptor.artifact_name == name
                                        && descriptor.node_key != 0 =>
                                {
                                    match current_deployment(dns_conn, message.arg0)
                                        .filter(|current| !current.descriptor.is_empty())
                                    {
                                        Some(current)
                                            if charlotte_launch::deployment::decode(
                                                &current.descriptor,
                                            )
                                            .is_some_and(|previous| {
                                                descriptor.sequence < previous.sequence
                                            }) =>
                                        {
                                            clusterctl::ERR_STALE_DESCRIPTOR
                                        }
                                        Some(current)
                                            if charlotte_launch::deployment::decode(
                                                &current.descriptor,
                                            )
                                            .is_some_and(|previous| {
                                                descriptor.sequence == previous.sequence
                                            }) =>
                                        {
                                            if current.descriptor == descriptor_bytes {
                                                current.generation as i64
                                            } else {
                                                clusterctl::ERR_CONFLICTING_DESCRIPTOR
                                            }
                                        }
                                        _ => submit_deployment(
                                            dns_conn,
                                            message.arg0,
                                            dns::artifact_object_id(&name),
                                            descriptor.node_key,
                                            &descriptor.artifact_digest,
                                            &descriptor_bytes,
                                        ),
                                    }
                                }
                                _ => clusterctl::ERR_UNTRUSTED_DESCRIPTOR,
                            }
                        }
                        Some(_) => clusterctl::ERR_UNTRUSTED_DESCRIPTOR,
                        None => clusterctl::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                clusterctl::OP_STATUS => {
                    let name = packed_name(message.arg0);
                    if name.is_empty() {
                        if message.reply != 0 {
                            ipc_reply(message.reply, clusterctl::ERR_TOO_LARGE);
                        }
                        continue;
                    }
                    // Pass the deployment record through from the dns.
                    let call = ipc_scalar_call(dns_conn, dns::OP_DEPLOY_QUERY, message.arg0);
                    if call == 0 {
                        if message.reply != 0 {
                            ipc_reply(message.reply, clusterctl::ERR_NOT_FOUND);
                        }
                        continue;
                    }
                    let (status, size, _returned_connection, memory) =
                        ipc_reply_wait_with_memory(call);
                    ipc_close(call);
                    if memory == 0 || (status as i64) < 0 {
                        if memory != 0 {
                            memory_close(memory);
                        }
                        if message.reply != 0 {
                            ipc_reply(message.reply, clusterctl::ERR_NOT_FOUND);
                        }
                        continue;
                    }
                    if message.reply != 0 {
                        ipc_reply_move(message.reply, memory, size as i64);
                    } else {
                        memory_close(memory);
                    }
                }
                clusterctl::OP_KEYCEREMONY => {
                    let result = match read_key(&message) {
                        Some(key) if key == trusted_key => {
                            // Forward the key to the dns, which commits it to
                            // the replicated state (leader-only; the reply is
                            // deferred until the ceremony record commits).
                            let key_memory = memory_alloc(1);
                            let (data_vaddr_5_map_status, data_vaddr_5_vaddr) =
                                memory_map_any(key_memory, true);
                            if key_memory == 0 || data_vaddr_5_map_status != 0 {
                                if message.reply != 0 {
                                    ipc_reply(message.reply, clusterctl::ERR_UPLOAD_FAILED);
                                }
                                continue;
                            }
                            unsafe {
                                core::ptr::copy_nonoverlapping(
                                    key.as_ptr(),
                                    data_vaddr_5_vaddr as *mut u8,
                                    key.len(),
                                );
                            }
                            memory_unmap(key_memory);
                            let call =
                                ipc_scalar_call_move(dns_conn, dns::OP_SET_KEY, 0, key_memory);
                            if call == 0 {
                                clusterctl::ERR_NOT_LEADER
                            } else {
                                unsafe { wait_reply(call, REPLY_SPINS) }.0
                            }
                        }
                        Some(_) => clusterctl::ERR_UNTRUSTED_KEY,
                        None => clusterctl::ERR_TOO_LARGE,
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                clusterctl::OP_KEY => {
                    // Pass the replicated cluster key through from the dns.
                    let call = ipc_scalar_call(dns_conn, dns::OP_KEY, 0);
                    if call == 0 {
                        if message.reply != 0 {
                            ipc_reply(message.reply, clusterctl::ERR_NOT_FOUND);
                        }
                        continue;
                    }
                    let (status, size, _returned_connection, memory) =
                        ipc_reply_wait_with_memory(call);
                    ipc_close(call);
                    if memory == 0 || (status as i64) < 0 {
                        if memory != 0 {
                            memory_close(memory);
                        }
                        if message.reply != 0 {
                            ipc_reply(message.reply, clusterctl::ERR_NOT_FOUND);
                        }
                        continue;
                    }
                    if message.reply != 0 {
                        ipc_reply_move(message.reply, memory, size as i64);
                    } else {
                        memory_close(memory);
                    }
                }
                clusterctl::OP_JOIN => {
                    let result = {
                        let disco_conn = lookup(ns_connection, disco::NAME);
                        if disco_conn == 0 {
                            clusterctl::ERR_NO_CLUSTER
                        } else {
                            let status_call =
                                ipc_scalar_call(disco_conn, disco::OP_CLUSTER_STATUS, 0);
                            if status_call == 0 {
                                clusterctl::ERR_NO_CLUSTER
                            } else {
                                let (status, size, _returned_connection, memory) =
                                    ipc_reply_wait_with_memory(status_call);
                                ipc_close(status_call);
                                let mut outcome = clusterctl::ERR_NO_CLUSTER;
                                if memory != 0 && status == 0 {
                                    let (data_vaddr_4_map_status, data_vaddr_4_vaddr) =
                                        memory_map_any(memory, false);
                                    if data_vaddr_4_map_status == 0 {
                                        let bytes = unsafe {
                                            core::slice::from_raw_parts(
                                                data_vaddr_4_vaddr as *const u8,
                                                size as usize,
                                            )
                                        };
                                        if let Some((
                                            _self_role,
                                            self_raft_id,
                                            self_leader_id,
                                            peers,
                                        )) = parse_cluster_answer(bytes)
                                        {
                                            outcome = run_join(
                                                ns_connection,
                                                self_raft_id,
                                                self_leader_id,
                                                &peers,
                                            );
                                        }
                                        memory_unmap(memory);
                                    }
                                    memory_close(memory);
                                } else if memory != 0 {
                                    memory_close(memory);
                                }
                                outcome
                            }
                        }
                    };
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
            }
        }
    }
}

/// Drive a cluster join: pick an admission target from the discovery
/// answer and ask its raft service to admit this node. Prefers a peer that
/// reports leader; otherwise redirects through the first peer's (or this
/// node's) leader hint; otherwise honestly reports that no cluster was
/// found on the segment.
fn run_join(
    ns_connection: u64,
    self_raft_id: &[u8],
    self_leader_id: &[u8],
    peers: &[charlotte_protocol_disco::PeerEntry],
) -> i64 {
    if self_raft_id.is_empty() {
        return clusterctl::ERR_NO_CLUSTER;
    }
    let mut target: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    for (_, role, raft_id, _) in peers {
        if *role == ROLE_LEADER && !raft_id.is_empty() && target.is_empty() {
            target = raft_id.clone();
        }
    }
    for (_, _, _, leader_id) in peers {
        if !leader_id.is_empty() && target.is_empty() {
            target = leader_id.clone();
        }
    }
    if target.is_empty() && !self_leader_id.is_empty() {
        target = self_leader_id.to_vec();
    }
    if target.is_empty() {
        return clusterctl::ERR_NO_CLUSTER;
    }

    let leader_raft = lookup(ns_connection, catten_services::raft_name(&target));
    if leader_raft == 0 {
        return clusterctl::ERR_NO_CLUSTER;
    }

    let self_service_name = catten_services::raft_name(self_raft_id);

    let mut spec_buf = [0u8; 96];
    let Some(spec_len) =
        raft::encode_peer_spec(&mut spec_buf, self_raft_id, self_service_name, false)
    else {
        return clusterctl::ERR_NO_CLUSTER;
    };
    let payload = memory_alloc(1);
    let (data_vaddr_3_map_status, data_vaddr_3_vaddr) = memory_map_any(payload, true);
    if payload == 0 || data_vaddr_3_map_status != 0 {
        if payload != 0 {
            memory_close(payload);
        }
        return clusterctl::ERR_UPLOAD_FAILED;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(spec_buf.as_ptr(), data_vaddr_3_vaddr as *mut u8, spec_len);
    }
    memory_unmap(payload);
    let call = ipc_scalar_call_move(leader_raft, raft::OP_ADD_SERVER, spec_len as u64, payload);
    if call == 0 {
        clusterctl::ERR_NOT_LEADER
    } else {
        let (status, result, _returned) = ipc_reply_wait(call);
        ipc_close(call);
        if status == 0 {
            result as i64
        } else {
            clusterctl::ERR_NOT_LEADER
        }
    }
}

/// The 32 cluster-key bytes attached to an `OP_KEYCEREMONY` call.
fn read_key(message: &catten_syscall::IpcMessage) -> Option<[u8; 32]> {
    if message.memory == 0 {
        return None;
    }
    if memory_size(message.memory) < 32 {
        memory_close(message.memory);
        return None;
    }
    let (data_vaddr_2_map_status, data_vaddr_2_vaddr) = memory_map_any(message.memory, false);
    if data_vaddr_2_map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((data_vaddr_2_vaddr + index) as *const u8) };
    }
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(key)
}

/// The node key attached to an `OP_DEPLOY` call: `[node_key:u64 LE]`.
fn read_node_key(message: &catten_syscall::IpcMessage) -> Option<u64> {
    if message.memory == 0 {
        return None;
    }
    if memory_size(message.memory) < 8 {
        memory_close(message.memory);
        return None;
    }
    let (data_vaddr_map_status, data_vaddr_vaddr) = memory_map_any(message.memory, false);
    if data_vaddr_map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let node_key = unsafe { core::ptr::read_volatile(data_vaddr_vaddr as *const u64) };
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(node_key)
}

/// Unpack a packed-le short name (identical to the dns's helper).
fn packed_name(packed: u64) -> alloc::vec::Vec<u8> {
    let bytes = packed.to_le_bytes();
    let len = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |index| index + 1);
    bytes[..len].to_vec()
}
