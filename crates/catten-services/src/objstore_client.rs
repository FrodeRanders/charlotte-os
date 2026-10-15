//! Owned object-store transport for the Raft persistence pair.
//! Durable object IDs are names; dropping this client closes its connection,
//! never the stored objects. Accepted calls retain their moved memory owners.

use alloc::vec::Vec;

use catten_rt::owned::{
    CallResult,
    Connection,
    ConnectionRef,
    IpcError,
    MemoryError,
    OwnedMemory,
};

/// Diagnostic context only: object IDs are durable names, never capability owners.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoError {
    pub operation: IoOperation,
    pub object: Option<u64>,
    pub stage: IoStage,
    pub cause: IoCause,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoOperation {
    Lookup,
    Read,
    Create,
    SetSize,
    Write,
    Flush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoStage {
    Lookup,
    Allocate,
    Map,
    Unmap,
    Submit,
    Wait,
    Reply,
    Copy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoCause {
    LookupUnavailable,
    Memory(MemoryError),
    Ipc(IpcError),
    RemoteStatus(i64),
    InvalidReadReply {
        result: i64,
        memory_bytes: Option<usize>,
    },
    AllocationFailed,
}

#[derive(Clone, Copy)]
struct Request {
    operation: IoOperation,
    object: Option<u64>,
}

impl Request {
    fn error(self, stage: IoStage, cause: IoCause) -> IoError {
        IoError {
            operation: self.operation,
            object: self.object,
            stage,
            cause,
        }
    }

    fn status(self, result: i64) -> Result<(), IoError> {
        if result == crate::objstore::ERR_OK {
            Ok(())
        } else {
            Err(self.error(IoStage::Reply, IoCause::RemoteStatus(result)))
        }
    }

    fn read_size(self, result: i64, memory_bytes: Option<usize>) -> Result<Option<usize>, IoError> {
        if result == crate::objstore::ERR_NOT_FOUND && memory_bytes.is_none() {
            return Ok(None);
        }
        // Read status and byte lengths share the result register. Retain its
        // exact signed value and attachment capacity instead of guessing which
        // remote failure or malformed success produced an unusable reply.
        let size = usize::try_from(result).ok();
        match (size, memory_bytes) {
            (Some(size), Some(capacity)) if size <= capacity => Ok(Some(size)),
            _ => Err(self.error(
                IoStage::Reply,
                IoCause::InvalidReadReply {
                    result,
                    memory_bytes,
                },
            )),
        }
    }
}

pub(crate) trait ObjectStore: Send + Sync {
    fn read(&self, object: u64) -> Result<Option<Vec<u8>>, IoError>;
    /// Only exact successful creation authorizes an initial write.
    fn create(&self, object: u64) -> Result<(), IoError>;
    fn write(&self, object: u64, bytes: &[u8]) -> Result<(), IoError>;
    fn flush(&self) -> Result<(), IoError>;
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
    fn new(object: u64, bytes: &[u8]) -> Result<Self, IoError> {
        Ok(Self {
            size: Self::buffer(
                Request {
                    operation: IoOperation::SetSize,
                    object: Some(object),
                },
                &(bytes.len() as u64).to_le_bytes(),
            )?,
            payload: Self::buffer(
                Request {
                    operation: IoOperation::Write,
                    object: Some(object),
                },
                bytes,
            )?,
        })
    }

    fn buffer(request: Request, bytes: &[u8]) -> Result<OwnedMemory, IoError> {
        let memory = OwnedMemory::allocate(bytes.len().max(1).div_ceil(4096))
            .map_err(|error| request.error(IoStage::Allocate, IoCause::Memory(error)))?;
        let mut mapping = memory
            .map_writable()
            .map_err(|(_, error)| request.error(IoStage::Map, IoCause::Memory(error)))?;
        mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        mapping.unmap().map_err(|(_, error)| request.error(IoStage::Unmap, IoCause::Memory(error)))
    }
}

impl Client {
    pub(crate) fn connect(names: ConnectionRef<'_>) -> Result<Self, IoError> {
        let (_, connection) = crate::wait_for_registered_name_owned(names, crate::objstore::NAME)
            .ok_or_else(|| {
            Request {
                operation: IoOperation::Lookup,
                object: None,
            }
            .error(IoStage::Lookup, IoCause::LookupUnavailable)
        })?;
        Ok(Self {
            connection,
        })
    }

    fn call(&self, request: Request, opcode: u32) -> Result<CallResult, IoError> {
        self.connection
            .call(opcode, request.object.unwrap_or(0))
            .map_err(|error| request.error(IoStage::Submit, IoCause::Ipc(error)))?
            .wait()
            .map_err(|error| request.error(IoStage::Wait, IoCause::Ipc(error)))
    }

    fn call_move(
        &self,
        request: Request,
        opcode: u32,
        memory: OwnedMemory,
    ) -> Result<CallResult, IoError> {
        self.connection
            .call_move(opcode, request.object.unwrap_or(0), memory)
            .map_err(|(_, error)| request.error(IoStage::Submit, IoCause::Ipc(error)))?
            .wait()
            .map_err(|error| request.error(IoStage::Wait, IoCause::Ipc(error)))
    }
}

impl ObjectStore for Client {
    fn read(&self, object: u64) -> Result<Option<Vec<u8>>, IoError> {
        let request = Request {
            operation: IoOperation::Read,
            object: Some(object),
        };
        let reply = self.call(request, crate::objstore::OP_READ)?;
        let Some(size) =
            request.read_size(reply.result, reply.memory.as_ref().map(OwnedMemory::len))?
        else {
            return Ok(None);
        };
        let memory = reply.memory.expect("validated read attachment");
        let mapping = memory
            .map_read_only()
            .map_err(|(_, error)| request.error(IoStage::Map, IoCause::Memory(error)))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| request.error(IoStage::Copy, IoCause::AllocationFailed))?;
        bytes.extend_from_slice(&mapping.as_slice()[..size]);
        Ok(Some(bytes))
    }

    fn create(&self, object: u64) -> Result<(), IoError> {
        let request = Request {
            operation: IoOperation::Create,
            object: Some(object),
        };
        request.status(self.call(request, crate::objstore::OP_CREATE_AT)?.result)
    }

    fn write(&self, object: u64, bytes: &[u8]) -> Result<(), IoError> {
        let prepared = PreparedWrite::new(object, bytes)?;
        let size = Request {
            operation: IoOperation::SetSize,
            object: Some(object),
        };
        size.status(self.call_move(size, crate::objstore::OP_SET_SIZE, prepared.size)?.result)?;
        let payload = Request {
            operation: IoOperation::Write,
            object: Some(object),
        };
        payload.status(self.call_move(payload, crate::objstore::OP_WRITE, prepared.payload)?.result)
    }

    fn flush(&self) -> Result<(), IoError> {
        let request = Request {
            operation: IoOperation::Flush,
            object: None,
        };
        request.status(self.call(request, crate::objstore::OP_FLUSH)?.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_rejections_retain_signed_status_and_request_context() {
        for operation in
            [IoOperation::Create, IoOperation::SetSize, IoOperation::Write, IoOperation::Flush]
        {
            let request = Request {
                operation,
                object: if operation == IoOperation::Flush {
                    None
                } else {
                    Some(u64::MAX)
                },
            };
            assert_eq!(request.status(crate::objstore::ERR_OK), Ok(()));
            for status in
                [crate::objstore::ERR_IO_ERROR, crate::objstore::ERR_DENIED, i64::MIN, i64::MAX]
            {
                assert_eq!(
                    request.status(status),
                    Err(IoError {
                        operation,
                        object: request.object,
                        stage: IoStage::Reply,
                        cause: IoCause::RemoteStatus(status),
                    })
                );
            }
        }
    }

    #[test]
    fn read_absence_never_hides_other_status_or_invalid_memory_length() {
        let request = Request {
            operation: IoOperation::Read,
            object: Some(17),
        };
        assert_eq!(request.read_size(crate::objstore::ERR_NOT_FOUND, None), Ok(None));
        // The same small result with memory is a length, not an absent object.
        assert_eq!(request.read_size(crate::objstore::ERR_NOT_FOUND, Some(4096)), Ok(Some(1)));
        assert_eq!(request.read_size(0, Some(4096)), Ok(Some(0)));
        assert_eq!(request.read_size(4096, Some(4096)), Ok(Some(4096)));
        for (result, memory_bytes) in
            [(crate::objstore::ERR_IO_ERROR, None), (0, None), (-1, Some(4096)), (4097, Some(4096))]
        {
            assert_eq!(
                request.read_size(result, memory_bytes),
                Err(IoError {
                    operation: IoOperation::Read,
                    object: Some(17),
                    stage: IoStage::Reply,
                    cause: IoCause::InvalidReadReply {
                        result,
                        memory_bytes
                    },
                })
            );
        }
    }
}
