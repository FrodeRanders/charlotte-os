//! Non-blocking node-local service invocation state machine.

use catten_services::{
    dns,
    ns,
};
use catten_syscall::{
    ipc_close,
    ipc_reply_poll_with_memory,
    ipc_scalar_call,
    memory_close,
};

use super::state::{
    LocalCallDestination,
    LocalCallStage,
    PendingLocalCall,
};

/// Begin a service invocation through the node-local name service. The caller
/// must retain and poll the returned state from its reactor.
pub(super) fn begin_local_call(
    ns_conn: u64,
    name: &[u8],
    opcode: u32,
    arg: i64,
    deadline: u64,
    destination: LocalCallDestination,
) -> Result<PendingLocalCall, i64> {
    let lookup = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, catten_services::name(name));
    if lookup == 0 {
        return Err(dns::ERR_NOT_FOUND);
    }
    Ok(PendingLocalCall {
        completion: lookup,
        connection: 0,
        opcode,
        arg,
        deadline,
        stage: LocalCallStage::Lookup,
        destination,
    })
}

/// Advance one local invocation without blocking the Raft reactor. Returns a
/// terminal scalar result when lookup/invocation finishes or its deadline
/// expires.
pub(super) fn poll_local_call(call: &mut PendingLocalCall, now: u64) -> Option<i64> {
    if now >= call.deadline {
        ipc_close(call.completion);
        if call.connection != 0 {
            ipc_close(call.connection);
            call.connection = 0;
        }
        // Once invocation was submitted the target may have executed even if
        // its reply did not arrive. Preserve that uncertainty for callers.
        return Some(match call.stage {
            LocalCallStage::Lookup => dns::ERR_NOT_FOUND,
            LocalCallStage::Invoke => dns::ERR_UNCERTAIN,
        });
    }

    let (status, result, returned_connection, memory) = ipc_reply_poll_with_memory(call.completion);
    if status == 1 {
        return None;
    }
    ipc_close(call.completion);
    if memory != 0 {
        memory_close(memory);
    }

    match call.stage {
        LocalCallStage::Lookup => {
            if status != 0 || result < 1 || returned_connection == 0 {
                if returned_connection != 0 {
                    ipc_close(returned_connection);
                }
                return Some(dns::ERR_NOT_FOUND);
            }
            let completion = ipc_scalar_call(returned_connection, call.opcode, call.arg as u64);
            if completion == 0 {
                ipc_close(returned_connection);
                return Some(dns::ERR_NOT_FOUND);
            }
            call.completion = completion;
            call.connection = returned_connection;
            call.stage = LocalCallStage::Invoke;
            None
        }
        LocalCallStage::Invoke => {
            if returned_connection != 0 {
                ipc_close(returned_connection);
            }
            ipc_close(call.connection);
            call.connection = 0;
            Some(
                if status == 0 {
                    result as i64
                } else {
                    dns::ERR_UNCERTAIN
                },
            )
        }
    }
}
