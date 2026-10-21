//! Public development fixtures exercise operational-file admission in the CLI.
#![cfg(unix)]

use std::{
    fs,
    net::TcpListener,
    os::unix::fs::{
        symlink,
        DirBuilderExt,
    },
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Output,
    },
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use charlotte_launch::{
    deployment,
    execution_target,
    operations,
    operations_bundle,
    release,
    sha256,
};
use ed25519_compact::{
    KeyPair,
    Seed,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cluster-sign")).args(args).output().unwrap()
}
fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

struct Fixture {
    directory: PathBuf,
    release_key: String,
    cluster: String,
    release_digest: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}
impl Fixture {
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }

    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "charlotte-operational-input-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();
        // The complete fixture owns its directory before any fallible setup.
        let mut fixture = Self {
            directory,
            release_key: String::new(),
            cluster: "11".repeat(32),
            release_digest: String::new(),
        };
        let pair = KeyPair::from_seed(Seed::new([7; 32])); // Public fixture seed.
        fixture.release_key = hex(pair.pk.as_ref());
        let variants = [deployment::ArtifactVariant {
            target: execution_target::ExecutionTarget::baseline(
                execution_target::Architecture::Aarch64,
            ),
            priority: 0,
            artifact_digest: [9; 32],
            object_key: b"connector.elf",
        }];
        let fields = deployment::DescriptorFields {
            variants: &variants,
            sequence: 1,
            node_key: 0,
            artifact_name: b"connector",
            stack_pages_per_thread: charlotte_launch::DEFAULT_USER_STACK_PAGES as u16,
            max_threads: charlotte_launch::DEFAULT_USER_MAX_THREADS as u16,
            shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
            placement: charlotte_launch::placement::PlacementPolicy::singleton(),
            grants: &[],
        };
        let mut descriptor = vec![0; deployment::encoded_len(&fields).unwrap()];
        deployment::encode_unsigned(&fields, &pair.pk, &mut descriptor).unwrap();
        let signature = pair.sk.sign(deployment::signature_digest(&descriptor).unwrap(), None);
        assert!(deployment::set_signature(&mut descriptor, signature.as_ref().try_into().unwrap()));
        let descriptors = [descriptor.as_slice()];
        let fields = release::ReleaseFields {
            sequence: 1,
            release_name: b"public-release",
            descriptors: &descriptors,
        };
        let mut bytes = vec![0; release::encoded_len(&fields).unwrap()];
        release::encode_unsigned(&fields, &pair.pk, &mut bytes).unwrap();
        let signature = pair.sk.sign(release::signature_digest(&bytes).unwrap(), None);
        assert!(release::set_signature(&mut bytes, signature.as_ref().try_into().unwrap()));
        fixture.release_digest = hex(&sha256::digest(&bytes));
        fs::write(fixture.path("release"), bytes).unwrap();
        fs::write(
            fixture.path("recipient.pub"),
            hex(&charlotte_launch::DEVELOPMENT_RECIPIENT_PUBLIC_KEY),
        )
        .unwrap();
        fs::write(
            fixture.path("operations.pub"),
            hex(&charlotte_launch::DEVELOPMENT_OPERATIONS_PUBLIC_KEY),
        )
        .unwrap();
        fs::write(fixture.path("plaintext"), b"public opaque envelope payload").unwrap();
        let result = run(&[
            "operations-seal",
            text(&fixture.path("envelope")),
            "public-profile",
            "s3",
            &fixture.cluster,
            &fixture.release_digest,
            "1",
            "1000",
            text(&fixture.path("recipient.pub")),
            concat!(env!("CARGO_MANIFEST_DIR"), "/dev-operations-key.hex"),
            text(&fixture.path("plaintext")),
        ]);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        fixture
    }

    fn verify(&self, source: &Path) -> Output {
        run(&["operations-verify", text(source), text(&self.path("operations.pub"))])
    }

    fn open(&self, source: &Path, output: &Path) -> Output {
        run(&[
            "operations-open",
            text(source),
            &self.cluster,
            &self.release_digest,
            "500",
            concat!(env!("CARGO_MANIFEST_DIR"), "/dev-recipient-key.hex"),
            text(&self.path("operations.pub")),
            text(output),
        ])
    }

    fn sign_bundle(&self, release: &Path, envelope: &Path, output: &Path) -> Output {
        run(&[
            "operations-bundle-sign",
            text(output),
            "1",
            &self.cluster,
            &self.release_key,
            concat!(env!("CARGO_MANIFEST_DIR"), "/dev-operations-key.hex"),
            text(&self.path("recipient.pub")),
            text(release),
            "connector",
            "profiles/connector.enc",
            text(envelope),
        ])
    }

    fn bundle(&self) -> PathBuf {
        let output = self.path("bundle");
        let result = self.sign_bundle(&self.path("release"), &self.path("envelope"), &output);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        output
    }

    fn verify_bundle(&self, source: &Path) -> Output {
        run(&[
            "operations-bundle-verify",
            text(source),
            &self.cluster,
            &self.release_key,
            text(&self.path("operations.pub")),
            text(&self.path("recipient.pub")),
            "500",
        ])
    }
}

