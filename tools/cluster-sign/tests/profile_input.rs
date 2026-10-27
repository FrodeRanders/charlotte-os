//! Bounded plaintext-profile reads use public fixture data.
use std::io::{
    self,
    Cursor,
    Read,
};

use charlotte_launch::operations::MAX_PROFILE_LEN;
use zeroize::Zeroizing;

#[path = "../src/profile_input.rs"]
mod profile_input;
#[path = "../../../tests/transport/support.rs"]
mod support;
use support::observe;

fn read_profile(reader: impl Read) -> io::Result<profile_input::ProfileInput> {
    profile_input::read_profile_bounded(reader, MAX_PROFILE_LEN)
}

#[test]
fn successful_input_erases_without_growth() {
    let input = b"ZZZZZZZZ-profile";
    let observed = observe(0, 0, 0, || {
        let profile = read_profile(Cursor::new(input)).unwrap();
        assert_eq!(profile.as_bytes(), input);
        drop(profile);
    });
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
    assert!(observed.copy_erased, "plaintext retained: {observed:?}");
}

struct PartialError(bool);
impl Read for PartialError {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Err(io::ErrorKind::Other.into());
        }
        self.0 = true;
        output[..8].fill(b'Z');
        Ok(8)
    }
}

#[test]
fn partial_read_error_erases_copied_plaintext() {
    let observed = observe(0, 0, 0, || {
        assert_eq!(read_profile(PartialError(false)).err().unwrap().kind(), io::ErrorKind::Other);
    });
    assert!(observed.copy_erased, "failed-read prefix retained: {observed:?}");
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0);
}

struct DirtyResult(usize);
impl Read for DirtyResult {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        output.fill(b'Z');
        if self.0 == usize::MAX {
            Err(io::ErrorKind::Other.into())
        } else {
            Ok(self.0)
        }
    }
}

#[test]
fn dirty_error_eof_and_impossible_count_erase_destination() {
    for count in [usize::MAX, 0, MAX_PROFILE_LEN + 2] {
        let observed = observe(0, 0, 128, || {
            assert!(read_profile(DirtyResult(count)).is_err());
        });
        assert!(observed.copy_erased, "dirty destination retained: {observed:?}");
        assert_eq!(observed.copy_releases, 1);
        assert_eq!(observed.growths, 0);
    }
}

struct DirtyShort(bool);
impl Read for DirtyShort {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Ok(0);
        }
        self.0 = true;
        output.fill(b'Z');
        Ok(8)
    }
}

#[test]
fn short_result_lends_only_prefix_and_erases_dirty_tail() {
    let observed = observe(0, 0, 128, || {
        let profile = read_profile(DirtyShort(false)).unwrap();
        assert_eq!(profile.as_bytes(), b"ZZZZZZZZ");
        drop(profile);
    });
    assert!(observed.copy_erased, "unreported tail retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0);
}

#[test]
fn exact_limit_accepts_and_sentinel_rejects_without_growth() {
    for len in [MAX_PROFILE_LEN, MAX_PROFILE_LEN + 1] {
        let input = vec![b'Z'; len];
        let observed = observe(0, 0, 0, || {
            let result = read_profile(Cursor::new(&input));
            if len == MAX_PROFILE_LEN {
                assert_eq!(result.as_ref().unwrap().as_bytes(), input);
            } else {
                assert_eq!(result.as_ref().err().unwrap().kind(), io::ErrorKind::InvalidData);
            }
            drop(result);
        });
        assert!(observed.copy_erased);
        assert_eq!(observed.copy_releases, 1);
        assert_eq!(observed.growths, 0);
    }
    assert_eq!(read_profile(io::empty()).err().unwrap().kind(), io::ErrorKind::InvalidData);
}

struct InterruptedInput(u8);
impl Read for InterruptedInput {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.0 += 1;
        match self.0 {
            1 => {
                output.fill(b'Z');
                Err(io::ErrorKind::Interrupted.into())
            }
            2 => {
                output[..8].fill(b'A');
                Ok(8)
            }
            _ => Ok(0),
        }
    }
}

