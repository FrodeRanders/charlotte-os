use alloc::{
    sync::Arc,
    vec,
    vec::Vec,
};
use core::fmt;

use rustls::{
    CertificateError,
    DigitallySignedStruct,
    DistinguishedName,
    RootCertStore,
    ServerConfig,
    SignatureScheme,
    client::danger::HandshakeSignatureValid,
    pki_types::{
        CertificateDer,
        PrivateKeyDer,
        UnixTime,
    },
    server::{
        WebPkiClientVerifier,
        danger::{
            ClientCertVerified,
            ClientCertVerifier,
        },
    },
    time_provider::TimeProvider,
};

// Protocol dimensions, not workload policy: RFC 8446 section 5.2. Input
// storage also covers Rustls's conservative pre-negotiation record allowance.
const TLS_PLAINTEXT_BYTES: usize = 16_384;
const TLS_CIPHERTEXT_RECORD_BYTES: usize = TLS_PLAINTEXT_BYTES + 256 + 5;
const PRENEGOTIATION_RECORD_BYTES: usize = TLS_PLAINTEXT_BYTES + 2048 + 5;

/// Co-vary credential, record and operation budgets here. These are boot-time
/// inputs, copied into an immutable configuration before accepting any client.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_roots: usize,
    pub max_enrollments: usize,
    pub max_chain_certificates: usize,
    pub max_certificate_bytes: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_plaintext_bytes: usize,
    pub send_fragment_bytes: usize,
    pub handshake_ms: u64,
    pub session_ms: u64,
    pub work_per_step: usize,
}

impl Limits {
    pub const DEFAULT: Self = Self {
        max_roots: 8,
        max_enrollments: 64,
        max_chain_certificates: 4,
        max_certificate_bytes: 8 * 1024,
        max_input_bytes: 64 * 1024,
        max_output_bytes: 64 * 1024,
        max_plaintext_bytes: 16 * 1024,
        send_fragment_bytes: 4096,
        handshake_ms: 5_000,
        session_ms: 60_000,
        work_per_step: 32,
    };

