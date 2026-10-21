//! Erasing text owners for the file-only private-key reader.
use std::io::Read;

use zeroize::Zeroizing;

pub(super) const MAX_KEY_FILE_BYTES: usize = 4096;

/// The caller admits the opened regular-file descriptor first. Keep the
/// existing full-capacity erasing owner across reads, errors and size rejection.
pub(super) fn read_text(reader: impl Read) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let mut contents = Zeroizing::new(Vec::with_capacity(MAX_KEY_FILE_BYTES + 1));
    reader
        .take(MAX_KEY_FILE_BYTES as u64 + 1)
        .read_to_end(&mut contents)
        .map_err(|_| "cannot read private-key file")?;
    if contents.len() > MAX_KEY_FILE_BYTES {
        return Err("private-key file exceeds 4096 bytes");
    }
    Ok(contents)
}

/// Strip blank/comment lines and surrounding whitespace without growing or
/// returning an unprotected copy. The caller retains its original text owner.
pub(super) fn normalize_key_text(contents: &str) -> Zeroizing<String> {
    let mut encoded = Zeroizing::new(String::with_capacity(contents.len()));
    for line in contents.lines().map(str::trim) {
        if !line.is_empty() && !line.starts_with('#') {
            encoded.push_str(line);
        }
    }
    encoded
}
