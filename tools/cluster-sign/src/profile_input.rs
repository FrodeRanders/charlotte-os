//! Bounded plaintext input for operational-profile encryption.
use std::{
    fs::OpenOptions,
    io::{
        self,
        Read,
    },
};

use charlotte_launch::operations::MAX_PROFILE_LEN;
use zeroize::Zeroizing;

/// Keep the complete initialized destination owned, including bytes a faulty
/// reader writes beyond its reported count. Only the accepted prefix is lent.
pub(super) struct ProfileInput {
    bytes: Zeroizing<Vec<u8>>,
    len: usize,
}

impl ProfileInput {
    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

pub(super) fn read_file(path: &str) -> io::Result<ProfileInput> {
    read_bounded_file(path, MAX_PROFILE_LEN)
}

pub(super) fn read_bounded_file(path: &str, maximum: usize) -> io::Result<ProfileInput> {
    read_file_with_policy(path, maximum, false)
}

pub(super) fn read_bounded_private_file(path: &str, maximum: usize) -> io::Result<ProfileInput> {
    read_file_with_policy(path, maximum, true)
}

fn read_file_with_policy(path: &str, maximum: usize, private: bool) -> io::Result<ProfileInput> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "plaintext profile must be a regular file",
        ));
    }
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "management private input must belong to the effective user with no group/other \
                 permissions",
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    if metadata.len() == 0 || metadata.len() > maximum as u64 {
        return Err(invalid_length());
    }
    read_profile_bounded(file, maximum)
}

/// The file adapter checks its captured descriptor before this byte reader.
/// One extra byte detects growth beyond the envelope's existing profile bound.
pub(super) fn read_profile_bounded(
    mut reader: impl Read,
    maximum: usize,
) -> io::Result<ProfileInput> {
    let capacity = maximum.checked_add(1).filter(|_| maximum != 0).ok_or_else(invalid_length)?;
    let mut bytes = Zeroizing::new(vec![0; capacity]);
    let mut len = 0;
    loop {
        let destination = &mut bytes[len..];
        let count = match reader.read(destination) {
            Ok(count) if count <= destination.len() => count,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid profile read count",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if count == 0 {
            break;
        }
        len += count;
        if len > maximum {
            return Err(invalid_length());
        }
    }
    if len == 0 {
        return Err(invalid_length());
    }
    Ok(ProfileInput {
        bytes,
        len,
    })
}

fn invalid_length() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "plaintext profile length is outside configured bounds",
    )
}
