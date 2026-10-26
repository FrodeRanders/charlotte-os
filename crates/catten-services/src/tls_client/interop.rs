//! Host-only OpenSSL TLS 1.3 interoperability and rejection controls.
//!
//! All roots and keys are fresh public test material. The production handshake
//! owner, verifier, identity parser, cipher and RNG registration are exercised;
//! the entropy slot contains a cfg(test)-only host source, not kernel authority.

extern crate std;

use std::{
    fs,
    io::{
        BufRead,
        BufReader,
    },
    net::{
        TcpListener,
        TcpStream,
    },
    path::PathBuf,
    process::{
        Child,
        Command,
        Stdio,
    },
    string::String,
    sync::mpsc,
    thread::{
        self,
        JoinHandle,
    },
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use super::*;

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(10);

struct Fixture {
    directory: PathBuf,
    child: Option<Child>,
    output: Option<JoinHandle<()>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(output) = self.output.take() {
            output.join().unwrap();
        }
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(std::format!(
            "charlotte-tls-interop-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let fixture = Self {
            directory,
            child: None,
            output: None,
        };
        for name in ["ca", "server", "client"] {
            fixture.openssl(&[
                "ecparam",
                "-name",
                "prime256v1",
                "-genkey",
                "-noout",
                "-out",
                &std::format!("{name}.key"),
            ]);
        }
        fixture.openssl(&[
            "req",
            "-x509",
            "-new",
            "-key",
            "ca.key",
            "-days",
            "36500",
            "-subj",
            "/CN=Charlotte public test root",
            "-out",
            "ca.pem",
        ]);
        for (name, purpose, serial) in
            [("server", "serverAuth", "2"), ("client", "clientAuth", "3")]
        {
            fixture.openssl(&[
                "req",
                "-new",
                "-key",
                &std::format!("{name}.key"),
                "-subj",
                "/CN=tls.test",
                "-out",
                &std::format!("{name}.csr"),
            ]);
            fs::write(
                fixture.directory.join("extensions"),
                std::format!(
                    concat!(
                        "subjectAltName=DNS:tls.test\n",
                        "basicConstraints=critical,CA:FALSE\n",
                        "extendedKeyUsage={purpose}\n"
                    ),
                    purpose = purpose
                ),
            )
            .unwrap();
            fixture.openssl(&[
                "x509",
                "-req",
                "-in",
                &std::format!("{name}.csr"),
                "-CA",
                "ca.pem",
                "-CAkey",
                "ca.key",
                "-set_serial",
                serial,
                "-days",
                "36500",
                "-extfile",
                "extensions",
                "-out",
                &std::format!("{name}.pem"),
            ]);
        }
        fixture.openssl(&[
            "req",
            "-x509",
            "-new",
            "-key",
            "client.key",
            "-days",
            "36500",
            "-subj",
            "/CN=Untrusted public client",
            "-out",
            "untrusted.pem",
        ]);
        for name in ["ca", "server", "client", "untrusted"] {
            fixture.openssl(&[
                "x509",
                "-in",
                &std::format!("{name}.pem"),
                "-outform",
                "DER",
                "-out",
                &std::format!("{name}.der"),
            ]);
        }
        fixture.openssl(&["ec", "-in", "client.key", "-outform", "DER", "-out", "client-key.der"]);
        fixture
    }

    fn openssl(&self, args: &[&str]) {
        let output =
            Command::new("openssl").args(args).current_dir(&self.directory).output().unwrap();
        assert!(
            output.status.success(),
            "openssl {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn bytes(&self, name: &str) -> Vec<u8> {
        fs::read(self.directory.join(name)).unwrap()
    }

    fn connect(&mut self, client_required: bool) -> HostSocket {
        assert!(self.child.is_none());
        let reservation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let mut command = Command::new("openssl");
        command.args([
            "s_server",
            "-accept",
            &std::format!("{address}"),
            "-cert",
            "server.pem",
            "-key",
            "server.key",
            "-tls1_3",
            "-groups",
            "P-256",
            "-ciphersuites",
            "TLS_AES_128_GCM_SHA256",
            "-naccept",
            "1",
            "-www",
        ]);
        if client_required {
            command.args(["-Verify", "1", "-verify_return_error", "-CAfile", "ca.pem"]);
        }
        let mut child = command
            .current_dir(&self.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (ready, notification) = mpsc::channel();
        self.output = Some(thread::spawn(move || {
            let mut ready = Some(ready);
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if line.starts_with("ACCEPT") {
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(());
                    }
                }
            }
        }));
        self.child = Some(child);
        notification.recv_timeout(FIXTURE_TIMEOUT).expect("OpenSSL server readiness");
        let socket = TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(FIXTURE_TIMEOUT)).unwrap();
        socket.set_write_timeout(Some(FIXTURE_TIMEOUT)).unwrap();
        HostSocket(socket)
    }
}

