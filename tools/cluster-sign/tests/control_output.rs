//! Public fixtures exercise output admission through the actual signing CLI.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{
        symlink,
        DirBuilderExt,
        OpenOptionsExt,
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

use ed25519_compact::{
    KeyPair,
    Seed,
};

const KINDS: [&str; 3] = ["deployment", "release", "shutdown"];
const MARKER: &[u8] = b"public existing output must survive";

fn run(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cluster-sign")).args(args).output().unwrap()
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().into()
}

struct Fixture {
    directory: PathBuf,
    public: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "charlotte-control-output-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();
        // Own the exclusive fixture directory before any fallible setup.
        let mut fixture = Self {
            directory,
            public: String::new(),
        };
        let pair = KeyPair::from_seed(Seed::new([7; 32])); // Public fixture seed.
        fixture.public = pair.pk.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        let private: String = pair.sk.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        use std::io::Write;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(fixture.path("private.hex"))
            .unwrap()
            .write_all(private.as_bytes())
            .unwrap();
        // Minimal ELF section-table fixture, for host signature metadata only.
        let mut image = vec![0; 193];
        image[..6].copy_from_slice(b"\x7fELF\x02\x01");
        image[18..20].copy_from_slice(&183u16.to_le_bytes());
        image[0x28..0x30].copy_from_slice(&64u64.to_le_bytes());
        image[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        image[0x3c..0x3e].copy_from_slice(&2u16.to_le_bytes());
        image[0x3e..0x40].copy_from_slice(&1u16.to_le_bytes());
        image[132..136].copy_from_slice(&3u32.to_le_bytes());
        image[152..160].copy_from_slice(&192u64.to_le_bytes());
        image[160..168].copy_from_slice(&1u64.to_le_bytes());
        fs::write(fixture.path("artifact.elf"), image).unwrap();
        let result = fixture.sign("deployment", &fixture.path("descriptor"));
        assert!(result.status.success(), "fixture: {result:?}");
        fixture
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }

    fn sign(&self, kind: &str, output: &Path) -> Output {
        let mut args = vec![format!("{kind}-sign"), text(output)];
        match kind {
            "deployment" => args.extend([
                "app".into(),
                "apps/app.elf".into(),
                text(&self.path("artifact.elf")),
                "0".into(),
                "1".into(),
                "4".into(),
                "1".into(),
                "5000".into(),
                text(&self.path("private.hex")),
            ]),
            "release" => args.extend([
                "public-release".into(),
                "1".into(),
                text(&self.path("private.hex")),
                text(&self.path("descriptor")),
            ]),
            "shutdown" => args.extend([
                "1".into(),
                "1".into(),
                "1".into(),
                "301".into(),
                "60000".into(),
                "5000".into(),
                text(&self.path("private.hex")),
            ]),
            _ => unreachable!(),
        }
        run(&args)
    }
}

#[test]
fn new_control_outputs_verify() {
    let fixture = Fixture::new();
    for kind in KINDS {
        let output = fixture.path(kind);
        let signed = fixture.sign(kind, &output);
        assert!(signed.status.success(), "{kind}: {signed:?}");
        let verified = run(&[format!("{kind}-verify"), text(&output), fixture.public.clone()]);
        assert!(verified.status.success(), "{kind}: {verified:?}");
        let bytes = fs::read(&output).unwrap();
        let digest: String =
            charlotte_launch::sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        println!("fresh-output {kind}: bytes={} sha256={digest}", bytes.len());
    }
}

fn rejects_existing_outputs(prepare: impl Fn(&Path, &Path)) {
    let fixture = Fixture::new();
    let mut failures = Vec::new();
    for kind in KINDS {
        let target = fixture.path(&format!("{kind}-target"));
        let output = fixture.path(kind);
        fs::write(&target, MARKER).unwrap();
        prepare(&target, &output);
        let before = fs::symlink_metadata(&output).unwrap();
        let result = fixture.sign(kind, &output);
        let after = fs::symlink_metadata(&output).unwrap();
        use std::os::unix::fs::MetadataExt;
        let preserved = fs::read(&output).unwrap() == MARKER
            && fs::read(&target).unwrap() == MARKER
            && before.ino() == after.ino()
            && before.file_type() == after.file_type();
        if result.status.success() || !preserved || !result.stdout.is_empty() {
            failures.push(format!(
                "{kind}: status={} preserved={preserved} stdout={:?} stderr={:?}",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "output rejection failures: {failures:#?}");
}

#[test]
fn existing_control_outputs_are_preserved() {
    rejects_existing_outputs(|_, output| fs::write(output, MARKER).unwrap());
}

#[test]
fn symlink_control_outputs_are_preserved() {
    rejects_existing_outputs(|target, output| symlink(target, output).unwrap());
}

#[test]
fn hardlink_control_outputs_are_preserved() {
    rejects_existing_outputs(|target, output| fs::hard_link(target, output).unwrap());
}
