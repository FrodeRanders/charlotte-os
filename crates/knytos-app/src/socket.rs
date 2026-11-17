//! Owned socket operations over an attenuated TCP/IP service connection.

use catten_rt::owned::{
    CallResult,
    ConnectionRef,
    IpcError,
    MemoryError,
    OwnedMemory,
    PendingCall,
};

pub const INTERFACE: u64 = super::name(b"SKT");
pub const VERSION: u32 = 1;
pub const NAME: u64 = super::name(b"tcpip");

pub const OP_SOCKET: u32 = 1;
/// Select a remote IPv4/port from a six-byte moved memory object. TCP
/// begins its handshake; UDP binds an ephemeral local port and retains
/// the remote endpoint for subsequent send/receive filtering.
pub const OP_CONNECT: u32 = 2;
/// Bind TCP or UDP to the little-endian port in a moved memory object.
pub const OP_BIND: u32 = 3;
pub const OP_LISTEN: u32 = 4;
/// Observe the current accept state without retaining a waiter.
pub const OP_TRY_ACCEPT: u32 = 5;
/// Immediate TCP/UDP send: full socket ID in arg0 and exact CSOCKTX1 page.
/// Attempts enqueue once without retaining a waiter; no packed fallback.
pub const OP_TRY_SEND: u32 = 6;
/// Move one writable `REPLY_BUFFER_BYTES` buffer into the call. Reply
/// returns that buffer with the byte count, or a scalar error/EOF. The
/// original allocating principal remains its backing sponsor.
pub const OP_RECV: u32 = 7;
/// Full socket ID; reply is CLOSE_STARTED or CLOSE_UNAVAILABLE, without attachments.
pub const OP_CLOSE: u32 = 8;
pub const CLOSE_STARTED: i64 = 0;
pub const CLOSE_UNAVAILABLE: i64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseReceipt {
    Started,
    Unavailable,
}
impl CloseReceipt {
    pub fn decode(result: i64) -> Option<Self> {
        match result {
            CLOSE_STARTED => Some(Self::Started),
            CLOSE_UNAVAILABLE => Some(Self::Unavailable),
            _ => None,
        }
    }
}
pub const OP_FRAME: u32 = 9;
/// Move one writable `REPLY_BUFFER_BYTES` buffer into the call. Reply
/// returns it holding the packed `STATUS_*` snapshot.
pub const OP_STATUS: u32 = 10;
pub const REPLY_BUFFER_BYTES: usize = 4096;

/// Receive/status never allocate service-sponsored reply backing. The
/// caller moves its own buffer; submission rejection returns its owner.
pub fn buffered_call(
    service: ConnectionRef<'_>,
    opcode: u32,
    arg0: u64,
) -> Result<PendingCall<'static>, SocketError> {
    let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
    service.call_move(opcode, arg0, memory).map_err(|(_, error)| SocketError::Ipc(error))
}
/// Cancel this socket's deferred `OP_RECV`, if any. The receive call is
/// completed with `ERR_WOULD_BLOCK` before this request is acknowledged,
/// allowing a bounded client wait to release the server-side reply slot
/// without closing the socket.
pub const OP_CANCEL_RECV: u32 = 11;
/// Query one socket's connection state without consuming it. The scalar
/// reply is one of `CONNECTION_STATE_*`; this lets event loops distinguish
/// an accepted nonblocking connect request from an established TCP flow.
pub const OP_CONNECTION_STATE: u32 = 12;
/// Bind TCP or UDP to one exact IPv4 address and port from a six-byte
/// moved memory object. This permits equal ports on distinct service VIPs.
pub const OP_BIND_IPV4: u32 = 13;
/// Listen for TCP on one exact IPv4 address and port. The payload matches
/// [`OP_BIND_IPV4`].
pub const OP_LISTEN_IPV4: u32 = 14;
/// Retain one complete call until this TCP listener establishes a peer or
/// closes. A successful scalar reply is zero. Bind its PendingCall to a
/// CQ to combine accept readiness with other socket/deadline interests.
pub const OP_ACCEPT: u32 = 15;
/// Deferred TCP send: full socket ID in arg0, exact CSOCKTX1 moved page.
/// Retains caller-sponsored backing until enqueue progress or terminal
/// socket state. A positive scalar acknowledges exactly that byte prefix.
pub const OP_SEND: u32 = 16;

