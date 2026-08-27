//! Verified TLS client transport over the capability-oriented socket service.
//!
//! The stream owns its socket and both TLS record buffers as one resource.
//! Construction requires an explicit DER trust anchor, the expected DNS name,
//! a synchronized Unix timestamp, and system entropy. There is no plaintext
//! fallback.

use alloc::{
    boxed::Box,
    vec,
    vec::Vec,
};
use core::{
    num::NonZeroU32,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use catten_rt::owned::ConnectionRef;
use embedded_io::{
    ErrorType,
    Read,
    Write as EmbeddedWrite,
};
use embedded_tls::{
    Aes128GcmSha256,
    Certificate,
    CryptoProvider,
    TlsClock,
    TlsConfig,
    TlsContext,
    TlsError,
    TlsVerifier,
    blocking::TlsConnection,
    pki::CertVerifier,
};
use rand_core::{
    CryptoRng,
    RngCore,
};

use crate::{
    entropy,
    socket,
};

const TLS_RECORD_BUFFER_LEN: usize = 16_640;
const TLS_CERTIFICATE_LEN: usize = 16_384;

static TLS_UNIX_SECONDS: AtomicU64 = AtomicU64::new(0);

/// Bounds used while adapting message-oriented CharlotteOS sockets to a byte
/// stream. A timeout is reported as a transport failure to embedded-tls.
#[derive(Clone, Copy)]
pub struct SocketBounds {
    pub send_attempts: usize,
    pub send_retry_ms: u64,
    pub receive_attempts: usize,
    pub receive_retry_ms: u64,
    pub receive_chunk_len: usize,
}

/// Inputs required to authenticate a TLS server.
pub struct OpenConfig<'a> {
    pub server_name: &'a str,
    pub ca_certificate_der: &'a [u8],
    pub unix_seconds: u64,
    pub socket_bounds: SocketBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenError {
    InvalidConfiguration,
    EntropyUnavailable,
    Handshake(u32),
}

/// A failure while using or explicitly closing an established TLS stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamError;

/// A verified TLS byte stream that owns all resources needed by the session.
#[must_use = "dropping the stream closes its TLS session and owned socket"]
pub struct OwnedTlsStream<'connection> {
    connection: Option<CharlotteTls<'connection>>,
    read_buffer: *mut [u8; TLS_RECORD_BUFFER_LEN],
    write_buffer: *mut [u8; TLS_RECORD_BUFFER_LEN],
    receive_chunk_len: usize,
}

impl<'connection> OwnedTlsStream<'connection> {
    pub fn open(
        socket: socket::OwnedSocket<'connection>,
        entropy_service: Option<ConnectionRef<'connection>>,
        config: OpenConfig<'_>,
    ) -> Result<Self, OpenError> {
        if config.server_name.is_empty()
            || config.ca_certificate_der.is_empty()
            || config.unix_seconds == 0
            || config.socket_bounds.send_attempts == 0
            || config.socket_bounds.receive_attempts == 0
            || config.socket_bounds.receive_chunk_len == 0
        {
            return Err(OpenError::InvalidConfiguration);
        }
        if (SystemRng {
            service: entropy_service,
        })
        .try_fill(&mut [0])
        .is_err()
        {
            return Err(OpenError::EntropyUnavailable);
        }

        TLS_UNIX_SECONDS.store(config.unix_seconds, Ordering::Relaxed);
        let read_buffer = Box::into_raw(Box::new([0; TLS_RECORD_BUFFER_LEN]));
        let write_buffer = Box::into_raw(Box::new([0; TLS_RECORD_BUFFER_LEN]));
        // SAFETY: these allocations remain exclusively owned by the wrapper,
        // are not moved, and are reclaimed only after `connection` is dropped.
        let read_ref: &'connection mut [u8] = unsafe { &mut *read_buffer };
        let write_ref: &'connection mut [u8] = unsafe { &mut *write_buffer };
        let io = SocketIo {
            socket,
            pending: Vec::new(),
            offset: 0,
            bounds: config.socket_bounds,
        };
        let mut connection = TlsConnection::new(io, read_ref, write_ref);
        let tls_config = TlsConfig::new().with_server_name(config.server_name);
        let provider = TlsProvider {
            rng: SystemRng {
                service: entropy_service,
            },
            verifier: CertVerifier::new(Certificate::X509(config.ca_certificate_der)),
        };
        if let Err(error) = connection.open(TlsContext::new(&tls_config, provider)) {
            let code = tls_error_code(&error);
            drop(connection);
            // SAFETY: the connection and all buffer borrows were dropped.
            unsafe {
                drop(Box::from_raw(read_buffer));
                drop(Box::from_raw(write_buffer));
            }
            return Err(OpenError::Handshake(code));
        }
        Ok(Self {
            connection: Some(connection),
            read_buffer,
            write_buffer,
            receive_chunk_len: config.socket_bounds.receive_chunk_len,
        })
    }

