//! Owned application-side client for the CharlotteOS Kafka service.
//!
//! Broker sockets, credentials, producer epochs, and sequence numbers remain
//! in `kafka.elf`. Applications own only service-scoped consumer, delivery,
//! and transaction resources. Every remote resource has an explicit
//! consuming teardown plus a best-effort `Drop` fallback.

use core::ops::Range;

use catten_rt::owned::{
    ConnectionRef,
    IpcError,
    MemoryError,
    OwnedMemory,
};
use charlotte_protocol_kafka::{
    self as protocol,
    DeliveredRecord,
    RecordRequest,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Ipc(IpcError),
    Memory(MemoryError),
    InvalidRequest,
    InvalidReply,
    Service(i64),
}

/// Index of a topic/partition route provisioned into this service endpoint.
///
/// Route zero is the profile's consume topic. Additional allow-listed produce
/// routes are numbered from one in launch-manifest order. Constructing a route
/// does not grant authority: the service rejects indices outside the endpoint's
/// immutable profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Route(u16);

impl Route {
    pub const DEFAULT: Self = Self(protocol::DEFAULT_ROUTE);

    pub const fn provisioned(index: u16) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u16 {
        self.0
    }
}

impl From<IpcError> for Error {
    fn from(value: IpcError) -> Self {
        Self::Ipc(value)
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

    pub fn produce(&self, record: RecordRequest<'_>) -> Result<i64, Error> {
        let reply = record_call(self.service, protocol::OP_PRODUCE, 0, record)?;
        if reply.result >= 0 && reply.memory.is_none() {
            Ok(reply.result)
        } else if reply.result < 0 {
            Err(Error::Service(reply.result))
        } else {
            Err(Error::InvalidReply)
        }
    }

    /// Produce to an allow-listed profile route.
    pub fn produce_to(&self, route: Route, record: RecordRequest<'_>) -> Result<i64, Error> {
        let reply = routed_record_call(self.service, protocol::OP_PRODUCE_TO, 0, route, record)?;
        if reply.result >= 0 && reply.memory.is_none() {
            Ok(reply.result)
        } else if reply.result < 0 {
            Err(Error::Service(reply.result))
        } else {
            Err(Error::InvalidReply)
        }
    }

    pub fn consumer(&self) -> Result<Consumer<'connection>, Error> {
        let reply = self.service.call(protocol::OP_CONSUMER_OPEN, 0)?.wait()?;
        if reply.result <= 0 {
            return Err(Error::Service(reply.result));
        }
        if reply.result > u32::MAX as i64 {
            return Err(Error::InvalidReply);
        }
        let consumer = Consumer {
            service: self.service,
            id: Some(reply.result as u32),
        };
        if reply.memory.is_some() {
            drop(consumer);
            return Err(Error::InvalidReply);
        }
        Ok(consumer)
    }

    pub fn begin_transaction(&self) -> Result<Transaction<'connection>, Error> {
        let reply = self.service.call(protocol::OP_TX_BEGIN, 0)?.wait()?;
        if reply.result <= 0 {
            return Err(Error::Service(reply.result));
        }
        if reply.result > u32::MAX as i64 {
            return Err(Error::InvalidReply);
        }
        let transaction = Transaction {
            service: self.service,
            id: Some(reply.result as u32),
            failed: false,
        };
        if reply.memory.is_some() {
            drop(transaction);
            return Err(Error::InvalidReply);
        }
        Ok(transaction)
    }
}

fn encode_record(record: RecordRequest<'_>) -> Result<(OwnedMemory, usize), Error> {
    let memory = OwnedMemory::allocate(1).map_err(Error::Memory)?;
    let mut mapping = memory.map_writable().map_err(|(_, error)| Error::Memory(error))?;
    let len = record.encode(mapping.as_mut_slice()).ok_or(Error::InvalidRequest)?;
    let memory = mapping.unmap().map_err(|(_, error)| Error::Memory(error))?;
    Ok((memory, len))
}

fn record_call(
    service: ConnectionRef<'_>,
    opcode: u32,
    resource_id: u32,
    record: RecordRequest<'_>,
) -> Result<catten_rt::owned::CallResult, Error> {
    let (memory, len) = encode_record(record)?;
    let arg0 = if resource_id == 0 {
        len as u64
    } else {
        ((len as u64) << 32) | u64::from(resource_id)
    };
    service
        .call_move(opcode, arg0, memory)
        .map_err(|(_, error)| Error::Ipc(error))?
        .wait()
        .map_err(Error::Ipc)
}

