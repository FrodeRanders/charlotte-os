//! Owned object-store transport for the Raft persistence pair.
//! Durable object IDs are names; dropping this client closes its connection,
//! never the stored objects. Accepted calls retain their moved memory owners.

use alloc::vec::Vec;

use catten_rt::owned::{
    Connection,
    ConnectionRef,
    OwnedMemory,
};

pub(crate) trait ObjectStore: Send + Sync {
    fn read(&self, object: u64) -> Result<Option<Vec<u8>>, ()>;
    /// Only exact successful creation authorizes an initial write.
    fn create(&self, object: u64) -> Result<(), ()>;
    fn write(&self, object: u64, bytes: &[u8]) -> Result<(), ()>;
    fn flush(&self) -> Result<(), ()>;
}

pub(crate) struct Client {
    connection: Connection,
}

/// Both transient buffers remain with this preparation until submission.
struct PreparedWrite {
    size: OwnedMemory,
    payload: OwnedMemory,
}

impl PreparedWrite {
    fn new(bytes: &[u8]) -> Result<Self, ()> {
        let mut size = OwnedMemory::allocate(1).map_err(|_| ())?.map_writable().map_err(|_| ())?;
        size.as_mut_slice()[..8].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
        let size = size.unmap().map_err(|_| ())?;
        let mut payload = OwnedMemory::allocate(bytes.len().max(1).div_ceil(4096))
            .map_err(|_| ())?
            .map_writable()
            .map_err(|_| ())?;
        payload.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            size,
            payload: payload.unmap().map_err(|_| ())?,
        })
    }
}

impl Client {
    pub(crate) fn connect(names: ConnectionRef<'_>) -> Result<Self, ()> {
        let (_, connection) =
            crate::wait_for_registered_name_owned(names, crate::objstore::NAME).ok_or(())?;
        Ok(Self {
            connection,
        })
    }
}

impl ObjectStore for Client {
    fn read(&self, object: u64) -> Result<Option<Vec<u8>>, ()> {
        let reply = self
            .connection
            .call(crate::objstore::OP_READ, object)
            .map_err(|_| ())?
            .wait()
            .map_err(|_| ())?;
        if reply.result == crate::objstore::ERR_NOT_FOUND && reply.memory.is_none() {
            return Ok(None);
        }
        let size = usize::try_from(reply.result).map_err(|_| ())?;
        let memory = reply.memory.ok_or(())?;
        if size > memory.len() {
            return Err(());
        }
        let mapping = memory.map_read_only().map_err(|_| ())?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| ())?;
        bytes.extend_from_slice(&mapping.as_slice()[..size]);
        Ok(Some(bytes))
    }

    fn create(&self, object: u64) -> Result<(), ()> {
        let reply = self
            .connection
            .call(crate::objstore::OP_CREATE_AT, object)
            .map_err(|_| ())?
            .wait()
            .map_err(|_| ())?;
        (reply.result == crate::objstore::ERR_OK).then_some(()).ok_or(())
    }

    fn write(&self, object: u64, bytes: &[u8]) -> Result<(), ()> {
        let prepared = PreparedWrite::new(bytes)?;
        let reply = self
            .connection
            .call_move(crate::objstore::OP_SET_SIZE, object, prepared.size)
            .map_err(|_| ())?
            .wait()
            .map_err(|_| ())?;
        if reply.result != crate::objstore::ERR_OK {
            return Err(());
        }
        let reply = self
            .connection
            .call_move(crate::objstore::OP_WRITE, object, prepared.payload)
            .map_err(|_| ())?
            .wait()
            .map_err(|_| ())?;
        (reply.result == crate::objstore::ERR_OK).then_some(()).ok_or(())
    }

    fn flush(&self) -> Result<(), ()> {
        let reply = self
            .connection
            .call(crate::objstore::OP_FLUSH, 0)
            .map_err(|_| ())?
            .wait()
            .map_err(|_| ())?;
        (reply.result == crate::objstore::ERR_OK).then_some(()).ok_or(())
    }
}