struct HostSocket(TcpStream);
impl ErrorType for HostSocket {
    type Error = TransportError;
}
impl Read for HostSocket {
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, Self::Error> {
        std::io::Read::read(&mut self.0, bytes).map_err(|_| TransportError)
    }
}
impl EmbeddedWrite for HostSocket {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error> {
        std::io::Write::write(&mut self.0, bytes).map_err(|_| TransportError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        std::io::Write::flush(&mut self.0).map_err(|_| TransportError)
    }
}

fn config(ca: &[u8]) -> OpenConfig<'_> {
    OpenConfig {
        server_name: "tls.test",
        ca_certificate_der: ca,
        client_certificate_der: None,
        client_private_key_der: None,
        unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        socket_bounds: SocketBounds {
            send_attempts: 1,
            send_retry_ms: 0,
            receive_attempts: 1,
            receive_retry_ms: 0,
            receive_chunk_len: 4096,
        },
    }
}

fn roundtrip<Socket: Read + EmbeddedWrite>(
    socket: Socket,
    config: &OpenConfig<'_>,
) -> Result<(), OpenError> {
    let prepared = PreparedHandshake::new(config)?;
    let mut read = [0; TLS_RECORD_BUFFER_LEN];
    let mut write = [0; TLS_RECORD_BUFFER_LEN];
    let mut connection = TlsConnection::new(socket, &mut read, &mut write);
    prepared.open(&mut connection).map_err(|error| OpenError::Handshake(tls_error_code(&error)))?;
    connection
        .write(b"GET / HTTP/1.0\r\n\r\n")
        .map_err(|error| OpenError::Handshake(tls_error_code(&error)))?;
    connection.flush().map_err(|error| OpenError::Handshake(tls_error_code(&error)))?;
    let response = read_owned_chunk(&mut connection, 4096).map_err(|_| OpenError::Handshake(1))?;
    assert!(response.starts_with(b"HTTP/1.0 200 ok"));
    let _ = connection.close();
    Ok(())
}

#[derive(Clone, Copy)]
enum Fault {
    TruncatedRecord,
    TamperedHandshake,
    WriteRejected,
}

struct FaultSocket {
    socket: HostSocket,
    fault: Fault,
    received: usize,
}

impl ErrorType for FaultSocket {
    type Error = TransportError;
}
impl Read for FaultSocket {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> {
        if matches!(self.fault, Fault::TruncatedRecord) && self.received == 8 {
            return Ok(0);
        }
        if output.is_empty() {
            return Ok(0);
        }
        // Deliberately fragment this fixture stream to make the wire mutation
        // independent of host packet boundaries. Byte 17 is in ServerHello's
        // random, so changing it invalidates the authenticated transcript.
        let read = self.socket.read(&mut output[..1])?;
        if read != 0 && self.received == 17 && matches!(self.fault, Fault::TamperedHandshake) {
            output[0] ^= 1;
        }
        self.received += read;
        Ok(read)
    }
}
impl EmbeddedWrite for FaultSocket {
    fn write(&mut self, input: &[u8]) -> Result<usize, Self::Error> {
        if matches!(self.fault, Fault::WriteRejected) {
            Err(TransportError)
        } else {
            self.socket.write(input)
        }
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.socket.flush()
    }
}

