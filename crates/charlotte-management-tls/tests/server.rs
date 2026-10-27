use std::{
    fs,
    io::{
        Cursor,
        Read,
        Write,
    },
    path::PathBuf,
    process::{
        Command,
        Stdio,
    },
    sync::{
        Arc,
        atomic::{
            AtomicU64,
            Ordering,
        },
    },
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use charlotte_management_tls::*;
use embassy_crypto_rustcrypto as _;
use rustls::{
    ClientConfig,
    ClientConnection,
    RootCertStore,
};

struct HostEntropy;
impl embassy_crypto::driver::Rng for HostEntropy {
    fn fill_bytes(bytes: &mut [u8]) {
        getrandom::fill(bytes).expect("host fixture entropy");
    }
}
embassy_crypto::rng_impl!(HostEntropy);

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

impl Fixture {
    fn command(&self, args: &[&str]) {
        let result = Command::new("openssl").args(args).current_dir(&self.0).output().unwrap();
        assert!(
            result.status.success(),
            "openssl {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    fn new() -> Self {
        let fixture = Self(std::env::temp_dir().join(format!(
            "charlotte-management-tls-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        )));
        fs::create_dir(&fixture.0).unwrap();
        for name in ["ca", "server", "reader", "admin", "unknown", "foreign"] {
            fixture.command(&[
                "ecparam",
                "-name",
                "prime256v1",
                "-genkey",
                "-noout",
                "-out",
                &format!("{name}.key"),
            ]);
        }
        fixture.command(&[
            "req",
            "-x509",
            "-new",
            "-key",
            "ca.key",
            "-days",
            "3650",
            "-subj",
            "/CN=Charlotte public test CA",
            "-out",
            "ca.pem",
        ]);
        for (serial, name) in ["server", "reader", "admin", "unknown"].iter().enumerate() {
            fixture.command(&[
                "req",
                "-new",
                "-key",
                &format!("{name}.key"),
                "-subj",
                &format!("/CN={name}"),
                "-out",
                &format!("{name}.csr"),
            ]);
            fs::write(
                fixture.0.join("extensions"),
                format!(
                    concat!(
                        "basicConstraints=critical,CA:FALSE\n",
                        "keyUsage=critical,digitalSignature\n",
                        "subjectAltName=DNS:tls.test\n",
                        "extendedKeyUsage={}\n",
                    ),
                    if *name == "server" {
                        "serverAuth"
                    } else {
                        "clientAuth"
                    }
                ),
            )
            .unwrap();
            fixture.command(&[
                "x509",
                "-req",
                "-in",
                &format!("{name}.csr"),
                "-CA",
                "ca.pem",
                "-CAkey",
                "ca.key",
                "-set_serial",
                &(serial + 2).to_string(),
                "-days",
                "3650",
                "-extfile",
                "extensions",
                "-out",
                &format!("{name}.pem"),
            ]);
        }
        fixture.command(&[
            "req",
            "-x509",
            "-new",
            "-key",
            "foreign.key",
            "-days",
            "3650",
            "-subj",
            "/CN=Foreign public test CA",
            "-out",
            "foreign.pem",
        ]);
        for name in ["ca", "server", "reader", "admin", "unknown", "foreign"] {
            fixture.command(&[
                "x509",
                "-in",
                &format!("{name}.pem"),
                "-outform",
                "DER",
                "-out",
                &format!("{name}.der"),
            ]);
            if name != "ca" {
                fixture.command(&[
                    "pkcs8",
                    "-topk8",
                    "-nocrypt",
                    "-in",
                    &format!("{name}.key"),
                    "-outform",
                    "DER",
                    "-out",
                    &format!("{name}-key.der"),
                ]);
            }
        }
        fixture
    }

    fn cert(&self, name: &str) -> CertificateDer<'static> {
        CertificateDer::from(fs::read(self.0.join(format!("{name}.der"))).unwrap())
    }

    fn key(&self, name: &str) -> PrivateKeyDer<'static> {
        PrivateKeyDer::try_from(fs::read(self.0.join(format!("{name}-key.der"))).unwrap()).unwrap()
    }

    fn credentials(&self) -> Credentials {
        Credentials {
            server_chain: vec![self.cert("server")],
            server_key: self.key("server"),
            client_roots: vec![self.cert("ca")],
            operators: vec![
                self.identity("reader", Role::Reader),
                self.identity("admin", Role::Administrator),
            ],
        }
    }

    fn identity(&self, name: &str, role: Role) -> Enrollment {
        Enrollment {
            certificate_sha256: fingerprint(&self.cert(name)),
            role,
        }
    }
}

#[derive(Debug)]
struct Clock(AtomicU64);
impl TimeProvider for Clock {
    fn current_time(&self) -> Option<UnixTime> {
        let seconds = self.0.load(Ordering::Relaxed);
        (seconds != 0).then(|| UnixTime::since_unix_epoch(Duration::from_secs(seconds)))
    }
}

fn clock() -> Arc<Clock> {
    Arc::new(Clock(AtomicU64::new(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs())))
}

fn config(credentials: Credentials, clock: Arc<Clock>, limits: Limits) -> Config {
    Config::new(credentials, clock, limits).unwrap()
}

fn client_config(fixture: &Fixture, name: Option<&str>, root: &str) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(fixture.cert(root)).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots);
    let mut config = if let Some(name) = name {
        builder.with_client_auth_cert(vec![fixture.cert(name)], fixture.key(name)).unwrap()
    } else {
        builder.with_no_client_auth()
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

fn client(fixture: &Fixture, name: Option<&str>, root: &str) -> ClientConnection {
    ClientConnection::new(client_config(fixture, name, root), "tls.test".try_into().unwrap())
        .unwrap()
}

fn client_to_server(
    client: &mut ClientConnection,
    server: &mut Session,
    fragment: usize,
) -> Result<(), Error> {
    let mut bytes = Vec::new();
    while client.wants_write() {
        client.write_tls(&mut bytes).unwrap();
    }
    for chunk in bytes.chunks(fragment) {
        server.receive_tls(chunk, 10)?;
    }
    Ok(())
}

fn server_to_client(
    server: &mut Session,
    client: &mut ClientConnection,
    prefix: usize,
) -> Result<(), Error> {
    let mut work = 0;
    loop {
        work += 1;
        assert!(work < 1000, "engine failed to yield");
        match server.step(10)? {
            Event::Transmit => {
                let len = server.outgoing().len().min(prefix);
                let bytes = server.outgoing()[..len].to_vec();
                server.transmitted(len, 10)?;
                client.read_tls(&mut Cursor::new(bytes)).unwrap();
                client.process_new_packets().map_err(Error::Tls)?;
            }
            Event::NeedInput => return Ok(()),
            Event::Progress => continue,
            other => panic!("unexpected during handshake: {other:?}"),
        }
    }
}

fn handshake(
    server: &mut Session,
    client: &mut ClientConnection,
    fragment: usize,
) -> Result<(), Error> {
    for _ in 0..20 {
        let mut wire = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut wire).unwrap();
        }
        for chunk in wire.chunks(fragment) {
            server.receive_tls(chunk, 10)?;
            server_to_client(server, client, fragment)?;
        }
        server_to_client(server, client, fragment)?;
        if !client.is_handshaking() && server.identity().is_some() {
            return Ok(());
        }
    }
    panic!("handshake did not finish");
}

#[test]
fn tls13_authenticates_enrolled_roles_with_fragmented_receipts() {
    let fixture = Fixture::new();
    for (name, role) in [("reader", Role::Reader), ("admin", Role::Administrator)] {
        let mut server =
            Session::new(config(fixture.credentials(), clock(), Limits::DEFAULT), 0).unwrap();
        let mut client = client(&fixture, Some(name), "ca");
        handshake(&mut server, &mut client, 7).unwrap();
        let identity = server.identity().unwrap();
        assert_eq!(identity.certificate_sha256(), fixture.identity(name, role).certificate_sha256);
        assert_eq!(identity.role(), role);
        client.writer().write_all(b"GET /metrics HTTP/1.1\r\n\r\n").unwrap();
        client_to_server(&mut client, &mut server, 1).unwrap();
        let Event::Application {
            identity,
            bytes,
        } = server.step(11).unwrap()
        else {
            panic!("expected plaintext");
        };
        assert_eq!(&*bytes, b"GET /metrics HTTP/1.1\r\n\r\n");
        assert!(identity.allows(Permission::Observe));
        assert_eq!(identity.allows(Permission::Mutate), role == Role::Administrator);
        server.send_application(b"HTTP/1.1 200 OK\r\n\r\nprotected", 12).unwrap();
        let wire = server.outgoing().to_vec();
        assert!(!wire.windows(9).any(|bytes| bytes == b"protected"));
        client.read_tls(&mut Cursor::new(&wire)).unwrap();
        client.process_new_packets().unwrap();
        server.transmitted(wire.len(), 12).unwrap();
        let mut response = [0; 64];
        let len = client.reader().read(&mut response).unwrap();
        assert_eq!(&response[..len], b"HTTP/1.1 200 OK\r\n\r\nprotected");
    }
}

#[test]
fn standard_client_rejects_wrong_server_root_and_name() {
    let fixture = Fixture::new();
    for (root, name) in [("foreign", "tls.test"), ("ca", "substitute.test")] {
        let mut server =
            Session::new(config(fixture.credentials(), clock(), Limits::DEFAULT), 0).unwrap();
        let mut peer = ClientConnection::new(
            client_config(&fixture, Some("reader"), root),
            name.try_into().unwrap(),
        )
        .unwrap();
        assert!(handshake(&mut server, &mut peer, 97).is_err());
        assert_eq!(server.identity(), None);
    }
}

#[test]
fn minimum_fragments_and_maximum_plaintext_co_scale_the_wire_budget() {
    let fixture = Fixture::new();
    let limits = Limits {
        send_fragment_bytes: 32,
        ..Limits::DEFAULT
    };
    let mut server = Session::new(config(fixture.credentials(), clock(), limits), 0).unwrap();
    let mut peer = client(&fixture, Some("reader"), "ca");
    handshake(&mut server, &mut peer, 97).unwrap();
    let expected: Vec<_> = (0..limits.max_plaintext_bytes).map(|index| index as u8).collect();
    server.send_application(&expected, 11).unwrap();
    let wire = server.outgoing().to_vec();
    assert!(wire.len() > expected.len());
    assert!(wire.len() <= limits.max_output_bytes);
    for chunk in wire.chunks(29) {
        peer.read_tls(&mut Cursor::new(chunk)).unwrap();
        peer.process_new_packets().unwrap();
    }
    server.transmitted(wire.len(), 12).unwrap();
    let mut received = vec![0; expected.len()];
    peer.reader().read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
}

#[test]
fn missing_unknown_foreign_and_wrong_purpose_clients_reject_before_plaintext() {
    let fixture = Fixture::new();
    for name in [None, Some("unknown"), Some("foreign"), Some("server")] {
        let mut credentials = fixture.credentials();
        if matches!(name, Some("foreign" | "server")) {
            // Pinning an invalid chain/EKU must not override WebPKI checks.
            credentials.operators.push(fixture.identity(name.unwrap(), Role::Administrator));
        }
        let mut server = Session::new(config(credentials, clock(), Limits::DEFAULT), 0).unwrap();
        let mut peer = client(&fixture, name, "ca");
        assert!(handshake(&mut server, &mut peer, 97).is_err(), "accepted {name:?}");
        assert_eq!(server.identity(), None);
        assert!(server.step(11).is_err());
        assert!(server.receive_tls(b"GET /metrics HTTP/1.1\r\n\r\n", 12).is_err());
    }
}

#[test]
fn removing_enrollment_revokes_reconnect_and_does_not_resume_old_authority() {
    let fixture = Fixture::new();
    let mut credentials = fixture.credentials();
    let old = config(fixture.credentials(), clock(), Limits::DEFAULT);
    let mut first = Session::new(old.clone(), 0).unwrap();
    let peer_config = client_config(&fixture, Some("reader"), "ca");
    let mut peer =
        ClientConnection::new(peer_config.clone(), "tls.test".try_into().unwrap()).unwrap();
    handshake(&mut first, &mut peer, 97).unwrap();
    credentials.operators.retain(|identity| identity.role == Role::Administrator);
    let replacement = config(credentials, clock(), Limits::DEFAULT);
    let mut reconnect = Session::new(replacement, 0).unwrap();
    // Reuse the actual client's config/session cache; a new config must still
    // demand and reject the old reader's certificate, not accept a cached PSK.
    let mut peer = ClientConnection::new(peer_config, "tls.test".try_into().unwrap()).unwrap();
    assert!(handshake(&mut reconnect, &mut peer, 97).is_err());
    let mut admin = client(&fixture, Some("admin"), "ca");
    let mut accepted =
        Session::new(config(fixture.credentials(), clock(), Limits::DEFAULT), 0).unwrap();
    handshake(&mut accepted, &mut admin, 97).unwrap();
}

#[test]
fn plaintext_tamper_and_replay_are_terminal() {
    let fixture = Fixture::new();
    let mut plain =
        Session::new(config(fixture.credentials(), clock(), Limits::DEFAULT), 0).unwrap();
    plain.receive_tls(b"GET /metrics HTTP/1.1\r\n\r\n", 1).unwrap();
    assert!(plain.step(1).is_err());
    for tamper in [false, true] {
        let mut server =
            Session::new(config(fixture.credentials(), clock(), Limits::DEFAULT), 0).unwrap();
        let mut peer = client(&fixture, Some("reader"), "ca");
        handshake(&mut server, &mut peer, 97).unwrap();
        peer.writer().write_all(b"sensitive observation").unwrap();
        let mut frame = Vec::new();
        peer.write_tls(&mut frame).unwrap();
        let original = frame.clone();
        if tamper {
            *frame.last_mut().unwrap() ^= 1;
        }
        server.receive_tls(&frame, 11).unwrap();
        if !tamper {
            assert!(matches!(server.step(11).unwrap(), Event::Application { .. }));
            server.receive_tls(&original, 12).unwrap();
        }
        assert!(server.step(12).is_err());
        assert_eq!(server.identity(), None);
        assert!(server.receive_tls(&original, 13).is_err());
        assert!(server.outgoing().is_empty());
    }
}

#[test]
fn original_deadlines_capacity_and_invalid_receipts_fence_sessions() {
    let fixture = Fixture::new();
    let limits = Limits {
        handshake_ms: 50,
        session_ms: 100,
        ..Limits::DEFAULT
    };
    let config = config(fixture.credentials(), clock(), limits);
    let mut server = Session::new(config.clone(), 0).unwrap();
    assert!(matches!(server.step(49).unwrap(), Event::NeedInput));
    assert!(matches!(server.step(50), Err(Error::HandshakeDeadline)));
    assert!(matches!(server.step(51), Err(Error::HandshakeDeadline)));
    let mut backwards = Session::new(config.clone(), 20).unwrap();
    assert!(matches!(backwards.step(19), Err(Error::MonotonicClock)));
    assert!(Session::new(config.clone(), u64::MAX).is_err());
    let mut full = Session::new(config.clone(), 0).unwrap();
    assert!(matches!(
        full.receive_tls(&vec![0; limits.max_input_bytes + 1], 1),
        Err(Error::Capacity)
    ));
    let mut bad = Session::new(config.clone(), 0).unwrap();
    assert!(matches!(bad.transmitted(1, 1), Err(Error::InvalidReceipt)));
    let mut authenticated = Session::new(config, 0).unwrap();
    let mut peer = client(&fixture, Some("reader"), "ca");
    handshake(&mut authenticated, &mut peer, 97).unwrap();
    assert!(matches!(authenticated.step(100), Err(Error::SessionDeadline)));
}

#[test]
fn configuration_rejects_ambiguous_roles_missing_material_and_unusable_clock() {
    let fixture = Fixture::new();
    let make = |credentials| Config::new(credentials, clock(), Limits::DEFAULT);
    let mut missing = fixture.credentials();
    missing.operators.clear();
    assert!(matches!(make(missing), Err(PolicyError::MissingCredentials)));
    let mut duplicate = fixture.credentials();
    duplicate.operators.push(fixture.identity("reader", Role::Administrator));
    assert!(matches!(make(duplicate), Err(PolicyError::DuplicateEnrollment)));
    let mut bad_key = fixture.credentials();
    bad_key.server_key = fixture.key("reader");
    assert!(matches!(make(bad_key), Err(PolicyError::InvalidServerIdentity)));
    let no_clock = Arc::new(Clock(AtomicU64::new(0)));
    assert!(matches!(
        Config::new(fixture.credentials(), no_clock, Limits::DEFAULT),
        Err(PolicyError::InvalidClock)
    ));
    let limits = Limits {
        max_plaintext_bytes: 16_385,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        Config::new(fixture.credentials(), clock(), limits),
        Err(PolicyError::InvalidLimits)
    ));
    let limits = Limits {
        send_fragment_bytes: 32,
        max_output_bytes: 16_645,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        Config::new(fixture.credentials(), clock(), limits),
        Err(PolicyError::InvalidLimits)
    ));
}