    pub fn send_all(&mut self, mut bytes: &[u8]) -> Result<(), StreamError> {
        let connection = self.connection.as_mut().ok_or(StreamError)?;
        while !bytes.is_empty() {
            let written = connection.write(bytes).map_err(|_| StreamError)?;
            if written == 0 {
                return Err(StreamError);
            }
            bytes = &bytes[written..];
        }
        connection.flush().map_err(|_| StreamError)
    }

    pub fn receive(&mut self) -> Result<Vec<u8>, StreamError> {
        let mut bytes = vec![0; self.receive_chunk_len];
        let len = self
            .connection
            .as_mut()
            .ok_or(StreamError)?
            .read(&mut bytes)
            .map_err(|_| StreamError)?;
        if len == 0 {
            return Err(StreamError);
        }
        bytes.truncate(len);
        Ok(bytes)
    }

    pub fn close(mut self) -> Result<(), StreamError> {
        let connection = self.connection.take().ok_or(StreamError)?;
        let (io, tls_result) = match connection.close() {
            Ok(io) => (io, Ok(())),
            Err((io, _)) => (io, Err(StreamError)),
        };
        let socket_result = io.socket.close().map_err(|_| StreamError);
        tls_result.and(socket_result)
    }
}

impl Drop for OwnedTlsStream<'_> {
    fn drop(&mut self) {
        drop(self.connection.take());
        // SAFETY: `connection` was dropped first, ending both exclusive
        // borrows. Each pointer came from Box::into_raw exactly once.
        unsafe {
            drop(Box::from_raw(self.read_buffer));
            drop(Box::from_raw(self.write_buffer));
        }
    }
}

#[derive(Debug)]
struct TransportError;

impl core::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CharlotteOS socket transport error")
    }
}

impl core::error::Error for TransportError {}

impl embedded_io::Error for TransportError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

struct SocketIo<'connection> {
    socket: socket::OwnedSocket<'connection>,
    pending: Vec<u8>,
    offset: usize,
    bounds: SocketBounds,
}

impl ErrorType for SocketIo<'_> {
    type Error = TransportError;
}

impl Read for SocketIo<'_> {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.offset == self.pending.len() {
            let chunk = self
                .socket
                .receive_timeout(self.bounds.receive_attempts, self.bounds.receive_retry_ms)
                .map_err(|_| TransportError)?
                .ok_or(TransportError)?;
            let (memory, len) = chunk.into_parts();
            let mapping = memory.map_read_only().map_err(|_| TransportError)?;
            self.pending = mapping.as_slice()[..len].to_vec();
            self.offset = 0;
        }
        let len = output.len().min(self.pending.len() - self.offset);
        output[..len].copy_from_slice(&self.pending[self.offset..self.offset + len]);
        self.offset += len;
        if self.offset == self.pending.len() {
            self.pending.clear();
            self.offset = 0;
        }
        Ok(len)
    }
}

impl EmbeddedWrite for SocketIo<'_> {
    fn write(&mut self, input: &[u8]) -> Result<usize, Self::Error> {
        self.socket
            .send_all(input, self.bounds.send_attempts, self.bounds.send_retry_ms)
            .map_err(|_| TransportError)?;
        Ok(input.len())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct SystemRng<'connection> {
    service: Option<ConnectionRef<'connection>>,
}

