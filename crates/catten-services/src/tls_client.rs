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

use embedded_io::{
    ErrorType,
    Read,
    Write as EmbeddedWrite,
};
use embedded_tls::{
    Aes128GcmSha256,
    Certificate,
    PrivateKey,
    TlsClock,
    TlsConfig,
    TlsContext,
    TlsError,
    blocking::TlsConnection,
    pki::CertVerifier,
};
use zeroize::Zeroizing;

use crate::socket;

mod crypto;
mod entropy_client;
#[cfg(test)]
use entropy_client::{
    DOMAIN_ENTROPY,
    EntropySource,
    SystemRng,
};
pub use entropy_client::{
    EntropyConfig,
    EntropyError,
    EntropyLimits,
    ServiceEntropyRead,
    fill_domain_entropy,
    fill_entropy,
    fill_service_entropy,
    install_domain_entropy,
};
#[cfg(all(test, unix))]
mod interop;
mod runtime;

use runtime::{
    HandshakeClock,
    HandshakeGuard,
};

const TLS_RECORD_BUFFER_LEN: usize = 16_640;
const TLS_CERTIFICATE_LEN: usize = 16_384;

// The selected TLS AES-128-GCM implementation uses these AES owners. Require
// their erasing Drop even if another workspace package changes its features.
// This does not cover TLS traffic secrets, HKDF state or GHASH/POLYVAL.
const _: fn() = || {
    fn erases_on_drop<T: zeroize::ZeroizeOnDrop>() {}
    erases_on_drop::<aes::Aes128>();
    erases_on_drop::<aes::Aes128Enc>();
    erases_on_drop::<aes::Aes128Dec>();
};

// The selected TLS cipher dispatches to the sole registered erasing driver.
const _: fn(
    <Aes128GcmSha256 as embedded_tls::TlsCipherSuite>::Cipher,
) -> embassy_crypto::Aes128Gcm = |cipher| cipher;

// Require the selected authentication-state owners to have Drop. AArch64's
// default software backend erases its fields. Freestanding x86 selects it with
// polyval_force_soft because the pinned dispatcher does not drop its backend.
// Reject flag/feature overrides rather than silently losing state erasure.
// Pinned source establishes the destructor's semantics, not needs_drop alone.
#[cfg(any(target_arch = "aarch64", all(target_arch = "x86_64", target_os = "none")))]
const _: () = {
    assert!(core::mem::needs_drop::<polyval::Polyval>());
    assert!(core::mem::needs_drop::<ghash::GHash>());
};

// The verifier's static clock is admitted for exactly one handshake. Entropy
// authority is installed once and owned for this domain's lifetime.
static HANDSHAKE_CLOCK: HandshakeClock = HandshakeClock::new();
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

/// Inputs required to authenticate a TLS server and, optionally, present a
/// client identity.
pub struct OpenConfig<'a> {
    pub server_name: &'a str,
    pub ca_certificate_der: &'a [u8],
    pub client_certificate_der: Option<&'a [u8]>,
    pub client_private_key_der: Option<&'a [u8]>,
    pub unix_seconds: u64,
    pub socket_bounds: SocketBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenError {
    InvalidConfiguration,
    EntropyUnavailable,
    CryptoBusy,
    Handshake(u32),
}

impl OpenError {
    /// Whether opening TLS failed because the underlying byte stream closed or
    /// returned I/O failure, rather than because peer authentication failed.
    pub const fn is_transport_failure(self) -> bool {
        matches!(self, Self::Handshake(1 | 6))
    }
}

/// A failure while using or explicitly closing an established TLS stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamError;

/// A verified TLS byte stream that owns all resources needed by the session.
#[must_use = "dropping the stream closes its TLS session and owned socket"]
pub struct OwnedTlsStream<'connection> {
    connection: Option<CharlotteTls<'connection>>,
    _buffers: TlsRecordBuffers,
    receive_chunk_len: usize,
}

