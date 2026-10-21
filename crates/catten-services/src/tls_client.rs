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
    SignatureScheme,
    TlsClock,
    TlsConfig,
    TlsContext,
    TlsError,
    TlsVerifier,
    blocking::TlsConnection,
    pki::CertVerifier,
};
use p256::{
    SecretKey,
    ecdsa::{
        DerSignature,
        SigningKey,
    },
};
use rand_core::{
    CryptoRng,
    RngCore,
};
use zeroize::Zeroizing;

use crate::{
    entropy,
    socket,
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

// Keep the dependency feature edges tied to the cipher selected by TLS.
const _: fn(<Aes128GcmSha256 as embedded_tls::TlsCipherSuite>::Cipher) -> aes_gcm::Aes128Gcm =
    |cipher| cipher;

// The current AArch64 software POLYVAL backend erases its fields on Drop.
// Reject a feature/backend change that removes that destructor. The pinned
// x86 dispatch and optional ARM PMULL implementations need separate fixes;
// this assertion does not qualify their state erasure.
#[cfg(target_arch = "aarch64")]
const _: () = {
    assert!(core::mem::needs_drop::<polyval::Polyval>());
    assert!(core::mem::needs_drop::<ghash::GHash>());
};

/// Unix time used by the in-progress TLS handshake. The verifier consults the
/// clock only while `open` runs to completion, and each service domain is
/// single-threaded, so one handshake-scoped value is sufficient. It is stored
/// immediately before the verifier is constructed and replaced by the next
/// handshake.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntropyUnavailable;

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
        entropy_service: Option<ConnectionRef<'connection>>,
        config: OpenConfig<'_>,
    ) -> Result<Self, OpenError> {
        if config.server_name.is_empty()
            || config.ca_certificate_der.is_empty()
            || config.unix_seconds == 0
            || config.socket_bounds.send_attempts == 0
            || config.socket_bounds.receive_attempts == 0
            || config.socket_bounds.receive_chunk_len == 0
            || config.client_certificate_der.is_some() != config.client_private_key_der.is_some()
            || config
                .client_private_key_der
                .is_some_and(|key| SecretKey::from_sec1_der(key).is_err())
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
        let tls_config = TlsConfig::new().with_server_name(config.server_name);
        let provider = TlsProvider {
            rng: SystemRng {
                service: entropy_service,
            },
            verifier: CertVerifier::new(Certificate::X509(config.ca_certificate_der)),
            client_certificate_der: config.client_certificate_der,
            client_private_key_der: config.client_private_key_der,
        };
        if let Err(error) = connection.open(TlsContext::new(&tls_config, provider)) {
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

struct SystemRng<'connection> {
    service: Option<ConnectionRef<'connection>>,
}

/// Fill a caller-owned buffer from the same architecture/leased entropy path
/// used by TLS. Protocol clients use this for nonces without gaining raw
/// device authority.
pub fn fill_entropy(
    service: Option<ConnectionRef<'_>>,
    destination: &mut [u8],
) -> Result<(), EntropyUnavailable> {
    SystemRng {
        service,
    }
    .try_fill(destination)
    .map_err(|_| EntropyUnavailable)
}

impl SystemRng<'_> {
    /// Fill a handshake buffer, retrying a briefly busy entropy service
    /// before treating the missing randomness as fatal. `RngCore::fill_bytes`
    /// cannot report an error, so exhaustion remains fail-stop; callers that
    /// can handle it use [`fill_entropy`] or `try_fill_bytes`.
    fn fill_or_fail(&self, destination: &mut [u8]) {
        for attempt in 0..3 {
            if self.try_fill(destination).is_ok() {
                return;
            }
            if attempt < 2 {
                crate::sleep_ms(1);
            }
        }
        panic!("system entropy unavailable during a TLS handshake");
    }

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
        let input = mapping.as_slice().get(..remaining).ok_or_else(rng_error)?;
        destination[offset..].copy_from_slice(input);
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
        self.fill_or_fail(&mut bytes);
        u64::from_ne_bytes(bytes)
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        self.fill_or_fail(destination);
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

struct TlsProvider<'connection, 'ca, 'identity> {
    rng: SystemRng<'connection>,
    verifier: CertVerifier<'ca, Aes128GcmSha256, SynchronizedClock, TLS_CERTIFICATE_LEN>,
    client_certificate_der: Option<&'identity [u8]>,
    client_private_key_der: Option<&'identity [u8]>,
}

impl CryptoProvider for TlsProvider<'_, '_, '_> {
    type CipherSuite = Aes128GcmSha256;
    type Signature = DerSignature;

    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }

    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Aes128GcmSha256>, TlsError> {
        Ok(&mut self.verifier)
    }

    fn signer(
        &mut self,
    ) -> Result<(impl signature::SignerMut<Self::Signature>, SignatureScheme), TlsError> {
        let private_key = self.client_private_key_der.ok_or(TlsError::InvalidPrivateKey)?;
        let secret_key =
            SecretKey::from_sec1_der(private_key).map_err(|_| TlsError::InvalidPrivateKey)?;
        Ok((SigningKey::from(&secret_key), SignatureScheme::EcdsaSecp256r1Sha256))
    }

    fn client_cert(&mut self) -> Option<Certificate<impl AsRef<[u8]>>> {
        self.client_certificate_der.map(Certificate::X509)
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

#[cfg(test)]
mod tests {
    use aes::cipher::{
        BlockDecrypt,
        BlockEncrypt,
        KeyInit,
    };
    use aes_gcm::AeadInPlace;

    #[test]
    fn selected_tls_gcm_preserves_ciphertext_tag_and_rejects_tampering() {
        type Cipher = <embedded_tls::Aes128GcmSha256 as embedded_tls::TlsCipherSuite>::Cipher;
        let cipher = Cipher::new(&[0u8; 16].into());
        let nonce = [0u8; 12].into();
        let mut block = [0u8; 16];
        let expected = [
            0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92, 0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2,
            0xfe, 0x78,
        ];
        let tag = cipher.encrypt_in_place_detached(&nonce, b"", &mut block).unwrap();
        assert_eq!(block, expected);
        assert_eq!(
            tag.as_slice(),
            [
                0xab, 0x6e, 0x47, 0xd4, 0x2c, 0xec, 0x13, 0xbd, 0xf5, 0x3a, 0x67, 0xb2, 0x12, 0x57,
                0xbd, 0xdf,
            ]
        );
        let mut invalid = tag;
        invalid[0] ^= 1;
        assert!(cipher.decrypt_in_place_detached(&nonce, b"", &mut block, &invalid).is_err());
        assert_eq!(block, expected);
        cipher.decrypt_in_place_detached(&nonce, b"", &mut block, &tag).unwrap();
        assert_eq!(block, [0; 16]);
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
