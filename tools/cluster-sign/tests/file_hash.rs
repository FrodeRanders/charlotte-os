//! Public byte fixtures check bounded hashing and the actual file-only CLI.
use std::io::{
    self,
    Read,
};
#[path = "../src/file_hash.rs"]
mod file_hash;
#[path = "../../../tests/transport/support.rs"]
mod support;

struct Pattern {
    remaining: usize,
    offset: usize,
    fragment: usize,
}
impl Read for Pattern {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert_eq!(output.len(), file_hash::CHUNK_LEN);
        let count = self.remaining.min(output.len()).min(self.fragment);
        for (i, byte) in output[..count].iter_mut().enumerate() {
            *byte = ((self.offset + i) % 251) as u8;
        }
        self.offset += count;
        self.remaining -= count;
        Ok(count)
    }
}
fn expected(hex: &str) -> [u8; 32] {
    std::array::from_fn(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
}
#[test]
fn streaming_vectors_match_python_without_heap_allocations() {
    for line in
        include_str!("fixtures/file-hash-vectors.txt").lines().filter(|l| !l.starts_with('#'))
    {
        let (length, hex) = line.split_once(' ').unwrap();
        let length = length.parse().unwrap();
        let mut actual = [0; 32];
        let observed = support::observe(0, 0, 0, || {
            actual = file_hash::digest(Pattern {
                remaining: length,
                offset: 0,
                fragment: 113,
            })
            .unwrap();
        });
        assert_eq!(actual, expected(hex));
        assert_eq!(observed.allocations, 0);
        assert_eq!(observed.growths, 0);
    }
}
struct Interrupted(bool);
impl Read for Interrupted {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if !self.0 {
            self.0 = true;
            output.fill(b'Z');
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(0)
    }
}
#[test]
fn interrupted_read_does_not_hash_dirty_destination() {
    assert_eq!(
        file_hash::digest(Interrupted(false)).unwrap(),
        expected("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
}
struct Failing(u8);
impl Read for Failing {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        output.fill(b'Z');
        self.0 += 1;
        match self.0 {
            1 => Ok(8),
            2 => Err(io::ErrorKind::Other.into()),
            _ => unreachable!(),
        }
    }
}
struct Impossible;
impl Read for Impossible {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        Ok(output.len() + 1)
    }
}
#[test]
fn partial_error_and_impossible_count_return_no_digest() {
    assert_eq!(file_hash::digest(Failing(0)).unwrap_err().kind(), io::ErrorKind::Other);
    assert_eq!(file_hash::digest(Impossible).unwrap_err().kind(), io::ErrorKind::InvalidData);
    fn erasing<T: zeroize::ZeroizeOnDrop>() {}
    erasing::<zeroize::Zeroizing<[u8; file_hash::CHUNK_LEN]>>();
}

#[cfg(unix)]
mod cli {
    use std::{
        fs,
        os::unix::fs::{
            symlink,
            DirBuilderExt,
        },
        path::PathBuf,
        process::Command,
        sync::atomic::{
            AtomicUsize,
            Ordering,
        },
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "charlotte-file-hash-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn run(path: &std::path::Path) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_cluster-sign"))
            .args(["sha256", path.to_str().unwrap()])
            .output()
            .unwrap()
    }
    #[test]
    fn cli_regular_files_preserve_exact_digest_and_large_input() {
        let directory = Directory::new();
        let input = directory.0.join("input");
        for length in [0, 4194305] {
            fs::write(&input, (0..length).map(|i| (i % 251) as u8).collect::<Vec<_>>()).unwrap();
            let result = run(&input);
            assert!(result.status.success());
            let line = include_str!("fixtures/file-hash-vectors.txt")
                .lines()
                .find(|l| l.starts_with(&format!("{length} ")))
                .unwrap();
            assert_eq!(
                String::from_utf8(result.stdout).unwrap(),
                format!("{}\n", line.split_once(' ').unwrap().1)
            );
        }
    }
    #[test]
    fn cli_rejects_alias_and_nonregular_sources() {
        let directory = Directory::new();
        let input = directory.0.join("input");
        fs::write(&input, b"public fixture").unwrap();
        let alias = directory.0.join("alias");
        symlink(&input, &alias).unwrap();
        let result = run(&alias);
        assert!(
            !result.status.success(),
            "hash accepted alias: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(result.stdout.is_empty());
        assert!(!run(&directory.0).status.success());
        let fifo = directory.0.join("fifo");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(!run(&fifo).status.success());
    }
}
