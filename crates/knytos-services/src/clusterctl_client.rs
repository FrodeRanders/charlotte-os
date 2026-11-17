//! Bounded cluster-control calls for the deployment HTTP adapter.

use catten_rt::owned::{
    CallResult,
    ConnectionRef,
    IpcError,
    OwnedMemory,
    PendingCall,
};

use crate::{
    clusterctl,
    deadline::Deadline,
};

pub const RPC_BUDGET_MS: u64 = 5_000;

#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    Admission(IpcError),
    Observation(IpcError),
    Timeout,
}

impl Error {
    pub const fn code(&self) -> i64 {
        match self {
            Self::Timeout => clusterctl::ERR_RPC_TIMEOUT,
            Self::Admission(_) | Self::Observation(_) => clusterctl::ERR_NOT_LEADER,
        }
    }
}

fn observe(mut pending: PendingCall<'_>, deadline: &Deadline) -> Result<CallResult, Error> {
    // Drop cancels the exact pending owner on timeout or wait rejection.
    // Neither outcome establishes whether the receiver committed; never replay.
    if deadline.expired() {
        return Err(Error::Timeout);
    }
    pending.wait_timeout(deadline.remaining_ms()).map_err(Error::Observation)?.ok_or(Error::Timeout)
}

/// Submit once and observe within one deadline, starting before submission.
pub fn call(connection: ConnectionRef<'_>, opcode: u32, arg0: u64) -> Result<CallResult, Error> {
    let deadline = Deadline::after(RPC_BUDGET_MS);
    let pending = connection.call(opcode, arg0).map_err(Error::Admission)?;
    observe(pending, &deadline)
}

/// Transfer one request owner exactly once. Submission rejection releases the
/// returned request; timeout cancels the pending owner. A delivered request
/// remains with its receiver, so cancellation is not remote rollback.
pub fn call_move(
    connection: ConnectionRef<'_>,
    opcode: u32,
    arg0: u64,
    request: OwnedMemory,
) -> Result<CallResult, Error> {
    let deadline = Deadline::after(RPC_BUDGET_MS);
    let pending = connection
        .call_move(opcode, arg0, request)
        .map_err(|(_request, error)| Error::Admission(error))?;
    observe(pending, &deadline)
}
