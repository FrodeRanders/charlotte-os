//! Owned application client for the capability-grant controller.

use catten_rt::owned::{
    Connection,
    ConnectionRef,
    IpcError,
    LaunchMemoryRef,
    MemoryError,
    OwnedMemory,
};

use crate::grant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Ipc(IpcError),
    Memory(MemoryError),
    InvalidRequest,
    Service(i64),
}

/// Acquire one capability allowed by the launch-provided signed deployment
/// descriptor. No name-service connection or connector secret enters the
/// application.
pub fn acquire(
    controller: ConnectionRef<'_>,
    descriptor: &LaunchMemoryRef<'_>,
    service: &[u8],
    rights: u16,
) -> Result<Connection, Error> {
    let descriptor_mapping = descriptor.map_read_only().map_err(Error::Memory)?;
    let request = OwnedMemory::allocate(1).map_err(Error::Memory)?;
    let mut request_mapping = request.map_writable().map_err(|(_, error)| Error::Memory(error))?;
    let len = grant::encode_request(
        service,
        rights,
        descriptor_mapping.as_slice(),
        request_mapping.as_mut_slice(),
    )
    .ok_or(Error::InvalidRequest)?;
    let request = request_mapping.unmap().map_err(|(_, error)| Error::Memory(error))?;
    let reply = controller
        .call_move(grant::OP_ACQUIRE, len as u64, request)
        .map_err(|(_, error)| Error::Ipc(error))?
        .wait()
        .map_err(Error::Ipc)?;
    if reply.result < 0 {
        return Err(Error::Service(reply.result));
    }
    if reply.memory.is_some() {
        return Err(Error::InvalidRequest);
    }
    reply.connection.ok_or(Error::InvalidRequest)
}