pub const CONNECTION_STATE_CLOSED: i64 = 0;
pub const CONNECTION_STATE_CONNECTING: i64 = 1;
pub const CONNECTION_STATE_ESTABLISHED: i64 = 2;
pub const CONNECTION_STATE_CLOSING: i64 = 3;

/// `TcpipStatus` snapshot layout (all little-endian u32 words in a moved
/// page). Offsets are in u32 words from the base of the reply page. Words
/// 0..=4 are the original v1 layout; the remaining words extend it without
/// disturbing existing readers, which only consume the leading words.
pub const STATUS_OFFSET_IP: u32 = 0;
pub const STATUS_OFFSET_RX_FRAMES: u32 = 1;
pub const STATUS_OFFSET_TX_SENDS: u32 = 2;
pub const STATUS_OFFSET_SOCKETS: u32 = 3;
pub const STATUS_OFFSET_MAGIC: u32 = 4;
/// Transmit calls that failed (send buffer full or socket error).
pub const STATUS_OFFSET_TX_SEND_ERRORS: u32 = 5;
/// Address-acquisition mode: 0 = static, 1 = DHCP.
pub const STATUS_OFFSET_DHCP_MODE: u32 = 6;
/// Default-route gateway in network order, 0 when none is configured.
pub const STATUS_OFFSET_GATEWAY: u32 = 7;
/// Interface MTU in bytes.
pub const STATUS_OFFSET_MTU: u32 = 8;
/// Effective SocketSet slots after heap-based resource clamping, including
/// the reserved DHCP slot when active.
pub const STATUS_OFFSET_SOCKET_CAPACITY: u32 = 9;
/// Per-principal socket quota from the launch policy.
pub const STATUS_OFFSET_SOCKET_QUOTA: u32 = 10;
/// Per-principal buffer quota in bytes from the launch policy.
pub const STATUS_OFFSET_BUFFER_QUOTA: u32 = 11;
/// Requested socket-table ceiling before heap-based clamping.
pub const STATUS_OFFSET_SOCKET_CEILING: u32 = 12;
pub const STATUS_WORDS: usize = 13;
pub const STATUS_MAGIC: u32 = 0x5443_5053;

/// Shared SocketSet capacity (after reserving DHCP, when enabled) is full.
pub const ERR_TOO_MANY_SOCKETS: i64 = -1;
pub const ERR_BAD_SOCKET: i64 = -2;
pub const ERR_CONNECTION_REFUSED: i64 = -3;
pub const ERR_NOT_CONNECTED: i64 = -4;
pub const ERR_WOULD_BLOCK: i64 = -5;
pub const ERR_BAD_DOMAIN: i64 = -6;
pub const ERR_BAD_OPCODE: i64 = -7;
/// The caller's per-principal socket or buffer budget is exhausted.
pub const ERR_QUOTA_EXCEEDED: i64 = -8;
pub const ERR_ID_EXHAUSTED: i64 = -9;

pub const DOMAIN_TCP: u64 = 1;
pub const DOMAIN_UDP: u64 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketError {
    Ipc(IpcError),
    Memory(MemoryError),
    Service(i64),
    InvalidReply,
    RetryExhausted,
}

impl From<IpcError> for SocketError {
    fn from(value: IpcError) -> Self {
        Self::Ipc(value)
    }
}

fn send_memory(bytes: &[u8]) -> Result<OwnedMemory, SocketError> {
    if bytes.is_empty() || bytes.len() > super::socket_send::MAX_PAYLOAD {
        return Err(SocketError::InvalidReply);
    }
    let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
    let mut mapping = memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
    if !super::socket_send::encode(bytes, mapping.as_mut_slice()) {
        return Err(SocketError::InvalidReply);
    }
    mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))
}

