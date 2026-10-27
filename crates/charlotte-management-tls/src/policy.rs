use alloc::{
    sync::Arc,
    vec,
    vec::Vec,
};
use core::fmt;

pub use charlotte_launch::management::{
    Enrollment,
    Limits,
    Role,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Observe,
    Mutate,
}

/// Audit identity is the exact enrolled leaf, never a peer-supplied name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Identity {
    enrollment: Enrollment,
    verified_at: u64,
}

impl Identity {
    pub(crate) fn authenticated(enrollment: Enrollment) -> Self {
        Self {
            enrollment,
            verified_at: 0,
        }
    }

    pub fn certificate_sha256(self) -> [u8; 32] {
        self.enrollment.certificate_sha256
    }

    pub fn role(self) -> Role {
        self.enrollment.role
    }

    /// UTC second of this checked authority, not a trusted-time attestation.
    pub fn verified_at_unix_seconds(self) -> u64 {
        self.verified_at
    }

    pub(crate) fn verified_at(&mut self, seconds: u64) {
        self.verified_at = seconds;
    }

    pub fn allows(self, permission: Permission) -> bool {
        permission == Permission::Observe || self.enrollment.role == Role::Administrator
    }
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
    InvalidProfile,
    AllocationFailed,
}

pub fn fingerprint(certificate: &[u8]) -> [u8; 32] {
    embassy_crypto::Sha256::digest(certificate)
}

#[derive(Clone)]
pub struct Config {
    pub(crate) tls: Arc<ServerConfig>,
    pub(crate) operators: Arc<[Enrollment]>,
    pub(crate) limits: Limits,
    clock: Arc<dyn TimeProvider>,
    verifier: Arc<EnrolledVerifier>,
}

/// Credential preparation before time is available. Session accepts only the
/// activated Config, which requires a real nonzero clock reading.
pub struct PreparedConfig {
    config: Config,
    clock: Arc<dyn TimeProvider>,
}
impl PreparedConfig {
    pub fn limits(&self) -> Limits {
        self.config.limits
    }

    pub fn new(
        credentials: Credentials,
        clock: Arc<dyn TimeProvider>,
        limits: Limits,
    ) -> Result<Self, PolicyError> {
        Ok(Self {
            config: Config::build(credentials, clock.clone(), limits)?,
            clock,
        })
    }

    pub fn from_profile(bytes: &[u8], clock: Arc<dyn TimeProvider>) -> Result<Self, PolicyError> {
        Ok(Self {
            config: Config::decode_profile(bytes, clock.clone())?,
            clock,
        })
    }

    pub fn activate(&self) -> Result<Config, PolicyError> {
        if self.clock.current_time().is_none_or(|time| time.as_secs() == 0) {
            return Err(PolicyError::InvalidClock);
        }
        Ok(self.config.clone())
    }
}

impl Config {
    /// The embedding service borrows this complete profile from launch-owned
    /// read-only memory. This decoder grants no provisioning authority: the
    /// kernel must authenticate the resource before constructing that launch.
    pub fn from_profile(bytes: &[u8], clock: Arc<dyn TimeProvider>) -> Result<Self, PolicyError> {
        PreparedConfig::from_profile(bytes, clock)?.activate()
    }

    fn decode_profile(bytes: &[u8], clock: Arc<dyn TimeProvider>) -> Result<Self, PolicyError> {
        let profile = charlotte_launch::management::Profile::decode(bytes)
            .map_err(|_| PolicyError::InvalidProfile)?;
        fn copy(bytes: &[u8]) -> Result<Vec<u8>, PolicyError> {
            let mut result = Vec::new();
            result.try_reserve_exact(bytes.len()).map_err(|_| PolicyError::AllocationFailed)?;
            result.extend_from_slice(bytes);
            Ok(result)
        }
        fn certificates<'a>(
            source: impl Iterator<Item = &'a [u8]>,
        ) -> Result<Vec<CertificateDer<'static>>, PolicyError> {
            let mut result = Vec::new();
            for certificate in source {
                result.try_reserve(1).map_err(|_| PolicyError::AllocationFailed)?;
                result.push(CertificateDer::from(copy(certificate)?));
            }
            Ok(result)
        }
        let format = PrivateKeyDer::try_from(profile.server_key())
            .map_err(|_| PolicyError::InvalidServerIdentity)?;
        // Own the destination before the first secret copy, then move its one
        // allocation directly into pki-types' erasing private-key owner.
        let mut key = zeroize::Zeroizing::new(Vec::new());
        key.try_reserve_exact(profile.server_key().len())
            .map_err(|_| PolicyError::AllocationFailed)?;
        match format {
            PrivateKeyDer::Pkcs8(_) | PrivateKeyDer::Sec1(_) => (),
            _ => return Err(PolicyError::InvalidServerIdentity),
        }
        key.extend_from_slice(profile.server_key());
        let server_key = match format {
            PrivateKeyDer::Pkcs8(_) => PrivateKeyDer::Pkcs8(core::mem::take(&mut *key).into()),
            PrivateKeyDer::Sec1(_) => PrivateKeyDer::Sec1(core::mem::take(&mut *key).into()),
            _ => unreachable!(),
        };
        let mut operators = Vec::new();
        for operator in profile.operators() {
            operators.try_reserve(1).map_err(|_| PolicyError::AllocationFailed)?;
            operators.push(operator.map_err(|_| PolicyError::InvalidProfile)?);
        }
        Self::build(
            Credentials {
                server_chain: certificates(profile.server_chain())?,
                server_key,
                client_roots: certificates(profile.client_roots())?,
                operators,
            },
            clock,
            profile.limits,
        )
    }

    pub fn new(
        credentials: Credentials,
        clock: Arc<dyn TimeProvider>,
        limits: Limits,
    ) -> Result<Self, PolicyError> {
        PreparedConfig::new(credentials, clock, limits)?.activate()
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub(crate) fn current_time(&self) -> Option<UnixTime> {
        self.clock.current_time().filter(|time| time.as_secs() != 0)
    }

    pub(crate) fn verify_peer(
        &self,
        chain: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<(), rustls::Error> {
        let (leaf, intermediates) =
            chain.split_first().ok_or(rustls::Error::NoCertificatesPresented)?;
        self.verifier.verify_client_cert(leaf, intermediates, now).map(|_| ())
    }

    fn build(
        credentials: Credentials,
        clock: Arc<dyn TimeProvider>,
        limits: Limits,
    ) -> Result<Self, PolicyError> {
        let limits = limits.validate().map_err(|_| PolicyError::InvalidLimits)?;
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
        let verifier = Arc::new(EnrolledVerifier {
            verifier,
            operators: operators.clone(),
            limits,
        });
        let mut tls =
            ServerConfig::builder_with_details(provider, Arc::new(NonzeroClock(clock.clone())))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .map_err(|_| PolicyError::InvalidVerifier)?
                .with_client_cert_verifier(verifier.clone())
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
            clock,
            verifier,
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
