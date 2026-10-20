//! Local request bytes retained through service admission and signing.
use alloc::{
    string::String,
    vec::Vec,
};

use charlotte_protocol_s3::ObjectRequest;
use zeroize::Zeroizing;

/// A decoded request with an erasing key owner. Wire input and capability
/// ownership remain with the caller; decoding copies only the validated key.
///
/// ```compile_fail
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<charlotte_s3::request::OwnedRequest>();
/// ```
pub struct OwnedRequest {
    pub flags: u16,
    pub content_length: u64,
    pub range_start: u64,
    pub range_end: u64,
    pub payload_sha256: [u8; 32],
    key: Zeroizing<Vec<u8>>,
}

impl OwnedRequest {
    /// Validate the complete current wire record before allocating/copying.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let request = ObjectRequest::decode(bytes)?;
        let mut key = Zeroizing::new(Vec::with_capacity(request.key.len()));
        key.extend_from_slice(request.key);
        Some(Self {
            flags: request.flags,
            content_length: request.content_length,
            range_start: request.range_start,
            range_end: request.range_end,
            payload_sha256: request.payload_sha256,
            key,
        })
    }

    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Construct the raw path using bucket/prefix fields from the validated
    /// launch profile. This helper grants no object access or signing authority.
    /// Key UTF-8 and segment restrictions match service admission; percent
    /// encoding remains a separate step. Original caller inputs stay borrowed.
    pub fn object_path(&self, bucket: &str, prefix: &str) -> Option<Zeroizing<String>> {
        let key = core::str::from_utf8(&self.key).ok()?;
        if key.starts_with('/')
            || key.as_bytes().contains(&0)
            || key.split('/').any(|segment| segment == "." || segment == "..")
        {
            return None;
        }
        let separator = if prefix.is_empty() {
            ""
        } else {
            "/"
        };
        let parts = ["/", bucket, "/", prefix.trim_end_matches('/'), separator, key];
        let len = parts.iter().try_fold(0usize, |len, part| len.checked_add(part.len()))?;
        let mut path = Zeroizing::new(String::with_capacity(len));
        for part in parts {
            path.push_str(part);
        }
        Some(path)
    }
}
