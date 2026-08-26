//! Owned application-side S3 service client.
//!
//! The service connection carries authority for one configured endpoint,
//! bucket, prefix, credential identity, and operation policy. Streaming
//! operation IDs are remote resources and therefore use consuming `close` /
//! `abort` methods plus best-effort `Drop` fallbacks.

use alloc::vec::Vec;

use catten_rt::owned::{
    ConnectionRef,
    IpcError,
    MemoryError,
    OwnedMemory,
};
use charlotte_protocol_s3::{
    self as protocol,
    ObjectMetadata,
    ObjectRequest,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Ipc(IpcError),
    Memory(MemoryError),
    InvalidRequest,
    InvalidReply,
    Service(i64),
}

impl From<IpcError> for Error {
    fn from(value: IpcError) -> Self {
        Self::Ipc(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectInfo {
    pub status: u16,
    pub content_length: u64,
    pub etag: Vec<u8>,
    pub version_id: Vec<u8>,
    pub request_id: Vec<u8>,
}

impl ObjectInfo {
    fn copy_from(metadata: ObjectMetadata<'_>) -> Self {
        Self {
            status: metadata.status,
            content_length: metadata.content_length,
            etag: metadata.etag.to_vec(),
            version_id: metadata.version_id.to_vec(),
            request_id: metadata.request_id.to_vec(),
        }
    }
}

/// A returned object-data page and its exact initialized length.
#[must_use = "dropping a chunk releases its memory capability"]
pub struct ObjectChunk {
    memory: OwnedMemory,
    len: usize,
}

impl ObjectChunk {
    pub fn new(memory: OwnedMemory, len: usize) -> Result<Self, (OwnedMemory, Error)> {
        if len == 0 || len > protocol::MAX_CHUNK_LEN || len > memory.len() {
            return Err((memory, Error::InvalidRequest));
        }
        Ok(Self {
            memory,
            len,
        })
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn memory(&self) -> &OwnedMemory {
        &self.memory
    }

    pub fn into_parts(self) -> (OwnedMemory, usize) {
        (self.memory, self.len)
    }
}

#[derive(Clone, Copy)]
pub struct Client<'connection> {
    service: ConnectionRef<'connection>,
}

impl<'connection> Client<'connection> {
    pub const fn new(service: ConnectionRef<'connection>) -> Self {
        Self {
            service,
        }
    }

    pub fn get(
        &self,
        request: ObjectRequest<'_>,
    ) -> Result<(GetObject<'connection>, ObjectInfo), Error> {
        let reply = request_call(self.service, protocol::OP_GET_BEGIN, request)?;
        if reply.result <= 0 {
            return Err(Error::Service(reply.result));
        }
        let operation = GetObject {
            service: self.service,
            id: Some(reply.result as u64),
            eof: false,
            failed: false,
        };
        let info = decode_info(reply.memory.ok_or(Error::InvalidReply)?)?;
        Ok((operation, info))
    }

    pub fn put(&self, request: ObjectRequest<'_>) -> Result<PutObject<'connection>, Error> {
        let reply = request_call(self.service, protocol::OP_PUT_BEGIN, request)?;
        if reply.result <= 0 {
            return Err(Error::Service(reply.result));
        }
        let operation = PutObject {
            service: self.service,
            id: Some(reply.result as u64),
            written: 0,
            expected: request.content_length,
        };
        if reply.memory.is_some() {
            return Err(Error::InvalidReply);
        }
        Ok(operation)
    }

    pub fn head(&self, request: ObjectRequest<'_>) -> Result<ObjectInfo, Error> {
        let reply = request_call(self.service, protocol::OP_HEAD, request)?;
        if reply.result < 0 {
            return Err(Error::Service(reply.result));
        }
        decode_info(reply.memory.ok_or(Error::InvalidReply)?)
    }

    pub fn delete(&self, request: ObjectRequest<'_>) -> Result<(), Error> {
        let reply = request_call(self.service, protocol::OP_DELETE, request)?;
        if reply.result == 0 && reply.memory.is_none() {
            Ok(())
        } else if reply.result < 0 {
            Err(Error::Service(reply.result))
        } else {
            Err(Error::InvalidReply)
        }
    }
}

fn request_call(
    service: ConnectionRef<'_>,
    opcode: u32,
    request: ObjectRequest<'_>,
) -> Result<catten_rt::owned::CallResult, Error> {
    let memory = OwnedMemory::allocate(1).map_err(Error::Memory)?;
    let mut mapping = memory.map_writable().map_err(|(_, error)| Error::Memory(error))?;
    let len = request.encode(mapping.as_mut_slice()).ok_or(Error::InvalidRequest)?;
    let memory = mapping.unmap().map_err(|(_, error)| Error::Memory(error))?;
    service
        .call_move(opcode, len as u64, memory)
        .map_err(|(_, error)| Error::Ipc(error))?
        .wait()
        .map_err(Error::Ipc)
}

fn decode_info(memory: OwnedMemory) -> Result<ObjectInfo, Error> {
    let mapping = memory.map_read_only().map_err(|(_, error)| Error::Memory(error))?;
    let metadata = ObjectMetadata::decode(mapping.as_slice()).ok_or(Error::InvalidReply)?;
    Ok(ObjectInfo::copy_from(metadata))
}

#[must_use = "dropping a GET operation closes it at the S3 service"]
pub struct GetObject<'connection> {
    service: ConnectionRef<'connection>,
    id: Option<u64>,
    eof: bool,
    failed: bool,
}

impl GetObject<'_> {
    pub fn read(&mut self) -> Result<Option<ObjectChunk>, Error> {
        if self.eof {
            return Ok(None);
        }
        if self.failed {
            return Err(Error::InvalidRequest);
        }
        let id = self.id.ok_or(Error::InvalidRequest)?;
        let pending = self.service.call(protocol::OP_GET_READ, id)?;
        let reply = match pending.wait() {
            Ok(reply) => reply,
            Err(error) => {
                self.failed = true;
                return Err(Error::Ipc(error));
            }
        };
        if reply.result < 0 {
            self.failed = true;
            return Err(Error::Service(reply.result));
        }
        if reply.result == 0 {
            if reply.memory.is_some() {
                self.failed = true;
                return Err(Error::InvalidReply);
            }
            self.eof = true;
            return Ok(None);
        }
        let Some(memory) = reply.memory else {
            self.failed = true;
            return Err(Error::InvalidReply);
        };
        match ObjectChunk::new(memory, reply.result as usize) {
            Ok(chunk) => Ok(Some(chunk)),
            Err((_, error)) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    pub fn close(mut self) -> Result<(), Error> {
        self.close_inner()
    }

    fn close_inner(&mut self) -> Result<(), Error> {
        let Some(id) = self.id.take() else {
            return Ok(());
        };
        let call = match self.service.call(protocol::OP_GET_CLOSE, id) {
            Ok(call) => call,
            Err(error) => {
                self.id = Some(id);
                return Err(Error::Ipc(error));
            }
        };
        let result = match call.wait() {
            Ok(result) => result.result,
            Err(error) => {
                self.id = Some(id);
                return Err(Error::Ipc(error));
            }
        };
        if result == 0 {
            Ok(())
        } else {
            self.id = Some(id);
            Err(Error::Service(result))
        }
    }
}

impl Drop for GetObject<'_> {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}

#[must_use = "a failed PUT chunk submission returns ownership of the memory"]
pub enum PutWriteError {
    NotSubmitted {
        memory: OwnedMemory,
        len: usize,
        error: IpcError,
    },
    Failed(Error),
}

#[must_use = "dropping a PUT operation aborts it at the S3 service"]
pub struct PutObject<'connection> {
    service: ConnectionRef<'connection>,
    id: Option<u64>,
    written: u64,
    expected: u64,
}

impl PutObject<'_> {
    pub const fn bytes_written(&self) -> u64 {
        self.written
    }

    pub fn write(&mut self, chunk: ObjectChunk) -> Result<usize, PutWriteError> {
        let id = self.id.ok_or(PutWriteError::Failed(Error::InvalidRequest))?;
        let (memory, len) = chunk.into_parts();
        if id > u32::MAX as u64 || self.written.saturating_add(len as u64) > self.expected {
            return Err(PutWriteError::NotSubmitted {
                memory,
                len,
                error: IpcError::Status(protocol::ERR_INVALID as u64),
            });
        }
        let packed = ((len as u64) << 32) | id;
        let pending = self.service.call_move(protocol::OP_PUT_WRITE, packed, memory).map_err(
            |(memory, error)| PutWriteError::NotSubmitted {
                memory,
                len,
                error,
            },
        )?;
        let result = match pending.wait() {
            Ok(reply) => reply.result,
            Err(error) => {
                let _ = self.abort_inner();
                return Err(PutWriteError::Failed(Error::Ipc(error)));
            }
        };
        if result < 0 {
            let _ = self.abort_inner();
            return Err(PutWriteError::Failed(Error::Service(result)));
        }
        if result as usize != len {
            let _ = self.abort_inner();
            return Err(PutWriteError::Failed(Error::InvalidReply));
        }
        self.written = self.written.saturating_add(result as u64);
        Ok(result as usize)
    }

    pub fn finish(mut self) -> Result<ObjectInfo, Error> {
        if self.written != self.expected {
            return Err(Error::InvalidRequest);
        }
        let id = self.id.take().ok_or(Error::InvalidRequest)?;
        let call = match self.service.call(protocol::OP_PUT_FINISH, id) {
            Ok(call) => call,
            Err(error) => {
                self.id = Some(id);
                return Err(Error::Ipc(error));
            }
        };
        let reply = match call.wait() {
            Ok(reply) => reply,
            Err(error) => {
                self.id = Some(id);
                return Err(Error::Ipc(error));
            }
        };
        if reply.result < 0 {
            return Err(Error::Service(reply.result));
        }
        decode_info(reply.memory.ok_or(Error::InvalidReply)?)
    }

    pub fn abort(mut self) -> Result<(), Error> {
        self.abort_inner()
    }

    fn abort_inner(&mut self) -> Result<(), Error> {
        let Some(id) = self.id.take() else {
            return Ok(());
        };
        let call = match self.service.call(protocol::OP_PUT_ABORT, id) {
            Ok(call) => call,
            Err(error) => {
                self.id = Some(id);
                return Err(Error::Ipc(error));
            }
        };
        let result = match call.wait() {
            Ok(result) => result.result,
            Err(error) => {
                self.id = Some(id);
                return Err(Error::Ipc(error));
            }
        };
        if result == 0 {
            Ok(())
        } else {
            self.id = Some(id);
            Err(Error::Service(result))
        }
    }
}

impl Drop for PutObject<'_> {
    fn drop(&mut self) {
        let _ = self.abort_inner();
    }
}