#[test]
fn openssl_wire_tamper_truncation_and_write_failure_reject() {
    let _serial = super::tests::host_crypto();
    for fault in [Fault::TruncatedRecord, Fault::TamperedHandshake, Fault::WriteRejected] {
        let mut fixture = Fixture::new();
        let ca = fixture.bytes("ca.der");
        let socket = FaultSocket {
            socket: fixture.connect(false),
            fault,
            received: 0,
        };
        let error = roundtrip(socket, &config(&ca)).unwrap_err();
        assert_eq!(error.is_transport_failure(), !matches!(fault, Fault::TamperedHandshake));
        assert_eq!(SynchronizedClock::now(), None);
    }
}

#[test]
fn openssl_server_authentication_and_client_identity_roundtrip() {
    let _serial = super::tests::host_crypto();
    for client_required in [false, true] {
        let mut fixture = Fixture::new();
        let ca = fixture.bytes("ca.der");
        let client = fixture.bytes("client.der");
        let key = Zeroizing::new(fixture.bytes("client-key.der"));
        let mut config = config(&ca);
        if client_required {
            config.client_certificate_der = Some(&client);
            config.client_private_key_der = Some(&key);
        }
        assert_eq!(roundtrip(fixture.connect(client_required), &config), Ok(()));
        assert_eq!(SynchronizedClock::now(), None);
    }
}

#[test]
fn openssl_rejects_wrong_host_root_dates_and_client_identity() {
    let _serial = super::tests::host_crypto();
    for rejection in
        ["host", "root", "expired", "not-yet-valid", "missing-client", "untrusted-client"]
    {
        let mut fixture = Fixture::new();
        let ca = fixture.bytes("ca.der");
        let wrong_ca = fixture.bytes("client.der");
        let untrusted = fixture.bytes("untrusted.der");
        let key = Zeroizing::new(fixture.bytes("client-key.der"));
        let mut config = config(&ca);
        match rejection {
            "host" => config.server_name = "wrong.test",
            "root" => config.ca_certificate_der = &wrong_ca,
            "expired" => config.unix_seconds += 100 * 366 * 86400,
            "not-yet-valid" => config.unix_seconds = 1,
            "untrusted-client" => {
                config.client_certificate_der = Some(&untrusted);
                config.client_private_key_der = Some(&key);
            }
            _ => {}
        }
        let result = roundtrip(fixture.connect(rejection.ends_with("client")), &config);
        assert!(result.is_err(), "{rejection} unexpectedly authenticated");
        if matches!(rejection, "host" | "root" | "expired" | "not-yet-valid") {
            assert!(!result.unwrap_err().is_transport_failure(), "{rejection}");
        }
        assert_eq!(SynchronizedClock::now(), None);
    }
}

#[test]
fn configuration_and_admission_reject_before_handshake() {
    let _serial = super::tests::host_crypto();
    let mut config = config(b"public invalid CA bytes, never parsed in this test");
    config.unix_seconds = 0;
    assert!(matches!(PreparedHandshake::new(&config), Err(OpenError::InvalidConfiguration)));
    assert_eq!(SynchronizedClock::now(), None);
    config.unix_seconds = 42;
    config.client_certificate_der = Some(b"certificate without private key");
    assert!(matches!(PreparedHandshake::new(&config), Err(OpenError::InvalidConfiguration)));
    config.client_private_key_der = Some(b"malformed SEC1");
    assert!(matches!(PreparedHandshake::new(&config), Err(OpenError::InvalidConfiguration)));
    config.client_certificate_der = None;
    config.client_private_key_der = None;
    let first = PreparedHandshake::new(&config).unwrap();
    let competing = OpenConfig {
        unix_seconds: 99,
        ..config
    };
    assert!(matches!(PreparedHandshake::new(&competing), Err(OpenError::CryptoBusy)));
    assert_eq!(SynchronizedClock::now(), Some(42));
    drop(first);
    let entropy = DOMAIN_ENTROPY.checkout().unwrap();
    assert!(matches!(PreparedHandshake::new(&config), Err(OpenError::EntropyUnavailable)));
    assert_eq!(SynchronizedClock::now(), None);
    let mut destination = [0xa5; 32];
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            <SystemRng as embassy_crypto::driver::Rng>::fill_bytes(&mut destination);
        }))
        .is_err()
    );
    assert_eq!(destination, [0xa5; 32]);
    drop(entropy);
    drop(PreparedHandshake::new(&config).unwrap());
    assert_eq!(SynchronizedClock::now(), None);
}
