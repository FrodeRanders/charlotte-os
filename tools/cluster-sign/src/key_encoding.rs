//! Owned text for newly generated key files.
use zeroize::Zeroizing;

/// Reserve the complete lowercase-hex record, including its trailing newline,
/// before copying key bytes. The caller retains its original key owner.
pub(super) fn encode_key_file(bytes: &[u8]) -> Option<Zeroizing<String>> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let len = bytes.len().checked_mul(2)?.checked_add(1)?;
    let mut encoded = Zeroizing::new(String::with_capacity(len));
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 15)] as char);
    }
    encoded.push('\n');
    Some(encoded)
}