/// A receive result whose memory remains owned until explicitly moved or
/// dropped. Only `len` bytes at the start of the page are initialized.
#[must_use = "dropping a received socket chunk releases its memory"]
pub struct ReceivedChunk {
    memory: OwnedMemory,
    len: usize,
}

impl ReceivedChunk {
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

/// The remote selector and its original service incarnation move together.
/// Only the owning socket/close wrappers can construct or consume this lease.
struct SocketLease<'connection> {
    service: ConnectionRef<'connection>,
    id: u64,
}

/// A socket owned through its original TCP/IP connection. Explicit close and
/// best-effort Drop consume the same close owner once. Rejected admission
/// returns an error with terminal retention; accepted uncertainty aborts.
#[must_use = "dropping a socket asks the TCP/IP service to close it"]
pub struct OwnedSocket<'connection> {
    lease: Option<SocketLease<'connection>>,
}

/// An accepted creation request can already own an unobserved remote socket.
/// Retain its original connection until a checked reply transfers that socket
/// or confirms rejection. Incomplete destruction aborts without replay.
#[must_use = "observe socket creation before releasing its service connection"]
pub struct OpeningSocket<'connection> {
    pending: Option<PendingCall<'static>>,
    service: Option<ConnectionRef<'connection>>,
}

impl<'connection> OpeningSocket<'connection> {
    pub fn bind_cq(&mut self, cq: u32) -> Result<(), IpcError> {
        self.pending.as_mut().expect("accepted creation").bind_cq(cq)
    }

    /// Service rejection is terminal and disarms this owner. Malformed or
    /// failed observation retains it, since remote creation is uncertain.
    pub fn poll(&mut self) -> Result<Option<OwnedSocket<'connection>>, SocketError> {
        let Some(reply) = self.pending.as_mut().expect("accepted creation").poll()? else {
            return Ok(None);
        };
        if reply.memory.is_some() || reply.connection.is_some() || reply.result == 0 {
            return Err(SocketError::InvalidReply);
        }
        drop(self.pending.take());
        let service = self.service.take().expect("unobserved creation");
        if reply.result < 0 {
            return Err(SocketError::Service(reply.result));
        }
        Ok(Some(OwnedSocket {
            lease: Some(SocketLease {
                service,
                id: reply.result as u64,
            }),
        }))
    }
}

impl Drop for OpeningSocket<'_> {
    fn drop(&mut self) {
        if self.service.is_some() {
            catten_rt::domain_abort();
        }
    }
}

/// Consuming logical-close operation shared by CQ and synchronous callers.
/// Incomplete Drop aborts before fields are released; it never issues a
/// second close. The synchronous driver retains an unsubmitted owner on
/// final admission rejection without invoking that destructor.
#[must_use = "drive logical close to a confirmed reply; incomplete Drop aborts"]
pub struct ClosingSocket<'connection> {
    pending: Option<PendingCall<'static>>,
    lease: Option<SocketLease<'connection>>,
}

