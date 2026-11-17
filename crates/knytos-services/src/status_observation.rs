//! One total budget for the RPCs composing a status snapshot.
//!
//! Each accepted call stays owned until observation or checked cancellation.
//! A missing result is unavailable data, never permission to replay a request.

use catten_rt::owned::{
    CallResult,
    ConnectionRef,
    IpcError,
    PendingCall,
};

use crate::{
    deadline::Deadline,
    socket,
};

pub const SNAPSHOT_MS: u64 = 5_000;

#[derive(Debug)]
pub enum Error {
    Admission(socket::SocketError),
    Observation(IpcError),
    Timeout,
}

pub struct Snapshot {
    deadline: Deadline,
}

impl Snapshot {
    pub fn new(deadline: Deadline) -> Self {
        Self {
            deadline,
        }
    }

    fn admit(&self) -> Result<(), Error> {
        if self.deadline.expired() {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }

    fn observe(&self, mut pending: PendingCall<'_>) -> Result<CallResult, Error> {
        // Timeout and wait admission rejection retain the pending owner until
        // Drop confirms cancellation or aborts. Neither proves remote rollback.
        self.admit()?;
        pending
            .wait_timeout(self.deadline.remaining_ms())
            .map_err(Error::Observation)?
            .ok_or(Error::Timeout)
    }

    pub fn call(
        &self,
        connection: ConnectionRef<'_>,
        opcode: u32,
        arg0: u64,
    ) -> Result<CallResult, Error> {
        self.admit()?;
        let pending = connection
            .call(opcode, arg0)
            .map_err(|error| Error::Admission(socket::SocketError::Ipc(error)))?;
        self.observe(pending)
    }

    pub fn buffered_call(
        &self,
        connection: ConnectionRef<'_>,
        opcode: u32,
        arg0: u64,
    ) -> Result<CallResult, Error> {
        // Do not allocate or publish another moved buffer after an earlier
        // source has consumed the snapshot's budget.
        self.admit()?;
        let pending = socket::buffered_call(connection, opcode, arg0).map_err(Error::Admission)?;
        self.observe(pending)
    }
}
