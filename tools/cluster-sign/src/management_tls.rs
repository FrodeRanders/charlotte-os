//! Verified mutual TLS for every management read/write. No application byte is
//! submitted until the handshake completes. Socket I/O keeps one original budget.

use std::{
    io::{
        self,
        Read,
        Write,
    },
    net::{
        SocketAddr,
        TcpStream,
    },
    sync::Arc,
};

use rustls::{
    pki_types::{
        pem::PemObject,
        CertificateDer,
        PrivateKeyDer,
        ServerName,
    },
    ClientConfig,
    ClientConnection,
    RootCertStore,
    StreamOwned,
};

use crate::{
    http_client::{
        Deadline,
        ExchangeFailure,
    },
    profile_input,
};

// Bound complete PEM files, including base64/line overhead, before parsing.
const FILE_BYTES: usize = charlotte_launch::management::MAX_PROFILE_BYTES * 2;
pub(super) struct BudgetedSocket {
    socket: TcpStream,
    deadline: Deadline,
}
impl Read for BudgetedSocket {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.socket.set_read_timeout(Some(self.deadline.remaining()?))?;
        self.socket.read(bytes)
    }
}
impl Write for BudgetedSocket {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.socket.set_write_timeout(Some(self.deadline.remaining()?))?;
        self.socket.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.set_write_timeout(Some(self.deadline.remaining()?))?;
        self.socket.flush()
    }
}
fn input(name: &str) -> crate::Result<profile_input::ProfileInput> {
    let path = std::env::var(name).map_err(|_| {
        format!(
            "missing {name}; verified management TLS requires CA, client certificate and \
             private-key files"
        )
    })?;
    let reader = if name == "CATTEN_MANAGEMENT_KEY_FILE" {
        profile_input::read_bounded_private_file
    } else {
        profile_input::read_bounded_file
    };
    reader(&path, FILE_BYTES).map_err(|error| format!("read {name}: {error}"))
}
fn certificates(name: &str) -> crate::Result<Vec<CertificateDer<'static>>> {
    let input = input(name)?;
    let certs = CertificateDer::pem_slice_iter(input.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| format!("{name}: invalid certificate PEM"))?;
    if certs.is_empty()
        || certs.len() > charlotte_launch::management::Limits::DEFAULT.max_roots
        || certs.iter().any(|certificate| {
            certificate.len() > charlotte_launch::management::Limits::DEFAULT.max_certificate_bytes
        })
    {
        return Err(format!("{name}: certificate capacity exceeded"));
    }
    Ok(certs)
}
fn configuration() -> crate::Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    for root in certificates("CATTEN_MANAGEMENT_CA_FILE")? {
        roots.add(root).map_err(|_| "invalid management CA".to_owned())?;
    }
    let chain = certificates("CATTEN_MANAGEMENT_CERT_FILE")?;
    if chain.len() > charlotte_launch::management::Limits::DEFAULT.max_chain_certificates {
        return Err("management client chain capacity exceeded".into());
    }
    let bytes = input("CATTEN_MANAGEMENT_KEY_FILE")?;
    let key = PrivateKeyDer::from_pem_slice(bytes.as_bytes())
        .map_err(|_| "invalid management private-key PEM".to_owned())?;
    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites = vec![rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256];
    provider.kx_groups = vec![rustls::crypto::ring::kx_group::SECP256R1];
    let mut config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| "management TLS versions".to_owned())?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .map_err(|_| "management client certificate/key mismatch".to_owned())?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.enable_early_data = false;
    Ok(config)
}

pub(super) fn connect(
    endpoint: SocketAddr,
    deadline: Deadline,
) -> std::result::Result<StreamOwned<ClientConnection, BudgetedSocket>, ExchangeFailure> {
    deadline.remaining().map_err(|error| ExchangeFailure::BeforeSubmission(error.to_string()))?;
    let config = configuration().map_err(ExchangeFailure::Configuration)?;
    let name = match std::env::var("CATTEN_MANAGEMENT_SERVER_NAME") {
        Ok(name) => ServerName::try_from(name)
            .map_err(|_| ExchangeFailure::Configuration("invalid management server name".into()))?,
        Err(std::env::VarError::NotPresent) => ServerName::IpAddress(endpoint.ip().into()),
        Err(_) => {
            return Err(ExchangeFailure::Configuration(
                "management server name is not UTF-8".into(),
            ))
        }
    };
    let socket = TcpStream::connect_timeout(
        &endpoint,
        deadline
            .remaining()
            .map_err(|error| ExchangeFailure::BeforeSubmission(error.to_string()))?,
    )
    .map_err(|error| {
        ExchangeFailure::BeforeSubmission(format!("connect management TLS {endpoint}: {error}"))
    })?;
    let mut socket = BudgetedSocket {
        socket,
        deadline,
    };
    let mut connection = ClientConnection::new(Arc::new(config), name).map_err(|error| {
        ExchangeFailure::Configuration(format!("management TLS configuration: {error}"))
    })?;
    while connection.is_handshaking() {
        connection.complete_io(&mut socket).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                ExchangeFailure::Configuration(format!(
                    "management TLS verification rejected before HTTP submission: {error}"
                ))
            } else {
                ExchangeFailure::BeforeSubmission(format!(
                    "management TLS handshake failed before HTTP submission: {error}"
                ))
            }
        })?;
    }
    deadline.remaining().map_err(|error| ExchangeFailure::BeforeSubmission(error.to_string()))?;
    Ok(StreamOwned::new(connection, socket))
}