impl ClosingSocket<'_> {
    /// Try admission once. Only a rejected submission may be attempted again.
    pub fn submit(&mut self) -> Result<(), IpcError> {
        assert!(self.pending.is_none(), "close already submitted");
        let lease = self.lease.as_ref().expect("socket already closed");
        self.pending = Some(lease.service.call(OP_CLOSE, lease.id)?);
        Ok(())
    }

    pub fn bind_cq(&mut self, cq: u32) -> Result<(), IpcError> {
        self.pending.as_mut().expect("accepted close").bind_cq(cq)
    }

    fn observe(&mut self, reply: Option<CallResult>) -> Result<Option<CloseReceipt>, SocketError> {
        let Some(reply) = reply else {
            return Ok(None);
        };
        if reply.memory.is_some() || reply.connection.is_some() {
            return Err(SocketError::InvalidReply);
        }
        let receipt = CloseReceipt::decode(reply.result).ok_or({
            if reply.result < 0 {
                SocketError::Service(reply.result)
            } else {
                SocketError::InvalidReply
            }
        })?;
        drop(self.pending.take());
        self.lease = None;
        Ok(Some(receipt))
    }

    /// A typed logical receipt ends this owner. Unavailable grants no
    /// former-ownership or physical cleanup proof. Rejection retains it.
    pub fn poll(&mut self) -> Result<Option<CloseReceipt>, SocketError> {
        let reply =
            self.pending.as_mut().expect("accepted close").poll().map_err(SocketError::Ipc)?;
        self.observe(reply)
    }

    /// Wait for this accepted call while retaining the entire owner through
    /// timeout or rejected wait admission. Neither outcome permits replay.
    pub fn wait_timeout(&mut self, timeout_ms: u64) -> Result<Option<CloseReceipt>, SocketError> {
        let reply = self
            .pending
            .as_mut()
            .expect("accepted close")
            .wait_timeout(timeout_ms)
            .map_err(SocketError::Ipc)?;
        self.observe(reply)
    }
}

impl Drop for ClosingSocket<'_> {
    fn drop(&mut self) {
        if self.lease.is_some() {
            catten_rt::domain_abort();
        }
    }
}

