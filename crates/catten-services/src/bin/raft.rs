#![allow(unused_unsafe)]
#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;

use catten_graft::charlotte::CharlotteTransport;
use catten_graft::log_store::{InMemoryLogStore, InMemoryPersistentStateStore};
use catten_graft::node::RaftNode;
use catten_graft::types::{
    Peer, NodeState, VoteRequest, AppendEntriesRequest, InstallSnapshotRequest,
};
use catten_graft::wire::{
    VoteRequestWire, VoteResponseWire, AppendEntriesRequestWire,
    AppendEntriesResponseWire, InstallSnapshotRequestWire,
    InstallSnapshotResponseWire, SCRATCH_VADDR,
};
use catten_rt::{Args, Input, config};
use catten_services::{ns, raft};
use catten_syscall::{
    IpcRights, cq_wait, ipc_endpoint_bind_cq, ipc_endpoint_create,
    ipc_recv, ipc_reply, ipc_reply_move, ipc_scalar_call_connection,
    ipc_reply_poll_with_memory, ipc_status, ipc_close, memory_alloc,
    memory_close, memory_map, memory_unmap, thread_exit,
    submit_timer, wait_timeout,
};

const POLL_SPINS: u64 = 50_000;

unsafe fn wait_reply_3(call: u64, max_spins: u64) -> Option<(i64, u64, u64)> {
    let mut spins: u64 = 0;
    loop {
        let (status, result, conn, mem) = unsafe { ipc_reply_poll_with_memory(call) };
        if status == 0 {
            unsafe { ipc_close(call); }
            return Some((result as i64, conn, mem));
        }
        if status != ipc_status::PENDING {
            return None;
        }
        spins += 1;
        if spins >= max_spins {
            return None;
        }
        core::hint::spin_loop();
    }
}

unsafe fn wait_reply_2(call: u64, max_spins: u64) -> Option<(i64, u64)> { unsafe {
    let (val1, val2, _val3) = wait_reply_3(call, max_spins)?;
    Some((val1, val2))
}}

unsafe fn write_struct_to_mem<T>(val: &T) -> Option<u64> {
    let cap = unsafe { memory_alloc(1) };
    if cap == 0 {
        return None;
    }
    if unsafe { memory_map(cap, SCRATCH_VADDR, true) } != 0 {
        unsafe { memory_close(cap); }
        return None;
    }
    let size = core::mem::size_of::<T>();
    unsafe {
        core::ptr::copy_nonoverlapping(
            (val as *const T) as *const u8,
            SCRATCH_VADDR as *mut u8,
            size,
        );
        memory_unmap(cap);
    }
    Some(cap)
}

unsafe fn read_struct_from_mem<T>(cap: u64) -> Option<T> {
    if cap == 0 {
        return None;
    }
    if unsafe { memory_map(cap, SCRATCH_VADDR, false) } != 0 {
        return None;
    }
    let val: T = unsafe { core::ptr::read_volatile(SCRATCH_VADDR as *const T) };
    unsafe {
        memory_unmap(cap);
        memory_close(cap);
    }
    Some(val)
}

