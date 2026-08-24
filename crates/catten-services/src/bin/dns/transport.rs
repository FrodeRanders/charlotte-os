//! Discovery and inbound Raft transport handling.

use alloc::vec::Vec;

use catten_graft::{
    node::RaftNode,
    wire::{
        encode_append_response,
        encode_snapshot_response,
        encode_vote_response,
    },
};
use catten_services::{
    disco,
    relmsg_transport::{
        InboundRpc,
        RelmsgRaftTransport,
        TAG_APPEND_RESPONSE,
        TAG_SNAPSHOT_RESPONSE,
        TAG_VOTE_RESPONSE,
    },
};
use catten_syscall::{
    ipc_close,
    ipc_reply_wait_with_memory,
    ipc_scalar_call,
    memory_close,
    memory_map_any,
    memory_unmap,
};

/// Query the discovery service for the current `(mac, node_id)` peer list.
pub(super) fn query_disco_peers(disco_conn: u64) -> Vec<([u8; 6], Vec<u8>)> {
    let call = ipc_scalar_call(disco_conn, disco::OP_LIST_PEERS, 0);
    if call == 0 {
        return Vec::new();
    }
    let (status, result, _returned_connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if status != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return Vec::new();
    }
    let len = result as usize;
    let (map_status, vaddr) = memory_map_any(memory, false);
    if map_status != 0 {
        memory_close(memory);
        return Vec::new();
    }
    let mut buf = Vec::with_capacity(len);
    unsafe {
        let src = vaddr as *const u8;
        for i in 0..len {
            buf.push(core::ptr::read_volatile(src.add(i)));
        }
        memory_unmap(memory);
    }
    memory_close(memory);
    charlotte_protocol_disco::parse_peer_list(&buf)
}

/// Apply one inbound Raft RPC and return its response through relmsg.
pub(super) fn drive_inbound(
    node: &mut RaftNode,
    transport: &RelmsgRaftTransport,
    source_mac: [u8; 6],
    inbound: InboundRpc,
    millis: u64,
) {
    match inbound {
        InboundRpc::VoteRequest(request) => {
            let response = node.handle_vote_request(request, millis);
            if let Ok(payload) = encode_vote_response(&response) {
                transport.send_response(source_mac, TAG_VOTE_RESPONSE, payload);
            }
        }
        InboundRpc::AppendEntries(request) => {
            let response = node.handle_append_entries(request, millis);
            if let Ok(payload) = encode_append_response(&response) {
                transport.send_response(source_mac, TAG_APPEND_RESPONSE, payload);
            }
        }
        InboundRpc::InstallSnapshot(request) => {
            let response = node.handle_install_snapshot(request, millis);
            if let Ok(payload) = encode_snapshot_response(&response) {
                transport.send_response(source_mac, TAG_SNAPSHOT_RESPONSE, payload);
            }
        }
    }
}