impl<'connection> OwnedSocket<'connection> {
    pub fn open_call(
        service: ConnectionRef<'connection>,
        domain: u64,
    ) -> Result<OpeningSocket<'connection>, IpcError> {
        Ok(OpeningSocket {
            pending: Some(service.call(OP_SOCKET, domain)?),
            service: Some(service),
        })
    }

    pub fn open(service: ConnectionRef<'connection>, domain: u64) -> Result<Self, IpcError> {
        let result = service.call(OP_SOCKET, domain)?.wait()?;
        if result.result < 1 {
            return Err(IpcError::Status(result.result as u64));
        }
        Ok(Self {
            lease: Some(SocketLease {
                service,
                id: result.result as u64,
            }),
        })
    }

    pub fn id(&self) -> u64 {
        self.lease.as_ref().expect("socket already closed").id
    }

    fn service(&self) -> ConnectionRef<'connection> {
        self.lease.as_ref().expect("socket already closed").service
    }

    pub fn call(&self, opcode: u32, arg0: u64) -> Result<PendingCall<'static>, IpcError> {
        self.service().call(opcode, arg0)
    }

    pub fn accept_call(&self) -> Result<PendingCall<'static>, IpcError> {
        self.service().call(OP_ACCEPT, self.id())
    }

    /// Return the protocol-neutral connection state exposed by tcpip.
    pub fn connection_state(&self) -> Result<i64, SocketError> {
        self.service()
            .call(OP_CONNECTION_STATE, self.id())
            .map_err(SocketError::Ipc)?
            .wait()
            .map(|result| result.result)
            .map_err(SocketError::Ipc)
    }

    /// Select an IPv4 peer and begin connecting. TCP handshaking proceeds
    /// in the tcpip reactor; [`send_all`](Self::send_all) handles its
    /// transient `ERR_WOULD_BLOCK` result.
    pub fn connect_ipv4(&self, address: [u8; 4], port: u16) -> Result<(), SocketError> {
        let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
        let mut mapping = memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
        mapping.as_mut_slice()[..4].copy_from_slice(&address);
        mapping.as_mut_slice()[4..6].copy_from_slice(&port.to_le_bytes());
        let memory = mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))?;
        let result = self
            .service()
            .call_move(OP_CONNECT, self.id(), memory)
            .map_err(|(_, error)| SocketError::Ipc(error))?
            .wait()
            .map_err(SocketError::Ipc)?
            .result;
        if result == 0 {
            Ok(())
        } else {
            Err(SocketError::Service(result))
        }
    }

    fn local_ipv4_operation(
        &self,
        opcode: u32,
        address: [u8; 4],
        port: u16,
    ) -> Result<(), SocketError> {
        let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
        let mut mapping = memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
        mapping.as_mut_slice()[..4].copy_from_slice(&address);
        mapping.as_mut_slice()[4..6].copy_from_slice(&port.to_le_bytes());
        let memory = mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))?;
        let result = self
            .service()
            .call_move(opcode, self.id(), memory)
            .map_err(|(_, error)| SocketError::Ipc(error))?
            .wait()
            .map_err(SocketError::Ipc)?
            .result;
        if result == 0 {
            Ok(())
        } else {
            Err(SocketError::Service(result))
        }
    }

    pub fn bind_ipv4(&self, address: [u8; 4], port: u16) -> Result<(), SocketError> {
        self.local_ipv4_operation(OP_BIND_IPV4, address, port)
    }

    /// Listen on any local address using the port-only protocol operation.
    /// This borrows the socket owner through setup and consumes its moved
    /// request backing exactly once; rejection leaves the socket owned.
    pub fn listen(&self, port: u16) -> Result<(), SocketError> {
        let result = self.listen_call(port)?.wait().map_err(SocketError::Ipc)?.result;
        if result == 0 {
            Ok(())
        } else {
            Err(SocketError::Service(result))
        }
    }

    /// Submit setup once while the containing operation retains this socket.
    /// CQ readiness and reply observation belong to that same operation.
    pub fn listen_call(&self, port: u16) -> Result<PendingCall<'static>, SocketError> {
        let memory = OwnedMemory::allocate(1).map_err(SocketError::Memory)?;
        let mut mapping = memory.map_writable().map_err(|(_, error)| SocketError::Memory(error))?;
        mapping.as_mut_slice()[..2].copy_from_slice(&port.to_le_bytes());
        let memory = mapping.unmap().map_err(|(_, error)| SocketError::Memory(error))?;
        self.service()
            .call_move(OP_LISTEN, self.id(), memory)
            .map_err(|(_, error)| SocketError::Ipc(error))
    }

    pub fn listen_ipv4(&self, address: [u8; 4], port: u16) -> Result<(), SocketError> {
        self.local_ipv4_operation(OP_LISTEN_IPV4, address, port)
    }

    /// Send the entire byte slice as one or more moved pages, handling
    /// partial acceptance and tcpip backpressure without exposing socket
    /// IDs or cleanup work to the caller.
    pub fn send_all(
        &self,
        bytes: &[u8],
        attempts: usize,
        retry_ms: u64,
    ) -> Result<(), SocketError> {
        let mut offset = 0usize;
        for _ in 0..attempts {
            if offset == bytes.len() {
                return Ok(());
            }
            let chunk_len = (bytes.len() - offset).min(super::socket_send::MAX_PAYLOAD);
            let result = self
                .try_send_call(&bytes[offset..offset + chunk_len])?
                .wait()
                .map_err(SocketError::Ipc)?
                .result;
            if result > 0 && result as usize <= chunk_len {
                offset += result as usize;
            } else if result == ERR_WOULD_BLOCK || result == 0 {
                super::sleep_ms(retry_ms);
            } else if result > chunk_len as i64 {
                return Err(SocketError::InvalidReply);
            } else {
                return Err(SocketError::Service(result));
            }
        }
        Err(SocketError::RetryExhausted)
    }

    /// Submit one immediate TCP/UDP enqueue attempt with the full socket
    /// identity and caller-owned frame. Acceptance must be observed once;
    /// timeout or observation failure cannot authorize replay.
    pub fn try_send_call(&self, bytes: &[u8]) -> Result<PendingCall<'static>, SocketError> {
        self.service()
            .call_move(OP_TRY_SEND, self.id(), send_memory(bytes)?)
            .map_err(|(_, error)| SocketError::Ipc(error))
    }

    /// Submit one deferred TCP send. The moved page contains an exact
    /// frame; no Rust memory borrow survives submission. Observe the
    /// positive prefix receipt before submitting any remaining bytes.
    pub fn send_call(
        &self,
        bytes: &[u8],
        deadline: &super::deadline::Deadline,
    ) -> Result<PendingCall<'static>, SocketError> {
        if deadline.expired() {
            return Err(SocketError::RetryExhausted);
        }
        let memory = send_memory(bytes)?;
        if deadline.expired() {
            return Err(SocketError::RetryExhausted);
        }
        self.service()
            .call_move(OP_SEND, self.id(), memory)
            .map_err(|(_memory, error)| SocketError::Ipc(error))
    }

    /// Submit a receive using this caller's backing, without waiting.
    pub fn receive_call(&self) -> Result<PendingCall<'static>, SocketError> {
        buffered_call(self.service(), OP_RECV, self.id())
    }

    /// Wait for and own the receive page supplied by this caller.
    pub fn receive(&self) -> Result<Option<ReceivedChunk>, SocketError> {
        let result = self.receive_call()?.wait().map_err(SocketError::Ipc)?;
        Self::decode_receive(result)
    }

    /// Poll for one receive page, returning `RetryExhausted` after a
    /// bounded wait. On timeout this explicitly cancels tcpip's retained
    /// receive slot before releasing the local pending-call capability.
    pub fn receive_timeout(
        &self,
        attempts: usize,
        retry_ms: u64,
    ) -> Result<Option<ReceivedChunk>, SocketError> {
        let mut pending = self.receive_call()?;
        for _ in 0..attempts {
            if let Some(result) = pending.poll().map_err(SocketError::Ipc)? {
                return Self::decode_receive(result);
            }
            super::sleep_ms(retry_ms);
        }
        let cancel =
            self.service().call(OP_CANCEL_RECV, self.id())?.wait().map_err(SocketError::Ipc)?;
        if cancel.result != 0 {
            return Err(SocketError::Service(cancel.result));
        }
        // tcpip processes messages from this connection in order. If the
        // reactor delivered bytes before observing the cancellation, keep
        // them; otherwise OP_CANCEL_RECV completes this call with the
        // distinguished timeout result.
        let result = pending.wait().map_err(SocketError::Ipc)?;
        if result.result == ERR_WOULD_BLOCK
            && result.connection.is_none()
            && result.memory.is_none()
        {
            Err(SocketError::RetryExhausted)
        } else {
            Self::decode_receive(result)
        }
    }

    pub fn decode_receive(result: CallResult) -> Result<Option<ReceivedChunk>, SocketError> {
        if result.connection.is_some() {
            return Err(SocketError::InvalidReply);
        }
        if result.result < 0 {
            return Err(SocketError::Service(result.result));
        }
        if result.result == 0 {
            return if result.memory.is_none() {
                Ok(None)
            } else {
                Err(SocketError::InvalidReply)
            };
        }
        let memory = result.memory.ok_or(SocketError::InvalidReply)?;
        let len = result.result as usize;
        if len > memory.len() {
            return Err(SocketError::InvalidReply);
        }
        Ok(Some(ReceivedChunk {
            memory,
            len,
        }))
    }

    /// Transfer this exact socket into an asynchronous logical-close owner.
    pub fn into_closing(mut self) -> ClosingSocket<'connection> {
        self.take_closing().expect("socket already closed")
    }

    fn take_closing(&mut self) -> Option<ClosingSocket<'connection>> {
        self.lease.take().map(|lease| ClosingSocket {
            pending: None,
            lease: Some(lease),
        })
    }

    pub fn close(mut self) -> Result<(), IpcError> {
        crate::socket_close::finish(self.take_closing().expect("socket already closed"))
    }
}

impl Drop for OwnedSocket<'_> {
    fn drop(&mut self) {
        if let Some(owner) = self.take_closing() {
            let _ = crate::socket_close::finish(owner);
        }
    }
}

/// Shared close policy, also used by platform reactors.
pub const CLOSE_MS: u64 = crate::socket_close::CLOSE_MS;
pub const ADMISSION_BACKOFF_MS: u64 = crate::socket_close::ADMISSION_BACKOFF_MS;
