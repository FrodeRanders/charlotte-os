//! Public fixture keys exercise the actual operational CLI's file boundary.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{
        symlink,
        DirBuilderExt,
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        // This owner exists only after exclusive directory creation succeeds.
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn commented_key(bytes: &[u8]) -> String {
    let hex = bytes.iter().map(|byte| format!("{byte:02X}")).collect::<String>();
    let prefix = format!("\n # public fixture\n {hex} \n#");
    format!("{prefix}{}", "x".repeat(4096 - prefix.len()))
}

#[test]
fn operational_commands_share_bounded_public_key_file_admission() {
    let path =
        std::env::temp_dir().join(format!("charlotte-public-key-input-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    let directory = Directory(path);
    let recipient = directory.0.join("recipient.pub");
    let signer = directory.0.join("signer.pub");
    let plaintext = directory.0.join("profile");
    let envelope = directory.0.join("envelope");
    let rejected_output = directory.0.join("rejected-envelope");
    fs::write(&recipient, commented_key(&charlotte_launch::DEVELOPMENT_RECIPIENT_PUBLIC_KEY))
        .unwrap();
    fs::write(&signer, commented_key(&charlotte_launch::DEVELOPMENT_OPERATIONS_PUBLIC_KEY))
        .unwrap();
    fs::write(&plaintext, b"public opaque envelope payload").unwrap();
    let executable = env!("CARGO_BIN_EXE_cluster-sign");
    let private = concat!(env!("CARGO_MANIFEST_DIR"), "/dev-operations-key.hex");
    let cluster = "11".repeat(32);
    let release = "22".repeat(32);
    let seal = |public: &Path, output: &Path| {
        Command::new(executable)
            .args([
                "operations-seal",
                output.to_str().unwrap(),
                "public-profile",
                "s3",
                &cluster,
                &release,
                "1",
                "1000",
                public.to_str().unwrap(),
                private,
                plaintext.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let verify = |public: &Path| {
        Command::new(executable)
            .args(["operations-verify", envelope.to_str().unwrap(), public.to_str().unwrap()])
            .output()
            .unwrap()
    };
    assert!(seal(&recipient, &envelope).status.success());
    assert!(verify(&signer).status.success());
    let original = fs::read(&envelope).unwrap();
    let alias = directory.0.join("alias");
    let oversized = directory.0.join("oversized");
    let fifo = directory.0.join("fifo");
    let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    for (accepted, sealing) in [(&recipient, true), (&signer, false)] {
        symlink(accepted, &alias).unwrap();
        let mut large = fs::read(accepted).unwrap();
        large.push(b'x');
        fs::write(&oversized, large).unwrap();
        for rejected in [&alias, &oversized, &directory.0, &fifo] {
            let result = if sealing {
                seal(rejected, &rejected_output)
            } else {
                verify(rejected)
            };
            assert!(!result.status.success(), "accepted unsafe public key source: {rejected:?}");
            assert!(!rejected_output.exists());
            assert_eq!(fs::read(&envelope).unwrap(), original);
        }
        fs::remove_file(&alias).unwrap();
    }
}