#[test]
fn interrupted_read_retains_existing_destination() {
    let observed = observe(0, 0, 128, || {
        let profile = read_profile(InterruptedInput(0)).unwrap();
        assert_eq!(profile.as_bytes(), b"AAAAAAAA");
        drop(profile);
    });
    assert!(observed.copy_erased);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0);
}

#[cfg(unix)]
#[test]
fn file_adapter_rejects_unsafe_sources_and_cli_preserves_envelope_contract() {
    use std::{
        fs,
        os::unix::fs::{
            symlink,
            DirBuilderExt,
            PermissionsExt,
        },
        path::PathBuf,
        process::Command,
    };
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let path = std::env::temp_dir().join(format!("charlotte-profile-input-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    let directory = Directory(path);
    let input = directory.0.join("profile");
    let output = directory.0.join("profile.enc");
    let opened = directory.0.join("opened");
    let executable = env!("CARGO_BIN_EXE_cluster-sign");
    let recipient_private = concat!(env!("CARGO_MANIFEST_DIR"), "/dev-recipient-key.hex");
    let signer_private = concat!(env!("CARGO_MANIFEST_DIR"), "/dev-operations-key.hex");
    let recipient_public = directory.0.join("recipient.pub");
    let signer_public = directory.0.join("operations.pub");
    fs::write(
        &recipient_public,
        charlotte_launch::DEVELOPMENT_RECIPIENT_PUBLIC_KEY
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
    .unwrap();
    fs::write(
        &signer_public,
        charlotte_launch::DEVELOPMENT_OPERATIONS_PUBLIC_KEY
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
    .unwrap();
    let cluster = "11".repeat(32);
    let release = "22".repeat(32);
    let seal = |path: &std::path::Path| {
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
                recipient_public.to_str().unwrap(),
                signer_private,
                path.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    for len in [0, MAX_PROFILE_LEN + 1] {
        fs::write(&input, vec![b'Z'; len]).unwrap();
        assert!(profile_input::read_file(input.to_str().unwrap()).is_err());
        assert!(!seal(&input).status.success());
        assert!(!output.exists());
    }
    fs::write(&input, b"ZZZZZZZZ-public-profile").unwrap();
    let alias = directory.0.join("alias");
    symlink(&input, &alias).unwrap();
    assert!(profile_input::read_file(alias.to_str().unwrap()).is_err());
    assert!(!seal(&alias).status.success());
    assert!(!output.exists());
    assert!(profile_input::read_file(directory.0.to_str().unwrap()).is_err());
    let fifo = directory.0.join("fifo");
    let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(profile_input::read_file(fifo.to_str().unwrap()).is_err());
    let plaintext = vec![b'Z'; MAX_PROFILE_LEN];
    fs::write(&input, &plaintext).unwrap();
    fs::set_permissions(&input, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        profile_input::read_bounded_private_file(input.to_str().unwrap(), MAX_PROFILE_LEN)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    fs::set_permissions(&input, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        profile_input::read_bounded_private_file(input.to_str().unwrap(), MAX_PROFILE_LEN)
            .unwrap()
            .as_bytes(),
        plaintext
    );
    assert_eq!(profile_input::read_file(input.to_str().unwrap()).unwrap().as_bytes(), plaintext);
    assert!(seal(&input).status.success());
    let encoded = fs::read(&output).unwrap();
    assert_eq!(
        charlotte_launch::operations::decode(&encoded).unwrap().ciphertext.len(),
        MAX_PROFILE_LEN
    );
    let result = Command::new(executable)
        .args([
            "operations-open",
            output.to_str().unwrap(),
            &cluster,
            &release,
            "500",
            recipient_private,
            signer_public.to_str().unwrap(),
            opened.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(Zeroizing::new(fs::read(&opened).unwrap()).as_slice(), plaintext);
    assert_eq!(fs::metadata(&opened).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!seal(&input).status.success());
    assert_eq!(fs::read(&output).unwrap(), encoded);
}