fn routed_record_call(
    service: ConnectionRef<'_>,
    opcode: u32,
    resource_id: u32,
    route: Route,
    record: RecordRequest<'_>,
) -> Result<catten_rt::owned::CallResult, Error> {
    let (memory, len) = encode_record(record)?;
    let arg0 = protocol::pack_routed_record_arg(resource_id, route.index(), len)
        .ok_or(Error::InvalidRequest)?;
    service
        .call_move(opcode, arg0, memory)
        .map_err(|(_, error)| Error::Ipc(error))?
        .wait()
        .map_err(Error::Ipc)
}

#[must_use = "dropping a consumer closes its service-side session"]
pub struct Consumer<'connection> {
    service: ConnectionRef<'connection>,
    id: Option<u32>,
}

impl<'connection> Consumer<'connection> {
    pub fn poll(&mut self) -> Result<Option<Delivery<'connection>>, Error> {
        let id = self.id.ok_or(Error::InvalidRequest)?;
        let reply = self.service.call(protocol::OP_CONSUMER_POLL, u64::from(id))?.wait()?;
        if reply.result == 0 && reply.memory.is_none() {
            return Ok(None);
        }
        if reply.result <= 0 {
            return Err(Error::Service(reply.result));
        }
        if reply.result > u32::MAX as i64 {
            return Err(Error::InvalidReply);
        }
        let mut token = DeliveryToken {
            service: self.service,
            id: Some(reply.result as u32),
        };
        let memory = reply.memory.ok_or(Error::InvalidReply)?;
        let mapping = memory.map_read_only().map_err(|(_, error)| Error::Memory(error))?;
        let record = DeliveredRecord::decode(mapping.as_slice()).ok_or(Error::InvalidReply)?;
        let info = DeliveryInfo {
            partition: record.partition,
            offset: record.offset,
            timestamp_ms: record.timestamp_ms,
            key: range_in_mapping(mapping.as_slice(), record.key),
            value: range_in_mapping(mapping.as_slice(), record.value),
        };
        let memory = mapping.unmap().map_err(|(_, error)| Error::Memory(error))?;
        Ok(Some(Delivery {
            token: DeliveryToken {
                service: token.service,
                id: token.id.take(),
            },
            memory: Some(memory),
            info,
        }))
    }

    pub fn close(mut self) -> Result<(), Error> {
        self.close_inner()
    }

    fn close_inner(&mut self) -> Result<(), Error> {
        let Some(id) = self.id.take() else {
            return Ok(());
        };
        match self.service.call(protocol::OP_CONSUMER_CLOSE, u64::from(id)) {
            Ok(call) => match call.wait() {
                Ok(reply) if reply.result == 0 => Ok(()),
                Ok(reply) => {
                    self.id = Some(id);
                    Err(Error::Service(reply.result))
                }
                Err(error) => {
                    self.id = Some(id);
                    Err(Error::Ipc(error))
                }
            },
            Err(error) => {
                self.id = Some(id);
                Err(Error::Ipc(error))
            }
        }
    }
}

impl Drop for Consumer<'_> {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryInfo {
    pub partition: i32,
    pub offset: i64,
    pub timestamp_ms: i64,
    key: Option<(usize, usize)>,
    value: Option<(usize, usize)>,
}

impl DeliveryInfo {
    pub fn key_range(&self) -> Option<Range<usize>> {
        self.key.map(|(start, len)| start..start + len)
    }

    pub fn value_range(&self) -> Option<Range<usize>> {
        self.value.map(|(start, len)| start..start + len)
    }
}

fn range_in_mapping(mapping: &[u8], value: Option<&[u8]>) -> Option<(usize, usize)> {
    value.map(|value| (value.as_ptr() as usize - mapping.as_ptr() as usize, value.len()))
}

#[must_use = "a delivery must be committed, included in a transaction, or released"]
pub struct Delivery<'connection> {
    token: DeliveryToken<'connection>,
    memory: Option<OwnedMemory>,
    info: DeliveryInfo,
}

impl Delivery<'_> {
    pub const fn info(&self) -> DeliveryInfo {
        self.info
    }
}

impl<'connection> Delivery<'connection> {
    pub fn into_parts(mut self) -> (DeliveryToken<'connection>, OwnedMemory, DeliveryInfo) {
        let token = DeliveryToken {
            service: self.token.service,
            id: self.token.id.take(),
        };
        let memory = self.memory.take().expect("delivery memory already taken");
        (token, memory, self.info)
    }
}

#[must_use = "dropping an uncommitted delivery token releases it for redelivery"]
pub struct DeliveryToken<'connection> {
    service: ConnectionRef<'connection>,
    id: Option<u32>,
}

