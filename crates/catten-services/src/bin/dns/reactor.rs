//! Small, independently reviewable phases of the DNS event reactor.

use alloc::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    string::String,
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
    ipc_reply,
    submit_detached_timer,
};

use super::{
    LOOP_TICK_MS,
    RAFT_TIMER_COOKIE,
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
pub(super) fn drain_local_unregistrations(
    calls: &mut Vec<catten_services::publication_generation::Unregistration>,
) {
    calls.retain_mut(|call| match call.advance() {
        None => true,
        Some(catten_services::publication_generation::Outcome::Removed) => false,
        Some(outcome) => {
            catten_rt::logln!("[dns] local retirement terminal outcome {:?}; no replay", outcome);
            false
        }
    });
}

/// Advance node-local invocations while continuing to service Raft traffic.
pub(super) fn drive_local_calls(
    pending: &mut Vec<PendingLocalCall>,
    completed: &mut VecDeque<CompletedCall>,
    next_reply_ordinal: &mut BTreeMap<String, u64>,
    remote_calls_served: &mut u32,
    transport: &RelmsgRaftTransport,
) {
    let mut index = 0;
    while index < pending.len() {
        let Some(result) = pending[index].operation.advance() else {
            index += 1;
            continue;
        };
        let call = pending.swap_remove(index);
        match call.destination {
            LocalCallDestination::Client {
                reply,
            } => {
                if let Some(reply) = reply {
                    let _ = reply.reply(result);
                }
            }
            LocalCallDestination::Remote {
                caller,
                session,
                call_id,
                peer,
                reply,
            } => {
                // Reserve the reply ordinal here, in reply-send order, so the
                // per-peer ACK count can identify this entry as settled even
                // when concurrently admitted calls complete out of order.
                let reply_ordinal = next_reply_ordinal.entry(peer.clone()).or_insert_with(|| {
                    transport.acknowledged_count_for(&peer, catten_services::rcall::TAG_REPLY)
                });
                *reply_ordinal = reply_ordinal.saturating_add(1);
                completed.push_back(CompletedCall {
                    caller,
                    session,
                    call_id,
                    result,
                    peer: peer.clone(),
                    settled_after_ack: *reply_ordinal,
                });
                *remote_calls_served = remote_calls_served.wrapping_add(1);
                config::write_u32_release(dns::status::REMOTE_CALLS_SERVED, *remote_calls_served);
                transport.send_message(
                    &peer,
                    catten_services::rcall::TAG_REPLY,
                    reply.finish(result),
                );
            }
        }
    }
}

/// Expire remote calls with an explicitly uncertain outcome: the target may
/// have executed before its reply was lost.
pub(super) fn expire_remote_calls(calls: &mut Vec<InFlightCall>) {
    let mut index = 0;
    while index < calls.len() {
        if !calls[index].deadline.expired() {
            index += 1;
            continue;
        }
        let call = calls.swap_remove(index);
        if let Some(reply) = call.reply {
            let _ = reply.reply(dns::ERR_UNCERTAIN);
        }
    }
}

/// Expire leader-routed catalog operations. Lookups have no side effect;
/// relayed deployment retries are safe because exact desired state is
/// idempotent in the replicated catalog.
pub(super) fn expire_queries(queries: &mut Vec<PendingQuery>) {
    let mut index = 0;
    while index < queries.len() {
        if !queries[index].deadline.expired() {
            index += 1;
            continue;
        }
        let query = queries.swap_remove(index);
        let reply = match query.kind {
            PendingQueryKind::Call {
                reply,
                ..
            } => {
                if let Some(reply) = reply {
                    let _ = reply.reply(dns::ERR_NOT_LEADER);
                }
                continue;
            }
            PendingQueryKind::Lookup {
                reply,
                ..
            }
            | PendingQueryKind::Deploy {
                reply,
            }
            | PendingQueryKind::Release {
                reply,
            }
            | PendingQueryKind::Operations {
                reply,
            }
            | PendingQueryKind::Shutdown {
                reply,
            }
            | PendingQueryKind::IngressPolicy {
                reply,
            } => reply,
        };
        if reply != 0 {
            ipc_reply(reply, dns::ERR_NOT_LEADER);
        }
    }
}

/// Sample elapsed Raft time and maintain its detached wake-up timer.
pub(super) fn advance_raft_clock(
    node: &mut RaftNode,
    clock: &mut catten_graft::clock::MonotonicClock,
    tick_due: bool,
    heartbeat_interval_ms: u64,
    last_heartbeat_broadcast: &mut u64,
    timer_armed: &mut bool,
) {
    catten_services::raft_clock::refresh(node, clock);
    if !tick_due {
        return;
    }
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
