//! Public key fixtures exercise the host writer's byte owner.
use core::hint::black_box;

use zeroize::Zeroizing;

#[path = "../src/key_encoding.rs"]
mod key_encoding;
#[path = "../../../tests/transport/support.rs"]
mod support;
use key_encoding::encode_key_file;
use support::observe;

#[test]
fn encoded_key_erases_without_newline_growth() {
    for len in [32, 64] {
        let bytes = vec![0x5a; len];
        let observed = observe(0, 0, 0, || {
            let encoded = encode_key_file(&bytes).unwrap();
            assert_eq!(encoded.len(), len * 2 + 1);
            assert!(encoded.ends_with('\n'));
            assert!(encoded[..encoded.len() - 1].bytes().all(|b| matches!(b, b'5' | b'a')));
            drop(black_box(encoded));
        });
        assert_eq!(observed.allocations, 1);
        assert_eq!(observed.growths, 0, "key file text grew: {observed:?}");
        assert_eq!(observed.copy_releases, 1);
        assert!(observed.copy_erased, "key text retained: {observed:?}");
    }
}

#[test]
fn abandoned_key_text_erases_on_consumer_error() {
    fn reject() -> Result<(), ()> {
        let _encoded = black_box(encode_key_file(&[0x5a; 64]).unwrap());
        Err(())
    }
    let observed = observe(0, 0, 0, || assert_eq!(reject(), Err(())));
    assert!(observed.copy_erased, "abandoned key retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn encoded_owner_moves_without_copying() {
    fn consume(encoded: Zeroizing<String>, address: usize) {
        assert_eq!(encoded.as_ptr().addr(), address);
        drop(black_box(encoded));
    }
    let observed = observe(0, 0, 0, || {
        let encoded = encode_key_file(&[0x5a; 32]).unwrap();
        let address = encoded.as_ptr().addr();
        consume(encoded, address);
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn hex_bytes_and_single_newline_are_preserved() {
    for (bytes, expected) in
        [(b"".as_slice(), "\n"), (b"\x00\xab\xff", "00abff\n"), (b"ZZZZZZZZ", "5a5a5a5a5a5a5a5a\n")]
    {
        assert_eq!(encode_key_file(bytes).unwrap().as_str(), expected);
    }
    let all_bytes: Vec<u8> = (0..=255).collect();
    let encoded = encode_key_file(&all_bytes).unwrap();
    let expected = include_str!("fixtures/all-byte-key.hex");
    assert_eq!(encoded.as_str(), expected);
    assert_eq!(all_bytes, (0..=255).collect::<Vec<_>>());
}

#[cfg(unix)]
#[test]
fn key_generation_commands_preserve_files_modes_and_private_output() {
    use std::{
        fs,
        os::unix::fs::{
            DirBuilderExt,
            PermissionsExt,
        },
        path::PathBuf,
        process::Command,
    };
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            // This process created the unique private fixture directory.
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let path = std::env::temp_dir().join(format!("charlotte-key-encoding-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    let directory = Directory(path);
    for (command, private_len) in [
        ("generate", 129),
        ("operations-signing-generate", 129),
        ("operations-recipient-generate", 65),
    ] {
        let private = directory.0.join(format!("{command}.hex"));
        let public = directory.0.join(format!("{command}.pub"));
        let output = Command::new(env!("CARGO_BIN_EXE_cluster-sign"))
            .arg(command)
            .arg(&private)
            .arg(&public)
            .output()
            .unwrap();
        assert!(output.status.success());
        let private_bytes = Zeroizing::new(fs::read(&private).unwrap());
        assert_eq!(private_bytes.len(), private_len);
        assert_eq!(private_bytes.last(), Some(&b'\n'));
        assert!(private_bytes[..private_len - 1]
            .iter()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(fs::metadata(&private).unwrap().permissions().mode() & 0o777, 0o600);
        let public_before = fs::read(&public).unwrap();
        let secret_text = std::str::from_utf8(&private_bytes[..private_len - 1]).unwrap();
        assert!(!String::from_utf8_lossy(&output.stdout).contains(secret_text));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(secret_text));
        let repeated = Command::new(env!("CARGO_BIN_EXE_cluster-sign"))
            .arg(command)
            .arg(&private)
            .arg(&public)
            .output()
            .unwrap();
        assert!(!repeated.status.success());
        assert_eq!(
            Zeroizing::new(fs::read(&private).unwrap()).as_slice(),
            private_bytes.as_slice()
        );
        let rollback = directory.0.join(format!("{command}-rollback.hex"));
        let rejected = Command::new(env!("CARGO_BIN_EXE_cluster-sign"))
            .arg(command)
            .arg(&rollback)
            .arg(&public)
            .output()
            .unwrap();
        assert!(!rejected.status.success());
        assert!(!rollback.exists());
        assert_eq!(fs::read(&public).unwrap(), public_before);
    }
}
