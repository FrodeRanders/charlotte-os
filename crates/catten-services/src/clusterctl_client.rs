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
    loop {
        if deadline.expired() {
            // Drop cancels the exact pending owner. Timeout does not prove a
            // receiver failed to commit; no accepted request is replayed.
            return Err(Error::Timeout);
        }
        if let Some(reply) = pending.poll().map_err(Error::Observation)? {
            return Ok(reply);
        }
        crate::sleep_ms(5);
    }
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