#[test]
fn ordinary_operational_files_preserve_signed_roundtrip() {
    let f = Fixture::new();
    assert!(f.verify(&f.path("envelope")).status.success());
    let opened = f.path("opened");
    assert!(f.open(&f.path("envelope"), &opened).status.success());
    assert_eq!(fs::read(opened).unwrap(), fs::read(f.path("plaintext")).unwrap());
    assert!(f.verify_bundle(&f.bundle()).status.success());
}

#[test]
fn envelope_commands_reject_symlink_sources() {
    let f = Fixture::new();
    let alias = f.path("alias");
    symlink(f.path("envelope"), &alias).unwrap();
    let output = f.path("opened");
    let verify = f.verify(&alias);
    let open = f.open(&alias, &output);
    assert!(
        !verify.status.success() && !open.status.success(),
        "symlink accepted: verify={} open={}",
        verify.status,
        open.status
    );
    assert!(!output.exists());
}

#[test]
fn bundle_commands_reject_symlink_sources() {
    let f = Fixture::new();
    let bundle = f.bundle();
    let alias = f.path("bundle-alias");
    let release_alias = f.path("release-alias");
    let envelope_alias = f.path("envelope-alias");
    symlink(&bundle, &alias).unwrap();
    symlink(f.path("release"), &release_alias).unwrap();
    symlink(f.path("envelope"), &envelope_alias).unwrap();
    let verify = f.verify_bundle(&alias);
    let first = f.path("first");
    let second = f.path("second");
    let sign_release = f.sign_bundle(&release_alias, &f.path("envelope"), &first);
    let sign_envelope = f.sign_bundle(&f.path("release"), &envelope_alias, &second);
    assert!(
        !verify.status.success()
            && !sign_release.status.success()
            && !sign_envelope.status.success(),
        "symlink accepted: verify={} release={} envelope={}",
        verify.status,
        sign_release.status,
        sign_envelope.status
    );
    assert!(!first.exists() && !second.exists());
}

fn rejected_at_file_boundary(output: Output, maximum: usize) {
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(&format!("expected a regular file of at most {maximum} bytes")),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn oversized_operational_files_reject_before_decoding_or_connecting() {
    let f = Fixture::new();
    let envelope = f.path("large-envelope");
    let release = f.path("large-release");
    let bundle = f.path("large-bundle");
    fs::write(&envelope, vec![0; operations::MAX_ENVELOPE_LEN + 1]).unwrap();
    fs::write(&release, vec![0; release::MAX_RELEASE_LEN + 1]).unwrap();
    fs::write(&bundle, vec![0; operations_bundle::MAX_BUNDLE_LEN + 1]).unwrap();
    let output = f.path("rejected-output");
    rejected_at_file_boundary(f.verify(&envelope), operations::MAX_ENVELOPE_LEN);
    rejected_at_file_boundary(f.open(&envelope, &output), operations::MAX_ENVELOPE_LEN);
    rejected_at_file_boundary(f.verify_bundle(&bundle), operations_bundle::MAX_BUNDLE_LEN);
    rejected_at_file_boundary(
        f.sign_bundle(&release, &f.path("envelope"), &output),
        release::MAX_RELEASE_LEN,
    );
    rejected_at_file_boundary(
        f.sign_bundle(&f.path("release"), &envelope, &output),
        operations::MAX_ENVELOPE_LEN,
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    rejected_at_file_boundary(
        run(&[
            "operations-bundle-notify",
            text(&bundle),
            &listener.local_addr().unwrap().to_string(),
        ]),
        operations_bundle::MAX_BUNDLE_LEN,
    );
    assert_eq!(listener.accept().err().unwrap().kind(), std::io::ErrorKind::WouldBlock);
    assert!(!output.exists());
}

#[test]
fn nonregular_operational_files_reject_before_output_or_connect() {
    let f = Fixture::new();
    let fifo = f.path("fifo");
    let name = std::ffi::CString::new(text(&fifo)).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let alias = f.path("bundle-alias");
    symlink(f.bundle(), &alias).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    let output = f.path("rejected-output");
    for source in [&fifo, &f.directory] {
        assert!(!f.verify(source).status.success());
        assert!(!f.open(source, &output).status.success());
        assert!(!f.verify_bundle(source).status.success());
        assert!(!f.sign_bundle(source, &f.path("envelope"), &output).status.success());
        assert!(!f.sign_bundle(&f.path("release"), source, &output).status.success());
        assert!(!run(&["operations-bundle-notify", text(source), &endpoint]).status.success());
    }
    assert!(!run(&["operations-bundle-notify", text(&alias), &endpoint]).status.success());
    assert_eq!(listener.accept().err().unwrap().kind(), std::io::ErrorKind::WouldBlock);
    assert!(!output.exists());
}

#[test]
fn excess_bindings_reject_before_referenced_file_reads() {
    let f = Fixture::new();
    let output = f.path("rejected-output");
    let absent = f.path("absent");
    let recipient = f.path("recipient.pub");
    let mut args = vec![
        "operations-bundle-sign",
        text(&output),
        "1",
        &f.cluster,
        &f.release_key,
        concat!(env!("CARGO_MANIFEST_DIR"), "/dev-operations-key.hex"),
        text(&recipient),
        text(&absent),
    ];
    for _ in 0..=operations_bundle::MAX_BINDINGS {
        args.extend(["connector", "profiles/connector.enc", text(&absent)]);
    }
    let result = run(&args);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("at most 8 bindings"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!output.exists());
}
