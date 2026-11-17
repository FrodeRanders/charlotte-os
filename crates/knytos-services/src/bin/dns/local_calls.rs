//! One owned node-local invocation, advanced on shared reactor readiness.

use catten_rt::owned::ConnectionRef;
use catten_services::{
    deadline::Deadline,
    local_invocation::LocalInvocation,
};

use super::state::{
    LocalCallDestination,
    PendingLocalCall,
};

pub(super) fn begin_local_call(
    names: ConnectionRef<'_>,
    name: &[u8],
    opcode: u32,
    arg: i64,
    deadline: Deadline,
    destination: LocalCallDestination,
) -> Result<PendingLocalCall, (i64, LocalCallDestination)> {
    match LocalInvocation::begin(names, name, opcode, arg, deadline, 0) {
        Ok(operation) => Ok(PendingLocalCall {
            operation,
            destination,
        }),
        Err(code) => Err((code, destination)),
    }
}