impl DeliveryToken<'_> {
    pub fn commit(mut self) -> Result<(), Error> {
        self.finish(protocol::OP_DELIVERY_COMMIT)
    }

    fn finish(&mut self, opcode: u32) -> Result<(), Error> {
        let Some(id) = self.id.take() else {
            return Ok(());
        };
        match self.service.call(opcode, u64::from(id)) {
            Ok(call) => match call.wait() {
                Ok(reply) if reply.result == 0 => Ok(()),
                Ok(reply) => {
                    self.id = Some(id);
                    Err(Error::Service(reply.result))
                }
                Err(error) => {
                    self.id = Some(id);
                    Err(Error::Ipc(error))
                }
            },
            Err(error) => {
                self.id = Some(id);
                Err(Error::Ipc(error))
            }
        }
    }
}

impl Drop for DeliveryToken<'_> {
    fn drop(&mut self) {
        let _ = self.finish(protocol::OP_DELIVERY_RELEASE);
    }
}

#[must_use = "transactions must be committed or aborted"]
pub struct Transaction<'connection> {
    service: ConnectionRef<'connection>,
    id: Option<u32>,
    failed: bool,
}

impl Transaction<'_> {
    pub fn produce(&mut self, record: RecordRequest<'_>) -> Result<i64, Error> {
        if self.failed {
            return Err(Error::InvalidRequest);
        }
        let id = self.id.ok_or(Error::InvalidRequest)?;
        match record_call(self.service, protocol::OP_TX_PRODUCE, id, record) {
            Ok(reply) if reply.result >= 0 && reply.memory.is_none() => Ok(reply.result),
            Ok(reply) if reply.result < 0 => {
                self.failed = true;
                Err(Error::Service(reply.result))
            }
            Ok(_) => {
                self.failed = true;
                Err(Error::InvalidReply)
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    /// Produce within this transaction to an allow-listed profile route.
    pub fn produce_to(&mut self, route: Route, record: RecordRequest<'_>) -> Result<i64, Error> {
        if self.failed {
            return Err(Error::InvalidRequest);
        }
        let id = self.id.ok_or(Error::InvalidRequest)?;
        match routed_record_call(self.service, protocol::OP_TX_PRODUCE_TO, id, route, record) {
            Ok(reply) if reply.result >= 0 && reply.memory.is_none() => Ok(reply.result),
            Ok(reply) if reply.result < 0 => {
                self.failed = true;
                Err(Error::Service(reply.result))
            }
            Ok(_) => {
                self.failed = true;
                Err(Error::InvalidReply)
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    pub fn include(&mut self, mut delivery: DeliveryToken<'_>) -> Result<(), Error> {
        if self.failed {
            return Err(Error::InvalidRequest);
        }
        let transaction = self.id.ok_or(Error::InvalidRequest)?;
        let delivery_id = delivery.id.ok_or(Error::InvalidRequest)?;
        let arg0 = u64::from(transaction) | (u64::from(delivery_id) << 32);
        let result = match self.service.call(protocol::OP_TX_INCLUDE_DELIVERY, arg0) {
            Ok(call) => match call.wait() {
                Ok(result) => result,
                Err(error) => {
                    self.failed = true;
                    return Err(Error::Ipc(error));
                }
            },
            Err(error) => {
                self.failed = true;
                return Err(Error::Ipc(error));
            }
        };
        if result.result == 0 && result.memory.is_none() {
            delivery.id = None;
            Ok(())
        } else {
            self.failed = true;
            Err(Error::Service(result.result))
        }
    }

    pub fn commit(mut self) -> Result<(), Error> {
        if self.failed {
            return Err(Error::InvalidRequest);
        }
        self.finish(protocol::OP_TX_COMMIT)
    }

    pub fn abort(mut self) -> Result<(), Error> {
        self.finish(protocol::OP_TX_ABORT)
    }

    fn finish(&mut self, opcode: u32) -> Result<(), Error> {
        let Some(id) = self.id.take() else {
            return Ok(());
        };
        match self.service.call(opcode, u64::from(id)) {
            Ok(call) => match call.wait() {
                Ok(reply) if reply.result == 0 => Ok(()),
                Ok(reply) => {
                    self.id = Some(id);
                    Err(Error::Service(reply.result))
                }
                Err(error) => {
                    self.id = Some(id);
                    Err(Error::Ipc(error))
                }
            },
            Err(error) => {
                self.id = Some(id);
                Err(Error::Ipc(error))
            }
        }
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        let _ = self.finish(protocol::OP_TX_ABORT);
    }
}