/// Stable allocations for embedded-tls's borrowed record buffers. Only this
/// adapter uses raw pointers: the library cannot own its borrowed buffers.
/// The connection must always be dropped before this owner.
struct TlsRecordBuffers {
    read: *mut [u8; TLS_RECORD_BUFFER_LEN],
    write: *mut [u8; TLS_RECORD_BUFFER_LEN],
}

impl TlsRecordBuffers {
    fn new() -> Self {
        Self {
            read: Box::into_raw(Box::new([0; TLS_RECORD_BUFFER_LEN])),
            write: Box::into_raw(Box::new([0; TLS_RECORD_BUFFER_LEN])),
        }
    }
}

impl Drop for TlsRecordBuffers {
    fn drop(&mut self) {
        // SAFETY: the adapter drops the connection first, ending both
        // exclusive borrows. Each allocation is adopted exactly once here.
        unsafe {
            use zeroize::Zeroize;
            let mut read = Box::from_raw(self.read);
            let mut write = Box::from_raw(self.write);
            read.as_mut().zeroize();
            write.as_mut().zeroize();
        }
    }
}

impl<'connection> OwnedTlsStream<'connection> {
    pub fn open(
        socket: socket::OwnedSocket<'connection>,
        config: OpenConfig<'_>,
    ) -> Result<Self, OpenError> {
        let handshake = PreparedHandshake::new(&config)?;
        let buffers = TlsRecordBuffers::new();
        // SAFETY: these allocations remain exclusively owned by the wrapper,
        // are not moved, and are reclaimed only after `connection` is dropped.
        let read_ref: &'connection mut [u8] = unsafe { &mut *buffers.read };
        let write_ref: &'connection mut [u8] = unsafe { &mut *buffers.write };
        let io = SocketIo {
            socket,
            pending: Vec::new(),
            offset: 0,
            bounds: config.socket_bounds,
        };
        let mut connection = TlsConnection::new(io, read_ref, write_ref);
        if let Err(error) = handshake.open(&mut connection) {
            let code = tls_error_code(&error);
            drop(connection);
            // `buffers` is then dropped and wiped on this error path too.
            return Err(OpenError::Handshake(code));
        }
        Ok(Self {
            connection: Some(connection),
            _buffers: buffers,
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

    pub fn receive(&mut self) -> Result<Zeroizing<Vec<u8>>, StreamError> {
        let connection = self.connection.as_mut().ok_or(StreamError)?;
        read_owned_chunk(connection, self.receive_chunk_len)
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

/// Read plaintext into an erasing owner before exposing the destination to its
/// producer. Success transfers that owner; error or EOF drops the same buffer.
/// The caller supplies the existing producer wait policy and chunk bound.
pub fn read_owned_chunk(
    reader: &mut impl Read,
    chunk_len: usize,
) -> Result<Zeroizing<Vec<u8>>, StreamError> {
    let mut bytes = Zeroizing::new(vec![0; chunk_len]);
    let len = reader.read(&mut bytes).map_err(|_| StreamError)?;
    if len == 0 || len > bytes.len() {
        return Err(StreamError);
    }
    bytes.truncate(len);
    Ok(bytes)
}

impl Drop for OwnedTlsStream<'_> {
    fn drop(&mut self) {
        drop(self.connection.take());
        // The record-buffer owner is dropped and wiped after this method.
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

struct SynchronizedClock;
impl TlsClock for SynchronizedClock {
    fn now() -> Option<u64> {
        HANDSHAKE_CLOCK.now()
    }
}

/// Owns the admitted verification clock and optional erasing identity through
/// one handshake; no provider borrows a caller's entropy capability globally.
struct PreparedHandshake<'config, 'data> {
    config: &'config OpenConfig<'data>,
    identity: Option<PrivateKey>,
    _clock: HandshakeGuard<'static>,
}

impl<'config, 'data> PreparedHandshake<'config, 'data> {
    fn new(config: &'config OpenConfig<'data>) -> Result<Self, OpenError> {
        if config.server_name.is_empty()
            || config.ca_certificate_der.is_empty()
            || config.unix_seconds == 0
            || config.socket_bounds.send_attempts == 0
            || config.socket_bounds.receive_attempts == 0
            || config.socket_bounds.receive_chunk_len == 0
            || config.client_certificate_der.is_some() != config.client_private_key_der.is_some()
        {
            return Err(OpenError::InvalidConfiguration);
        }
        let identity = config
            .client_private_key_der
            .map(crypto::load_client_key)
            .transpose()
            .map_err(|_| OpenError::InvalidConfiguration)?;
        let clock = HANDSHAKE_CLOCK.begin(config.unix_seconds).ok_or(OpenError::CryptoBusy)?;
        let mut probe = Zeroizing::new([0; 1]);
        fill_domain_entropy(&mut *probe).map_err(|_| OpenError::EntropyUnavailable)?;
        Ok(Self {
            config,
            identity,
            _clock: clock,
        })
    }

    fn open<Socket: Read + EmbeddedWrite>(
        self,
        connection: &mut TlsConnection<'_, Socket, Aes128GcmSha256>,
    ) -> Result<(), TlsError> {
        let config = TlsConfig::new().with_server_name(self.config.server_name);
        let verifier = CertVerifier::<Aes128GcmSha256, SynchronizedClock, TLS_CERTIFICATE_LEN>::new(
            Certificate::X509(self.config.ca_certificate_der),
        );
        let mut context = TlsContext::new(&config, verifier);
        if let (Some(certificate), Some(key)) =
            (self.config.client_certificate_der, self.identity.as_ref())
        {
            context = context.with_client_cert(Certificate::X509(certificate), key);
        }
        connection.open(context)
    }
}

type CharlotteTls<'connection> = TlsConnection<'connection, SocketIo<'connection>, Aes128GcmSha256>;

fn tls_error_code(error: &TlsError) -> u32 {
    match error {
        TlsError::ConnectionClosed => 1,
        TlsError::Unimplemented => 2,
        TlsError::MissingHandshake => 3,
        TlsError::MissingPsk => 33,
        TlsError::MissingServerName => 34,
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

#[cfg(test)]
mod tests {
    extern crate std;
    use std::sync::{
        Mutex,
        MutexGuard,
        Once,
    };
    static HOST_CRYPTO: Mutex<()> = Mutex::new(());
    static HOST_ENTROPY: Once = Once::new();

    pub(super) fn host_crypto() -> MutexGuard<'static, ()> {
        let guard = HOST_CRYPTO.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        HOST_ENTROPY.call_once(|| {
            assert!(super::DOMAIN_ENTROPY.install(super::EntropySource::Host).is_ok())
        });
        guard
    }

    use aes::cipher::{
        BlockDecrypt,
        BlockEncrypt,
        KeyInit,
    };

    mod gcm_contract {
        include!("tls_client/gcm_contract.rs");
    }

    #[test]
    fn selected_tls_gcm_preserves_ciphertext_tag_and_rejects_tampering() {
        gcm_contract::check();
    }

    #[test]
    fn erasing_aes_owners_preserve_block_encryption_and_decryption() {
        // Public AES-128 fixture, also checked independently with OpenSSL in
        // the audit evidence. No TLS or application credential is used here.
        let key = core::array::from_fn::<_, 16, _>(|i| i as u8);
        let plaintext = core::array::from_fn::<_, 16, _>(|i| (i * 0x11) as u8);
        let ciphertext = [
            0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
            0xc5, 0x5a,
        ];
        let cipher = aes::Aes128::new(&key.into());
        let mut block = plaintext.into();
        cipher.encrypt_block(&mut block);
        assert_eq!(block.as_slice(), ciphertext);
        cipher.decrypt_block(&mut block);
        assert_eq!(block.as_slice(), plaintext);

        let encrypt = aes::Aes128Enc::new(&key.into());
        encrypt.encrypt_block(&mut block);
        assert_eq!(block.as_slice(), ciphertext);
        let decrypt = aes::Aes128Dec::new(&key.into());
        decrypt.decrypt_block(&mut block);
        assert_eq!(block.as_slice(), plaintext);
    }
}
