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
};
use catten_services::{
    clusterctl,
    disco,
    dns,
    ns,
    objstore,
    raft,
    wait_reply,
};
use charlotte_protocol_disco::{
    ROLE_LEADER,
    parse_cluster_answer,
};
use catten_syscall::*;

const DATA_VADDR: usize = 0x0000_0000_2000_0000;
const SIZE_VADDR: usize = 0x0000_0000_0070_0000;
const REPLY_SPINS: u64 = 50_000_000;
const STAGE_SERVING: u32 = 6;

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
    if size_cap == 0 || memory_map(size_cap, SIZE_VADDR, true) != 0 {
        return false;
    }
    unsafe {
        (SIZE_VADDR as *mut u64).write_unaligned(bytes.len() as u64);
    }
    memory_unmap(size_cap);
    let set_size =
        ipc_scalar_call_borrow_read(obj_conn, objstore::OP_SET_SIZE, object_id, size_cap);
    if set_size == 0 || catten_services::spin_reply(set_size).0 != objstore::ERR_OK {
        return false;
    }
    memory_close(size_cap);

    // The artifact is an ELF (tens of KiB): the data cap must span every
    // page the bytes occupy, not a single page.
    let data = memory_alloc(bytes.len().div_ceil(4096).max(1));
    if data == 0 || memory_map(data, DATA_VADDR, true) != 0 {
        return false;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), DATA_VADDR as *mut u8, bytes.len());
    }
    memory_unmap(data);
    let write = ipc_scalar_call_move(obj_conn, objstore::OP_WRITE, object_id, data);
    if write == 0 || catten_services::spin_reply(write).0 != objstore::ERR_OK {
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
    if memory_map(memory, DATA_VADDR, false) != 0 {
        memory_close(memory);
        return None;
    }
    let mut hasher = charlotte_launch::sha256::Sha256::new();
    for index in 0..len {
        hasher.update(&[unsafe { core::ptr::read_volatile((DATA_VADDR + index) as *const u8) }]);
    }
    memory_unmap(memory);
    memory_close(memory);
    Some(hasher.finalize())
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
    if memory_map(message.memory, DATA_VADDR, false) != 0 {
        memory_close(message.memory);
        return None;
    }
    let len = unsafe { core::ptr::read_volatile(DATA_VADDR as *const u64) } as usize;
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
        payload.push(unsafe { core::ptr::read_volatile((DATA_VADDR + 8 + index) as *const u8) });
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
    config::write::<u32>(0, STAGE_SERVING);

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
                                // The dns commits the deployment; its reply is
                                // deferred until the manifest entry has
                                // replicated.
                                let request = memory_alloc(1);
                                if request == 0 || memory_map(request, DATA_VADDR, true) != 0 {
                                    clusterctl::ERR_UPLOAD_FAILED
                                } else {
                                    unsafe {
                                        core::ptr::write_volatile(
                                            DATA_VADDR as *mut u64,
                                            object_id,
                                        );
                                        core::ptr::write_volatile(
                                            (DATA_VADDR + 8) as *mut u64,
                                            node_key,
                                        );
                                        core::ptr::copy_nonoverlapping(
                                            artifact_digest.as_ptr(),
                                            (DATA_VADDR + 16) as *mut u8,
                                            artifact_digest.len(),
                                        );
                                    }
                                    memory_unmap(request);
                                    let call = ipc_scalar_call_move(
                                        dns_conn,
                                        dns::OP_DEPLOY,
                                        message.arg0,
                                        request,
                                    );
                                    if call == 0 {
                                        catten_syscall::el0_log(0x4354_4c00, 0x1111);
                                        clusterctl::ERR_NOT_LEADER
                                    } else {
                                        let (raw_status, raw_result, _raw_cap) =
                                            catten_syscall::ipc_reply_wait(call);
                                        catten_syscall::el0_log(
                                            0x4354_4c00,
                                            0x2222
                                                | ((raw_status as u64) << 8)
                                                | (((raw_result as i64) << 32) as u64),
                                        );
                                        ipc_close(call);
                                        catten_syscall::el0_log(
                                            0x4354_4c00,
                                            0x1112 | ((raw_result as i64 as u64) << 8),
                                        );
                                        if raw_status == 0 {
                                            raw_result as i64
                                        } else {
                                            clusterctl::ERR_NOT_LEADER
                                        }
                                    }
                                }
                            }
                            None => clusterctl::ERR_TOO_LARGE,
                        }
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
                            if key_memory == 0 || memory_map(key_memory, DATA_VADDR, true) != 0 {
                                if message.reply != 0 {
                                    ipc_reply(message.reply, clusterctl::ERR_UPLOAD_FAILED);
                                }
                                continue;
                            }
                            unsafe {
                                core::ptr::copy_nonoverlapping(
                                    key.as_ptr(),
                                    DATA_VADDR as *mut u8,
                                    key.len(),
                                );
                            }
                            memory_unmap(key_memory);
                            let call =
                                ipc_scalar_call_move(dns_conn, dns::OP_SET_KEY, 0, key_memory);
                            if call == 0 {
                                clusterctl::ERR_NOT_LEADER
                            } else {
                                let (generation, _) = unsafe { wait_reply(call, REPLY_SPINS) };
                                ipc_close(call);
                                generation
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
                                    if memory_map(memory, DATA_VADDR, false) == 0 {
                                        let bytes = unsafe {
                                            core::slice::from_raw_parts(
                                                DATA_VADDR as *const u8,
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
    peers: &[([u8; 6], u8, alloc::vec::Vec<u8>, alloc::vec::Vec<u8>)],
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
    if payload == 0 || memory_map(payload, DATA_VADDR, true) != 0 {
        if payload != 0 {
            memory_close(payload);
        }
        return clusterctl::ERR_UPLOAD_FAILED;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(spec_buf.as_ptr(), DATA_VADDR as *mut u8, spec_len);
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
    if memory_map(message.memory, DATA_VADDR, false) != 0 {
        memory_close(message.memory);
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((DATA_VADDR + index) as *const u8) };
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
    if memory_map(message.memory, DATA_VADDR, false) != 0 {
        memory_close(message.memory);
        return None;
    }
    let node_key = unsafe { core::ptr::read_volatile(DATA_VADDR as *const u64) };
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
