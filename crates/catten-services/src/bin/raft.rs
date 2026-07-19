#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    string::{
        String,
        ToString,
    },
    sync::Arc,
    vec::Vec,
};

use catten_graft::{
    charlotte::CharlotteTransport,
    log_store::{
        InMemoryLogStore,
        InMemoryPersistentStateStore,
    },
    node::RaftNode,
    types::{
        NodeState,
        Peer,
    },
    wire::{
        RAFT_RPC_MEMORY_SIZE,
        SCRATCH_VADDR,
        decode_append_request,
        decode_snapshot_request,
        decode_vote_request,
        encode_append_response,
        encode_snapshot_response,
        encode_vote_response,
    },
};
use catten_rt::{
    Args,
    Input,
    config,
};
use catten_services::{
    ns,
    raft,
};
use catten_syscall::{
    IpcRights,
    close as completion_close,
    cq_wait,
    ipc_close,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_reply_move,
    ipc_reply_poll,
    ipc_scalar_call,
    ipc_scalar_call_connection,
    ipc_status,
    memory_alloc,
    memory_close,
    memory_map,
    memory_unmap,
    submit_timer,
    thread_exit,
    wait,
    wait_timeout,
};

// Registration happens while the system is still bringing up its services and
// can legitimately take much longer than a best-effort peer lookup.  Keeping
// these budgets separate also prevents an absent peer from stalling the Raft
// event loop for the full registration timeout.
const REGISTER_SPINS: u64 = 50_000_000;
const DISCOVERY_SPINS: u64 = 50_000;
const LOOP_TICK_MS: u64 = 25;

fn fatal(stage: u64) -> ! {
    catten_syscall::el0_log(0x5241_4654, stage);
    unsafe { thread_exit() }
}

unsafe fn wait_reply_2(call: u64, max_spins: u64) -> Option<(i64, u64)> {
    // Keep the capability in memory across the multi-register reply-poll
    // syscall. This guards against the returned x1 result being confused
    // with the x1 input capability by aggressive inlining/register reuse.
    let saved_call = call;
    let mut spins: u64 = 0;
    loop {
        let call_cap = unsafe { core::ptr::read_volatile(&saved_call) };
        let (status, result, connection) = ipc_reply_poll(call_cap);
        if status == 0 {
            ipc_close(unsafe { core::ptr::read_volatile(&saved_call) });
            return Some((result as i64, connection));
        }
        // IPC_REPLY_POLL has its own compact ABI: 0=ready, 1=pending.
        // This is not the IPC receive-status enum (whose PENDING value is 3).
        if status != 1 {
            return None;
        }
        spins += 1;
        if spins >= max_spins {
            return None;
        }
        // The Charlotte scheduler is cooperative at this point in boot. A
        // pure spin can starve the name service that must produce this reply.
        if spins % 1_000 == 0 {
            let timer = submit_timer(1);
            if timer != 0 {
                wait(timer);
                completion_close(timer);
            }
        }
        core::hint::spin_loop();
    }
}

fn write_payload_to_mem(payload: &[u8]) -> Option<u64> {
    if payload.len() > RAFT_RPC_MEMORY_SIZE {
        return None;
    }
    let cap = memory_alloc(1);
    if cap == 0 {
        return None;
    }
    if memory_map(cap, SCRATCH_VADDR, true) != 0 {
        memory_close(cap);
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), SCRATCH_VADDR as *mut u8, payload.len());
    }
    memory_unmap(cap);
    Some(cap)
}

fn read_payload_from_mem(cap: u64) -> Option<Vec<u8>> {
    if cap == 0 {
        return None;
    }
    let map_status = memory_map(cap, SCRATCH_VADDR, false);
    if map_status != 0 {
        memory_close(cap);
        return None;
    }
    let value = unsafe {
        core::slice::from_raw_parts(SCRATCH_VADDR as *const u8, RAFT_RPC_MEMORY_SIZE).to_vec()
    };
    memory_unmap(cap);
    memory_close(cap);
    Some(value)
}

fn reply_payload(reply: u64, payload: Result<Vec<u8>, catten_graft::wire::WireError>, term: u64) {
    if let Ok(payload) = payload {
        if let Some(memory) = write_payload_to_mem(&payload) {
            ipc_reply_move(reply, memory, term as i64);
            return;
        }
    }
    ipc_reply(reply, -1);
}

fn discover_peer(
    ns_conn: u64,
    peer_id: &str,
    peer_name: u64,
    transport: &CharlotteTransport,
) -> bool {
    if transport.has_peer(peer_id) {
        return true;
    }
    let lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, peer_name);
    if lookup == 0 {
        let timer = submit_timer(1);
        if timer != 0 {
            wait(timer);
            completion_close(timer);
        }
        return false;
    }
    let Some((generation, connection)) = (unsafe { wait_reply_2(lookup, DISCOVERY_SPINS) }) else {
        ipc_close(lookup);
        return false;
    };
    if generation < 1 || connection == 0 {
        return false;
    }
    transport.add_peer(peer_id, connection);
    true
}