impl SystemRng<'_> {
    fn try_fill(&self, destination: &mut [u8]) -> Result<(), rand_core::Error> {
        let mut offset = 0;
        while offset < destination.len() {
            let Some(word) = catten_syscall::random_u64() else {
                break;
            };
            let bytes = word.to_ne_bytes();
            let length = (destination.len() - offset).min(bytes.len());
            destination[offset..offset + length].copy_from_slice(&bytes[..length]);
            offset += length;
        }
        if offset == destination.len() {
            return Ok(());
        }
        let remaining = destination.len() - offset;
        let service = self.service.ok_or_else(rng_error)?;
        let reply = service
            .call(entropy::OP_FILL, remaining as u64)
            .map_err(|_| rng_error())?
            .wait()
            .map_err(|_| rng_error())?;
        if reply.result != remaining as i64 {
            return Err(rng_error());
        }
        let mapping =
            reply.memory.ok_or_else(rng_error)?.map_read_only().map_err(|_| rng_error())?;
        destination[offset..].copy_from_slice(&mapping.as_slice()[..remaining]);
        Ok(())
    }
}

fn rng_error() -> rand_core::Error {
    rand_core::Error::from(
        NonZeroU32::new(rand_core::Error::CUSTOM_START).expect("nonzero error code"),
    )
}

impl RngCore for SystemRng<'_> {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.try_fill(&mut bytes).expect("system entropy unavailable");
        u64::from_ne_bytes(bytes)
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        self.try_fill_bytes(destination).expect("system entropy unavailable");
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), rand_core::Error> {
        self.try_fill(destination)
    }
}

impl CryptoRng for SystemRng<'_> {}

struct SynchronizedClock;

impl TlsClock for SynchronizedClock {
    fn now() -> Option<u64> {
        let seconds = TLS_UNIX_SECONDS.load(Ordering::Relaxed);
        (seconds != 0).then_some(seconds)
    }
}

struct TlsProvider<'connection, 'ca> {
    rng: SystemRng<'connection>,
    verifier: CertVerifier<'ca, Aes128GcmSha256, SynchronizedClock, TLS_CERTIFICATE_LEN>,
}

impl CryptoProvider for TlsProvider<'_, '_> {
    type CipherSuite = Aes128GcmSha256;
    type Signature = &'static [u8];

    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }

    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Aes128GcmSha256>, TlsError> {
        Ok(&mut self.verifier)
    }
}

type CharlotteTls<'connection> = TlsConnection<'connection, SocketIo<'connection>, Aes128GcmSha256>;

fn tls_error_code(error: &TlsError) -> u32 {
    match error {
        TlsError::ConnectionClosed => 1,
        TlsError::Unimplemented => 2,
        TlsError::MissingHandshake => 3,
        TlsError::HandshakeAborted(..) => 4,
        TlsError::AbortHandshake(..) => 5,
        TlsError::IoError | TlsError::Io(_) => 6,
        TlsError::InternalError => 7,
        TlsError::InvalidRecord => 8,
        TlsError::UnknownContentType => 9,
        TlsError::InvalidNonceLength => 10,
        TlsError::InvalidTicketLength => 11,
        TlsError::UnknownExtensionType => 12,
        TlsError::InsufficientSpace => 13,
        TlsError::InvalidHandshake => 14,
        TlsError::InvalidCipherSuite => 15,
        TlsError::InvalidSignatureScheme => 16,
        TlsError::InvalidSignature => 17,
        TlsError::InvalidExtensionsLength => 18,
        TlsError::InvalidSessionIdLength => 19,
        TlsError::InvalidSupportedVersions => 20,
        TlsError::InvalidApplicationData => 21,
        TlsError::InvalidKeyShare => 22,
        TlsError::InvalidCertificate => 23,
        TlsError::InvalidCertificateEntry => 24,
        TlsError::InvalidCertificateRequest => 25,
        TlsError::InvalidPrivateKey => 26,
        TlsError::UnableToInitializeCryptoEngine => 27,
        TlsError::ParseError(_) => 28,
        TlsError::OutOfMemory => 29,
        TlsError::CryptoError => 30,
        TlsError::EncodeError => 31,
        TlsError::DecodeError => 32,
    }
}
