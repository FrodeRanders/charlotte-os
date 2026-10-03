//! Owned application client for the capability-grant controller.

use catten_rt::owned::{
    Connection,
    ConnectionRef,
    Endpoint,
    IpcError,
    LaunchMemoryRef,
    MemoryError,
    OwnedMemory,
};
use catten_syscall::IpcRights;

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
/// application. Waits at most five monotonic seconds, retrying only temporary
/// unavailability. Every attempt owns its pending call; expiry cancels it.
pub fn acquire(
    controller: ConnectionRef<'_>,
    descriptor: &LaunchMemoryRef<'_>,
    service: &[u8],
    rights: u16,
) -> Result<Connection, Error> {
    let deadline = crate::deadline::Deadline::after(5_000);
    loop {
        match acquire_once(controller, descriptor, service, rights, &deadline) {
            Err(Error::Service(grant::ERR_UNAVAILABLE)) if !deadline.expired() => {
                crate::sleep_ms(100);
            }
            result => return result,
        }
    }
}

fn acquire_once(
    controller: ConnectionRef<'_>,
    descriptor: &LaunchMemoryRef<'_>,
    service: &[u8],
    rights: u16,
    deadline: &crate::deadline::Deadline,
) -> Result<Connection, Error> {
    if deadline.expired() {
        return Err(Error::Service(grant::ERR_UNAVAILABLE));
    }
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
    let mut pending = controller
        .call_move(grant::OP_ACQUIRE, len as u64, request)
        .map_err(|(_, error)| Error::Ipc(error))?;
    let reply = loop {
        if deadline.expired() {
            return Err(Error::Service(grant::ERR_UNAVAILABLE));
        }
        if let Some(reply) = pending.poll().map_err(Error::Ipc)? {
            break reply;
        }
        crate::sleep_ms(5);
    };
    if reply.result < 0 {
        return Err(Error::Service(reply.result));
    }
    if reply.memory.is_some() {
        return Err(Error::InvalidRequest);
    }
    reply.connection.ok_or(Error::InvalidRequest)
}

/// Publish one application-owned endpoint under an exact signed descriptor
/// grant. The controller and name service receive only delegated connections;
/// this function retains ownership of `endpoint` for the serving loop.
pub fn publish(
    controller: ConnectionRef<'_>,
    descriptor: &LaunchMemoryRef<'_>,
    service: &[u8],
    endpoint: &Endpoint,
) -> Result<i64, Error> {
    let descriptor_mapping = descriptor.map_read_only().map_err(Error::Memory)?;
    let request = OwnedMemory::allocate(1).map_err(Error::Memory)?;
    let mut request_mapping = request.map_writable().map_err(|(_, error)| Error::Memory(error))?;
    let len = grant::encode_request(
        service,
        charlotte_launch::deployment::RIGHT_PUBLISH,
        descriptor_mapping.as_slice(),
        request_mapping.as_mut_slice(),
    )
    .ok_or(Error::InvalidRequest)?;
    let request = request_mapping.unmap().map_err(|(_, error)| Error::Memory(error))?;
    let reply = controller
        .call_connection_copy(
            grant::OP_PUBLISH,
            len as u64,
            endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
            &request,
        )
        .map_err(Error::Ipc)?
        .wait()
        .map_err(Error::Ipc)?;
    if reply.result < 1 {
        Err(Error::Service(reply.result))
    } else if reply.connection.is_some() || reply.memory.is_some() {
        Err(Error::InvalidRequest)
    } else {
        Ok(reply.result)
    }
}