fn cmain(args: Args, _input: Input<0>) -> ! {
    let argc = args.len();

    let c0 = args.get(0).unwrap_or(b'r' as u32) as u8;
    let c1 = args.get(1).unwrap_or(b'1' as u32) as u8;
    let raw_id = [c0, c1];
    let node_id = core::str::from_utf8(&raw_id).unwrap_or("r1");

    config::write::<u32>(0, 1);

    let ns_conn = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => fatal(1),
    };
    config::write::<u32>(0, 2);

    let endpoint = ipc_endpoint_create(raft::INTERFACE, raft::VERSION, 8);
    if endpoint == 0 {
        fatal(2);
    }
    if ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        fatal(5);
    }
    config::write::<u32>(0, 3);

    let name_u64 = catten_services::name(alloc::format!("raft-{}", node_id).as_bytes());

    let register = ipc_scalar_call_connection(
        ns_conn,
        ns::OP_REGISTER,
        name_u64,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 {
        fatal(3);
    }
    config::write::<u32>(0, 4);

    let (generation, _) = unsafe { wait_reply_2(register, REGISTER_SPINS).unwrap_or((-1, 0)) };
    if generation < 1 {
        fatal(4);
    }
    config::write::<u32>(4, generation as u32);

    let log_store = Box::new(InMemoryLogStore::new());
    let persistent_store = Box::new(InMemoryPersistentStateStore::new());
    let transport = Arc::new(CharlotteTransport::new());

    let mut peers = Vec::new();
    let me = Peer::voter(node_id.to_string(), name_u64);
    peers.push(me.clone());

    let mut peer_specs: Vec<(String, u64)> = Vec::new();
    let mut i = 2;
    while i + 1 < argc {
        let pc0 = args.get(i).unwrap_or(0) as u8;
        let pc1 = args.get(i + 1).unwrap_or(0) as u8;
        let rid = [pc0, pc1];
        let peer_id = core::str::from_utf8(&rid).unwrap_or("");
        if peer_id.is_empty() {
            i += 2;
            continue;
        }

        let peer_name = catten_services::name(alloc::format!("raft-{}", peer_id).as_bytes());
        peers.push(Peer::voter(peer_id.to_string(), peer_name));
        peer_specs.push((peer_id.to_string(), peer_name));
        let _ = discover_peer(ns_conn, peer_id, peer_name, &transport);
        i += 2;
    }

    config::write::<u32>(0, 5);

    config::write::<u32>(0, 6);

    let mut node =
        RaftNode::new(me, 150, log_store, persistent_store, None, peers, transport.clone(), 0);

    let mut served: u32 = 0;

    let mut election_timer: u64 = submit_timer(LOOP_TICK_MS);

    loop {
        cq_wait(1, 0);

        let completed = node.poll_transport(node.millis());
        if completed > 0 {
            config::write::<u32>(16, completed as u32);
        }
        config::write::<u32>(
            8,
            match node.state {
                NodeState::Candidate => 2,
                NodeState::Leader => 3,
                NodeState::Follower => 1,
            },
        );

        // Registration order is nondeterministic. Keep all configured voters
        // in the cluster and retry name-service discovery until their
        // connection becomes available.
        for (peer_id, peer_name) in &peer_specs {
            let _ = discover_peer(ns_conn, peer_id, *peer_name, &transport);
        }

        let timer_fired = if election_timer != 0 {
            let (status, _result) = wait_timeout(election_timer, 0);
            if status == 0 {
                completion_close(election_timer);
                true
            } else {
                false
            }
        } else {
            false
        };

        // Drain inbound Raft traffic before acting on an election timeout.
        // A vote request and timer can become ready together; processing the
        // timer first makes both nodes become candidates and reject each
        // other's otherwise valid vote.
        loop {
            let message = ipc_recv(endpoint);
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
                    let request = read_payload_from_mem(message.memory)
                        .and_then(|payload| decode_vote_request(&payload).ok());
                    if let Some(request) = request {
                        let response = node.handle_vote_request(request, node.millis());
                        if message.reply != 0 {
                            reply_payload(
                                message.reply,
                                encode_vote_response(&response),
                                response.term,
                            );
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }

                raft::OP_APPEND_ENTRIES => {
                    let request = read_payload_from_mem(message.memory)
                        .and_then(|payload| decode_append_request(&payload).ok());
                    if let Some(request) = request {
                        let response = node.handle_append_entries(request, node.millis());
                        if message.reply != 0 {
                            reply_payload(
                                message.reply,
                                encode_append_response(&response),
                                response.term,
                            );
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }

                raft::OP_INSTALL_SNAPSHOT => {
                    let request = read_payload_from_mem(message.memory)
                        .and_then(|payload| decode_snapshot_request(&payload).ok());
                    if let Some(request) = request {
                        let response = node.handle_install_snapshot(request, node.millis());
                        if message.reply != 0 {
                            reply_payload(
                                message.reply,
                                encode_snapshot_response(&response),
                                response.term,
                            );
                        }
                    } else if message.reply != 0 {
                        ipc_reply(message.reply, -1);
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

        if timer_fired {
            node.set_millis(node.millis() + LOOP_TICK_MS);
            if node.check_timeout() {
                node.start_election(node.millis());
            }
            election_timer = submit_timer(LOOP_TICK_MS);
        }

        if node.state == NodeState::Leader {
            node.broadcast_heartbeat(node.millis());
        }

        config::write::<u32>(
            8,
            match node.state {
                NodeState::Candidate => 2,
                NodeState::Leader => 3,
                NodeState::Follower => 1,
            },
        );
    }
}

catten_rt::entry!(cmain);
