//! Streaming host-file hashing with one owned read destination.
use std::io::{
    self,
    Read,
};

use charlotte_launch::sha256::Sha256;
use zeroize::Zeroizing;

pub(super) const CHUNK_LEN: usize = 16 * 1024;

pub(super) fn digest(mut input: impl Read) -> io::Result<[u8; 32]> {
    let mut state = Sha256::new();
    let mut chunk = Zeroizing::new([0u8; CHUNK_LEN]);
    loop {
        let count = match input.read(&mut chunk[..]) {
            Ok(count) if count <= chunk.len() => count,
            Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if count == 0 {
            return Ok(state.finalize());
        }
        state.update(&chunk[..count]);
    }
}