fn cmain(args: Args, _input: Input<256>) -> ! {
    let argc = args.len();

    let c0 = args.get(0).unwrap_or(b'r' as u32) as u8;
    let c1 = args.get(1).unwrap_or(b'1' as u32) as u8;
    let raw_id = [c0, c1];
    let node_id = core::str::from_utf8(&raw_id).unwrap_or("r1");

    config::write::<u32>(0, 1);

    let ns_conn = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 2);

    let endpoint = unsafe { ipc_endpoint_create(raft::INTERFACE, raft::VERSION, 8) };
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 3);

    let name_u64 = catten_services::name(alloc::format!("raft-{}", node_id).as_bytes());

    let register = unsafe {
        ipc_scalar_call_connection(
            ns_conn,
            ns::OP_REGISTER,
            name_u64,
            endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
    };
    if register == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 4);

    let (generation, _) = unsafe { wait_reply_2(register, POLL_SPINS * 10).unwrap_or((-1, 0)) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(4, generation as u32);

    let log_store = Box::new(InMemoryLogStore::new());
    let persistent_store = Box::new(InMemoryPersistentStateStore::new());
    let transport = Box::new(CharlotteTransport::new());

    let mut peers = Vec::new();
    let me = Peer::voter(node_id.to_string(), name_u64);
    peers.push(me.clone());

    for i in 2..argc {
        let pc0 = args.get(i).unwrap_or(0) as u8;
        let pc1 = args.get(i + 1).unwrap_or(0) as u8;
        let rid = [pc0, pc1];
        let peer_id = core::str::from_utf8(&rid).unwrap_or("");
        if peer_id.is_empty() {
            continue;
        }

        let peer_name = catten_services::name(alloc::format!("raft-{}", peer_id).as_bytes());
        let lookup = unsafe {
            ipc_scalar_call_connection(
                ns_conn,
                ns::OP_LOOKUP,
                peer_name,
                0,
                IpcRights::SEND | IpcRights::CALL,
            )
        };

        if lookup == 0 {
            continue;
        }

        let (_gen, peer_conn) = unsafe { wait_reply_2(lookup, POLL_SPINS).unwrap_or((-1, 0)) };
        if peer_conn == 0 {
            continue;
        }

        config::write::<u32>(20 + i, peer_conn as u32);

        transport.add_peer(peer_id, peer_conn);
        peers.push(Peer::voter(peer_id.to_string(), peer_name));
    }

    config::write::<u32>(0, 5);

    if unsafe { ipc_endpoint_bind_cq(endpoint, 0) } != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 6);

    let mut node = RaftNode::new(
        me,
        150,
        log_store,
        persistent_store,
        None,
        peers,
        transport,
        0,
    );

    let mut served: u32 = 0;

    let election_timeout_ms: u64 = 150 + 100;
    let mut election_timer: u64 = unsafe { submit_timer(election_timeout_ms) };

    loop {
        unsafe { cq_wait(1, 0); }

        let timer_fired = if election_timer != 0 {
            let (status, _result) = unsafe { wait_timeout(election_timer, 0) };
            if status == 0 {
                unsafe { ipc_close(election_timer); }
                true
            } else {
                false
            }
        } else {
            false
        };

        if timer_fired {
            node.set_millis(node.millis() + election_timeout_ms);
            if node.check_timeout() {
                node.start_election(node.millis());
                config::write::<u32>(8, match node.state {
                    NodeState::Candidate => 2,
                    NodeState::Leader => 3,
                    NodeState::Follower => 1,
                });
            }
            election_timer = unsafe { submit_timer(election_timeout_ms) };
        }

        if node.state == NodeState::Leader {
            node.broadcast_heartbeat(node.millis());
            node.set_millis(node.millis() + 50);
        }

        loop {
            let message = unsafe { ipc_recv(endpoint) };
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !message.is_ok() {
                break;
            }

            served += 1;
            config::write::<u32>(12, served);

            match message.opcode {
                raft::OP_VOTE_REQUEST => {
                    let mut wire = VoteRequestWire {
                        term: message.arg0,
                        candidate_id_len: 0,
                        candidate_id_off: 0,
                        last_log_index: 0,
                        last_log_term: 0,
                    };

                    if message.memory != 0 {
                        if let Some(w) = unsafe { read_struct_from_mem::<VoteRequestWire>(message.memory) } {
                            wire = w;
                            wire.term = message.arg0;
                        }
                    }

                    let vote_req = VoteRequest {
                        term: wire.term,
                        candidate_id: node_id.to_string(),
                        last_log_index: wire.last_log_index,
                        last_log_term: wire.last_log_term,
                    };

                    let resp = node.handle_vote_request(vote_req, node.millis());
                    let resp_wire = VoteResponseWire {
                        term: resp.term,
                        vote_granted: if resp.vote_granted { 1 } else { 0 },
                        _pad: [0; 7],
                    };

                    if message.reply != 0 {
                        if let Some(mem) = unsafe { write_struct_to_mem(&resp_wire) } {
                            unsafe { ipc_reply_move(message.reply, mem, resp.term as i64); }
                        } else {
                            unsafe { ipc_reply(message.reply, if resp.vote_granted { 1 } else { 0 }); }
                        }
                    }
                }

                raft::OP_APPEND_ENTRIES => {
                    let mut wire = AppendEntriesRequestWire {
                        term: message.arg0,
                        leader_id_len: 0,
                        leader_id_off: 0,
                        prev_log_index: 0,
                        prev_log_term: 0,
                        leader_commit: 0,
                        entry_count: 0,
                        entries_data_off: 0,
                    };

                    if message.memory != 0 {
                        if let Some(w) = unsafe { read_struct_from_mem::<AppendEntriesRequestWire>(message.memory) } {
                            wire = w;
                            wire.term = message.arg0;
                        }
                    }

                    let ae_req = AppendEntriesRequest {
                        term: wire.term,
                        leader_id: node_id.to_string(),
                        prev_log_index: wire.prev_log_index,
                        prev_log_term: wire.prev_log_term,
                        leader_commit: wire.leader_commit,
                        entries: Vec::new(),
                    };

                    let resp = node.handle_append_entries(ae_req, node.millis());
                    let resp_wire = AppendEntriesResponseWire {
                        term: resp.term,
                        success: if resp.success { 1 } else { 0 },
                        _pad: [0; 7],
                        match_index: resp.match_index,
                    };

                    if message.reply != 0 {
                        if let Some(mem) = unsafe { write_struct_to_mem(&resp_wire) } {
                            let packed = (resp.term << 1) as i64 | if resp.success { 1 } else { 0 };
                            unsafe { ipc_reply_move(message.reply, mem, packed); }
                        } else {
                            let result = if resp.success { resp.term as i64 } else { -(resp.term as i64) };
                            unsafe { ipc_reply(message.reply, result); }
                        }
                    }
                }

                raft::OP_INSTALL_SNAPSHOT => {
                    let mut wire = InstallSnapshotRequestWire {
                        term: message.arg0,
                        leader_id_len: 0,
                        leader_id_off: 0,
                        last_included_index: 0,
                        last_included_term: 0,
                        offset: 0,
                        data_len: 0,
                        data_off: 0,
                        done: 0,
                        _pad: [0; 7],
                    };

                    if message.memory != 0 {
                        if let Some(w) = unsafe { read_struct_from_mem::<InstallSnapshotRequestWire>(message.memory) } {
                            wire = w;
                            wire.term = message.arg0;
                        }
                    }

                    let snap_req = InstallSnapshotRequest {
                        term: wire.term,
                        leader_id: node_id.to_string(),
                        last_included_index: wire.last_included_index,
                        last_included_term: wire.last_included_term,
                        offset: wire.offset,
                        data: Vec::new(),
                        done: wire.done != 0,
                    };

                    let resp = node.handle_install_snapshot(snap_req, node.millis());
                    if message.reply != 0 {
                        let resp_wire = InstallSnapshotResponseWire { term: resp.term };
                        if let Some(mem) = unsafe { write_struct_to_mem(&resp_wire) } {
                            unsafe { ipc_reply_move(message.reply, mem, resp.term as i64); }
                        } else {
                            unsafe { ipc_reply(message.reply, resp.term as i64); }
                        }
                    }
                }

                raft::OP_STATUS => {
                    let status: u32 = match node.state {
                        NodeState::Follower => 1,
                        NodeState::Candidate => 2,
                        NodeState::Leader => 3,
                    };
                    let result = (status as i64)
                        | ((node.current_term as i64) << 8)
                        | ((node.commit_index as i64) << 32);
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, result); }
                    }
                }

                _ => {
                    if message.reply != 0 {
                        unsafe { ipc_reply(message.reply, -1); }
                    }
                }
            }
        }
    }
}

catten_rt::entry!(cmain);
