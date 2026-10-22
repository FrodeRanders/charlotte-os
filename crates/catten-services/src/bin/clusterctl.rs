//! Cluster administration service: the programmatic "outside" interface to a
//! cluster (manual Chapter 19).
//!
//! `clusterctl` wraps the raw dns manifest ops and the object store behind
//! admin-level operations:
//!
//! 1. `OP_UPLOAD` verifies an offline CLS2 signature and logical identity, then stores the
//!    immutable ELF under its derived cluster-wide id.
//! 2. `OP_NOTIFY` verifies a signed central-store descriptor and submits it through the node-local
//!    DNS, which relays follower admissions to the current Raft leader.
//! 3. `OP_STATUS` and `OP_ROLLOUT` expose committed and generation-safe ready state.
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
    ShutdownRequest,
    config,
    owned::{
        Connection,
        Endpoint,
        OwnedMemory,
        ReplyToken,
    },
};
use catten_services::{
    clusterctl,
    disco,
    dns,
    ns,
    objstore,
    publication,
    raft,
    wait_for_registered_name_or_shutdown_owned,
    wait_reply,
};
use catten_syscall::*;
use charlotte_launch::clusterctl_status as status;
use charlotte_protocol_disco::{
    ROLE_LEADER,
    parse_cluster_answer,
};

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
/// present and is overwritten with the new content.
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

    // The ELF occupies tens of KiB. The data cap must span all of its pages.
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

fn serve(ctx: &Context) -> Result<ShutdownRequest, u32> {
    let trust = match ctx.manifest_value(charlotte_launch::ADMISSION_TRUST_MANIFEST_KEY) {
        Some(ManifestValue::Bytes(bytes)) => {
            charlotte_launch::trust::AdmissionTrust::decode(bytes).ok_or(0xdea5u32)?
        }
        _ => return Err(0xdea5),
    };
    let ns_connection = ctx.bootstrap_connection().ok_or(0xdea0u32)?;
    let (_, obj_connection) =
        match wait_for_registered_name_or_shutdown_owned(ctx, ns_connection, objstore::NAME) {
            Ok(found) => found,
            Err(request) => return Ok(request),
        };
    let (_, dns_connection) =
        match wait_for_registered_name_or_shutdown_owned(ctx, ns_connection, dns::NAME) {
            Ok(found) => found,
            Err(request) => return Ok(request),
        };
    let endpoint =
        Endpoint::create(clusterctl::INTERFACE, clusterctl::VERSION, 8).map_err(|_| 0xdea2u32)?;
    publication::register(ns_connection, clusterctl::NAME, &endpoint).map_err(|error| {
        catten_rt::logln!("[clusterctl] registration rejected: {error:?}");
        0xdea3u32
    })?;
    endpoint.bind_completion_queue(0).map_err(|_| 0xdea4u32)?;
    config::write::<u32>(status::STAGE, STAGE_SERVING);

    let mut relays = catten_services::clusterctl_relay::Relays::new(
        dns_connection.as_ref(),
        &trust.deployment_key,
        0,
    );
    let cq = ctx.completion_queue_layout();

    // The request handlers below are still the protocol adapter's legacy raw
    // reactor. These integers borrow the typed owners for this serving scope;
    // they are never closed, transferred, or adopted.
    let ns_connection = ns_connection.as_raw();
    let obj_conn = obj_connection.as_raw();
    let dns_conn = dns_connection.as_raw();

    loop {
        for _ in 0..cq.entries {
            if unsafe { cq_read(cq.base, cq.entries) }.is_none() {
                break;
            }
        }
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return Ok(request);
        }
        let mut progressed = false;
        for _ in 0..catten_services::clusterctl_relay::MAX_RELAYS {
            let message = ipc_recv(endpoint.as_raw());
            if matches!(message.status, ipc_status::NO_MESSAGE | ipc_status::PENDING) {
                // A cancellation claim can fence the queue front. Its existing
                // completion re-signals endpoint/CQ readiness for later work.
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() }
            }
            progressed = true;
            if catten_services::clusterctl_relay::handles(message.opcode) {
                // Receive ABI boundary: move the complete incoming transaction
                // into its existing controller relay, exactly once.
                match unsafe { catten_rt::owned::IncomingMessage::from_raw(message) } {
                    Ok(Some(incoming)) => relays.start(incoming),
                    _ => catten_rt::domain_abort(),
                }
                continue;
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
                                    &trust.artifact_key,
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
                clusterctl::OP_KEYCEREMONY => {
                    let result = match read_key(&message) {
                        Some(key) if key == trust.deployment_key => {
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
                                unsafe { wait_reply(call) }.0
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
                                        let size = usize::try_from(size)
                                            .ok()
                                            .filter(|size| *size <= memory_size(memory));
                                        if let Some(size) = size {
                                            let bytes = unsafe {
                                                core::slice::from_raw_parts(
                                                    data_vaddr_4_vaddr as *const u8,
                                                    size,
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
                    // Raw receive ABI boundary; unsupported requests still
                    // consume their uniquely moved memory object exactly once.
                    let _request = unsafe { OwnedMemory::from_raw(message.memory) }.ok();
                    let _connection = unsafe { Connection::from_raw(message.connection) }.ok();
                    if let Ok(token) = unsafe { ReplyToken::from_raw(message.reply) } {
                        let _ = token.reply(-1);
                    }
                }
            }
        }
        progressed |= relays.advance();
        if !progressed {
            catten_services::reactor::wait(relays.remaining_ms(), 0);
        }
    }
}

fn main(ctx: Context) -> ! {
    match serve(&ctx) {
        Ok(request) => request.complete(),
        Err(stage) => fail(stage),
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

/// Unpack a packed-le short name (identical to the dns's helper).
fn packed_name(packed: u64) -> alloc::vec::Vec<u8> {
    let bytes = packed.to_le_bytes();
    let len = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |index| index + 1);
    bytes[..len].to_vec()
}