    pub(crate) fn validate(self) -> Result<Self, PolicyError> {
        // TLS 1.3 peers may send a full 16 KiB plaintext record plus the
        // protocol's ciphertext allowance. Fragmented handshakes also retain
        // bounded internal Rustls storage (its current 65,535-byte limit).
        if self.max_roots == 0
            || self.max_enrollments == 0
            || self.max_chain_certificates == 0
            || self.max_certificate_bytes == 0
            || self.max_input_bytes < PRENEGOTIATION_RECORD_BYTES
            || self.max_output_bytes < TLS_CIPHERTEXT_RECORD_BYTES
            || self.max_plaintext_bytes == 0
            || self.max_plaintext_bytes > TLS_PLAINTEXT_BYTES
            || !(32..=TLS_PLAINTEXT_BYTES).contains(&self.send_fragment_bytes)
            || self.handshake_ms == 0
            || self.session_ms < self.handshake_ms
            || self.work_per_step == 0
            || self.max_chain_certificates.checked_mul(self.max_certificate_bytes).is_none()
        {
            return Err(PolicyError::InvalidLimits);
        }
        // Rustls's fragment setting includes the five-byte record header.
        // Each TLS 1.3 AES-GCM fragment also needs its inner content type and
        // authentication tag. Smaller fragments increase aggregate wire bytes.
        let fragments = self.max_plaintext_bytes.div_ceil(self.send_fragment_bytes - 5);
        let required = fragments
            .checked_mul(5 + 1 + 16)
            .and_then(|overhead| self.max_plaintext_bytes.checked_add(overhead))
            .ok_or(PolicyError::InvalidLimits)?;
        if required > self.max_output_bytes {
            return Err(PolicyError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Reader,
    Administrator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Observe,
    Mutate,
}

/// Audit identity is the exact enrolled leaf, never a peer-supplied name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Identity {
    enrollment: Enrollment,
}

impl Identity {
    pub(crate) fn authenticated(enrollment: Enrollment) -> Self {
        Self {
            enrollment,
        }
    }

    pub fn certificate_sha256(self) -> [u8; 32] {
        self.enrollment.certificate_sha256
    }

    pub fn role(self) -> Role {
        self.enrollment.role
    }

    pub fn allows(self, permission: Permission) -> bool {
        permission == Permission::Observe || self.enrollment.role == Role::Administrator
    }
}

/// A trusted boot input. Constructing this value does not authenticate its
/// provisioning source. Omission from a replacement configuration revokes new
/// sessions; the embedding service must retire sessions using the old config.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Enrollment {
    pub certificate_sha256: [u8; 32],
    pub role: Role,
}

pub struct Credentials {
    pub server_chain: Vec<CertificateDer<'static>>,
    pub server_key: PrivateKeyDer<'static>,
    pub client_roots: Vec<CertificateDer<'static>>,
    pub operators: Vec<Enrollment>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    InvalidLimits,
    MissingCredentials,
    CredentialCapacity,
    InvalidEnrollment,
    DuplicateEnrollment,
    InvalidRoot,
    InvalidServerIdentity,
    InvalidClock,
    InvalidVerifier,
}

pub fn fingerprint(certificate: &[u8]) -> [u8; 32] {
    embassy_crypto::Sha256::digest(certificate)
}

#[derive(Clone)]
pub struct Config {
    pub(crate) tls: Arc<ServerConfig>,
    pub(crate) operators: Arc<[Enrollment]>,
    pub(crate) limits: Limits,
}

impl Config {
    pub fn new(
        credentials: Credentials,
        clock: Arc<dyn TimeProvider>,
        limits: Limits,
    ) -> Result<Self, PolicyError> {
        let limits = limits.validate()?;
        if clock.current_time().is_none_or(|time| time.as_secs() == 0) {
            return Err(PolicyError::InvalidClock);
        }
        if credentials.server_chain.is_empty()
            || credentials.client_roots.is_empty()
            || credentials.operators.is_empty()
        {
            return Err(PolicyError::MissingCredentials);
        }
        if credentials.server_chain.len() > limits.max_chain_certificates
            || credentials.client_roots.len() > limits.max_roots
            || credentials.operators.len() > limits.max_enrollments
            || credentials
                .server_chain
                .iter()
                .chain(&credentials.client_roots)
                .any(|cert| cert.is_empty() || cert.len() > limits.max_certificate_bytes)
            || credentials.server_key.secret_der().len() > limits.max_certificate_bytes
        {
            return Err(PolicyError::CredentialCapacity);
        }
        for (index, identity) in credentials.operators.iter().enumerate() {
            if identity.certificate_sha256 == [0; 32] {
                return Err(PolicyError::InvalidEnrollment);
            }
            if credentials.operators[..index]
                .iter()
                .any(|old| old.certificate_sha256 == identity.certificate_sha256)
            {
                return Err(PolicyError::DuplicateEnrollment);
            }
        }
        let provider = Arc::new(crate::crypto::provider());
        let mut roots = RootCertStore::empty();
        for certificate in credentials.client_roots {
            roots.add(certificate).map_err(|_| PolicyError::InvalidRoot)?;
        }
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()
                .map_err(|_| PolicyError::InvalidVerifier)?;
        let operators: Arc<[Enrollment]> = credentials.operators.into();
        let mut tls = ServerConfig::builder_with_details(provider, Arc::new(NonzeroClock(clock)))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| PolicyError::InvalidVerifier)?
            .with_client_cert_verifier(Arc::new(EnrolledVerifier {
                verifier,
                operators: operators.clone(),
                limits,
            }))
            .with_single_cert(credentials.server_chain, credentials.server_key)
            .map_err(|_| PolicyError::InvalidServerIdentity)?;
        // Every connection proves possession against the current enrollment.
        // No PSK/ticket can retain authority from a retired configuration.
        tls.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
        tls.send_tls13_tickets = 0;
        tls.max_early_data_size = 0;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        tls.max_fragment_size = Some(limits.send_fragment_bytes);
        Ok(Self {
            tls: Arc::new(tls),
            operators,
            limits,
        })
    }
}

#[derive(Debug)]
struct NonzeroClock(Arc<dyn TimeProvider>);
impl TimeProvider for NonzeroClock {
    fn current_time(&self) -> Option<UnixTime> {
        self.0.current_time().filter(|time| time.as_secs() != 0)
    }
}

struct EnrolledVerifier {
    verifier: Arc<dyn ClientCertVerifier>,
    operators: Arc<[Enrollment]>,
    limits: Limits,
}

impl fmt::Debug for EnrolledVerifier {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("EnrolledManagementClientVerifier")
    }
}

impl ClientCertVerifier for EnrolledVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.verifier.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        if now.as_secs() == 0
            || intermediates.len() >= self.limits.max_chain_certificates
            || core::iter::once(end_entity)
                .chain(intermediates)
                .any(|cert| cert.is_empty() || cert.len() > self.limits.max_certificate_bytes)
        {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        let digest = fingerprint(end_entity);
        if !self.operators.iter().any(|identity| identity.certificate_sha256 == digest) {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        // Full chain, validity, EKU, constraints and handshake proof remain
        // Rustls/WebPKI's checks. Pinning alone cannot bypass them.
        self.verifier.verify_client_cert(end_entity, intermediates, now)
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("management requires TLS 1.3".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.verifier.verify_tls13_signature(message, cert, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.verifier.supported_verify_schemes()
    }
}
