//! Cluster administration service: the programmatic "outside" interface to a
//! cluster (manual Chapter 17).
//!
//! `clusterctl` wraps the raw dns manifest ops and the object store behind
//! admin-level operations:
//!
//! 1. `OP_UPLOAD` signs an uploaded payload with the cluster secret and stores it in the
//!    (node-local) object store under the artifact's derived cluster-wide id
//!    (`dns::artifact_object_id`).
//! 2. `OP_DEPLOY` assigns the artifact to a node by submitting a deployment record through the
//!    local dns (the manifest is replicated cluster state).
//! 3. `OP_STATUS` reports the committed deployment record.
//!
//! Artifact names are bare cluster-global names; the node dimension appears
//! only in the deployment record. The key ceremony is a placeholder: the
//! cluster secret remains a build-time constant (`dns::DEPLOY_SECRET`).
#![no_std]
#![no_main]

extern crate alloc;

catten_rt::entry!(main);

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    clusterctl,
    dns,
    ns,
    objstore,
    wait_reply,
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

    let data = memory_alloc(1);
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

/// The raw payload attached to an `OP_UPLOAD` call, copied out of the moved
/// memory object. The memory layout is `[payload_len:u64 LE][payload]`.
fn read_payload(message: &catten_syscall::IpcMessage) -> Option<alloc::vec::Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    if memory_map(message.memory, DATA_VADDR, false) != 0 {
        return None;
    }
    let len = unsafe { core::ptr::read_volatile(DATA_VADDR as *const u64) } as usize;
    if len == 0 || len > 4088 {
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
                            Some(payload) => {
                                let object_id = dns::artifact_object_id(&name);
                                let blob = catten_services::deploy::sign_payload(&payload);
                                if store_artifact(obj_conn, object_id, &blob) {
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
                                    }
                                    memory_unmap(request);
                                    let call = ipc_scalar_call_move(
                                        dns_conn,
                                        dns::OP_DEPLOY,
                                        message.arg0,
                                        request,
                                    );
                                    if call == 0 {
                                        clusterctl::ERR_NOT_LEADER
                                    } else {
                                        let (generation, _) =
                                            unsafe { wait_reply(call, REPLY_SPINS) };
                                        ipc_close(call);
                                        generation
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
                    if message.reply != 0 {
                        ipc_reply(message.reply, clusterctl::ERR_NOT_IMPLEMENTED);
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

/// The node key attached to an `OP_DEPLOY` call: `[node_key:u64 LE]`.
fn read_node_key(message: &catten_syscall::IpcMessage) -> Option<u64> {
    if message.memory == 0 {
        return None;
    }
    if memory_map(message.memory, DATA_VADDR, false) != 0 {
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