#[test]
fn validity_and_clock_loss_reject_even_pinned_clients() {
    let fixture = Fixture::new();
    for seconds in [1, u64::MAX / 2, 0] {
        let clock = clock();
        let config = config(fixture.credentials(), clock.clone(), Limits::DEFAULT);
        clock.0.store(seconds, Ordering::Relaxed);
        let mut server = Session::new(config, 0).unwrap();
        let mut peer = client(&fixture, Some("reader"), "ca");
        assert!(handshake(&mut server, &mut peer, 97).is_err());
        assert_eq!(server.identity(), None);
    }
}

#[test]
fn openssl_client_interoperates_with_the_freestanding_server_engine() {
    let fixture = Fixture::new();
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let config = config(fixture.credentials(), clock(), Limits::DEFAULT);
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        socket.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut session = Session::new(config, 0).unwrap();
        let mut responded = false;
        loop {
            match session.step(10).unwrap() {
                Event::Transmit => {
                    let bytes = session.outgoing();
                    let len = bytes.len();
                    socket.write_all(bytes).unwrap();
                    session.transmitted(len, 10).unwrap();
                }
                Event::NeedInput if responded => {
                    session.close_notify(10).unwrap();
                }
                Event::NeedInput => {
                    let mut bytes = [0; 4096];
                    let len = socket.read(&mut bytes).unwrap();
                    session.receive_tls(&bytes[..len], 10).unwrap();
                }
                Event::Application {
                    identity,
                    bytes,
                } => {
                    assert_eq!(identity.role(), Role::Reader);
                    assert!(!identity.allows(Permission::Mutate));
                    assert!(bytes.starts_with(b"GET /metrics HTTP/1.1\r\n"));
                    session.send_application(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nprotected", 10).unwrap();
                    responded = true;
                }
                Event::Progress => {}
                Event::Closed => return,
            }
        }
    });
    let mut process = Command::new("openssl")
        .args([
            "s_client",
            "-connect",
            &address.to_string(),
            "-tls1_3",
            "-quiet",
            "-verify_return_error",
            "-verify_hostname",
            "tls.test",
            "-servername",
            "tls.test",
            "-CAfile",
            "ca.pem",
            "-cert",
            "reader.pem",
            "-key",
            "reader.key",
            "-alpn",
            "http/1.1",
        ])
        .current_dir(&fixture.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: tls.test\r\n\r\n")
        .unwrap();
    let result = process.wait_with_output().unwrap();
    server.join().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(result.stdout.ends_with(b"protected"));
}
