//! Small, independently reviewable phases of the DNS event reactor.

use alloc::{
    collections::VecDeque,
    vec::Vec,
};

use catten_graft::{
    node::RaftNode,
    types::NodeState,
};
use catten_rt::config;
use catten_services::{
    dns,
    name_catalog::NameCatalog,
    relmsg_transport::RelmsgRaftTransport,
};
use catten_syscall::{
    ipc_close,
    ipc_reply,
    ipc_reply_poll_with_memory,
    submit_detached_timer,
};

use super::{
    LOOP_TICK_MS,
    RAFT_TIMER_COOKIE,
    local_calls::poll_local_call,
    state::{
        CompletedCall,
        InFlightCall,
        LocalCallDestination,
        PendingLocalCall,
        PendingQuery,
        PendingQueryKind,
    },
};

/// Retire completed unregister calls without parking the reactor.
pub(super) fn drain_local_unregistrations(calls: &mut Vec<u64>) {
    calls.retain(|call| {
        let (status, _result, _connection, _memory) = ipc_reply_poll_with_memory(*call);
        if status == 1 {
            true
        } else {
            ipc_close(*call);
            false
        }
    });
}

/// Advance node-local invocations while continuing to service Raft traffic.
pub(super) fn drive_local_calls(
    pending: &mut Vec<PendingLocalCall>,
    completed: &mut VecDeque<CompletedCall>,
    remote_calls_served: &mut u32,
    transport: &RelmsgRaftTransport,
    now: u64,
) {
    let mut index = 0;
    while index < pending.len() {
        let Some(result) = poll_local_call(&mut pending[index], now) else {
            index += 1;
            continue;
        };
        let call = pending.swap_remove(index);
        match call.destination {
            LocalCallDestination::Client {
                reply,
            } => {
                if reply != 0 {
                    ipc_reply(reply, result);
                }
            }
            LocalCallDestination::Remote {
                caller,
                session,
                call_id,
                target_generation,
                peer,
                settled_after_ack,
            } => {
                completed.push_back(CompletedCall {
                    caller,
                    session,
                    call_id,
                    result,
                    peer: peer.clone(),
                    settled_after_ack,
                });
                *remote_calls_served = remote_calls_served.wrapping_add(1);
                config::write_u32_release(dns::status::REMOTE_CALLS_SERVED, *remote_calls_served);
                transport.send_message(
                    &peer,
                    catten_services::rcall::TAG_REPLY,
                    catten_services::rcall::encode_reply(
                        session,
                        call_id,
                        target_generation,
                        result,
                    ),
                );
            }
        }
    }
}

/// Expire remote calls with an explicitly uncertain outcome: the target may
/// have executed before its reply was lost.
pub(super) fn expire_remote_calls(calls: &mut Vec<InFlightCall>, now: u64) {
    let mut index = 0;
    while index < calls.len() {
        if calls[index].deadline > now {
            index += 1;
            continue;
        }
        let call = calls.swap_remove(index);
        if call.reply != 0 {
            ipc_reply(call.reply, dns::ERR_UNCERTAIN);
        }
    }
}

/// Expire leader-routed catalog operations. Lookups have no side effect;
/// relayed deployment retries are safe because exact desired state is
/// idempotent in the replicated catalog.
pub(super) fn expire_queries(queries: &mut Vec<PendingQuery>, now: u64) {
    let mut index = 0;
    while index < queries.len() {
        if queries[index].deadline > now {
            index += 1;
            continue;
        }
        let query = queries.swap_remove(index);
        let reply = match query.kind {
            PendingQueryKind::Lookup {
                reply,
                ..
            }
            | PendingQueryKind::Call {
                reply,
                ..
            }
            | PendingQueryKind::Deploy {
                reply,
            } => reply,
        };
        if reply != 0 {
            ipc_reply(reply, dns::ERR_NOT_LEADER);
        }
    }
}

/// Advance the logical Raft clock and maintain its detached wake-up timer.
pub(super) fn advance_raft_clock(
    node: &mut RaftNode,
    tick_due: bool,
    heartbeat_interval_ms: u64,
    last_heartbeat_broadcast: &mut u64,
    timer_armed: &mut bool,
) {
    if !tick_due {
        return;
    }
    node.set_millis(node.millis() + LOOP_TICK_MS);
    if node.check_timeout() {
        node.start_election(node.millis());
    }
    if node.state == NodeState::Leader
        && node.millis().saturating_sub(*last_heartbeat_broadcast) >= heartbeat_interval_ms
    {
        node.broadcast_heartbeat(node.millis());
        *last_heartbeat_broadcast = node.millis();
    }
    if !*timer_armed {
        *timer_armed = submit_detached_timer(LOOP_TICK_MS, 0, RAFT_TIMER_COOKIE) != u64::MAX;
    }
}

pub(super) fn publish_status(node: &RaftNode, catalog: &NameCatalog) {
    config::write_u32_release(
        dns::status::PEER_COUNT,
        node.cluster_configuration.all_members().len() as u32,
    );
    config::write_u32_release(dns::status::CURRENT_TERM, node.current_term as u32);
    config::write_u32_release(
        dns::status::RAFT_STATE,
        match node.state {
            NodeState::Candidate => 2,
            NodeState::Leader => 3,
            NodeState::Follower => 1,
        },
    );
    config::write_u32_release(dns::status::CATALOG_ENTRIES, catalog.registered_count() as u32);
}
