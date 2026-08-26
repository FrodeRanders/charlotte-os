//! Wire protocol for the CharlotteOS S3 data-plane service.
//!
//! An S3 service instance is configured for exactly one endpoint, bucket,
//! key prefix, credential identity, and operation policy. Possession of its
//! connection capability is therefore the authority to perform those
//! operations; applications never receive the secret key.
#![no_std]

pub const INTERFACE: u64 = u64::from_le_bytes(*b"S3\0\0\0\0\0\0");
pub const VERSION: u32 = 1;
pub const NAME: u64 = u64::from_le_bytes(*b"s3\0\0\0\0\0\0");

/// Launch-manifest keys. Names are constrained to eight bytes by the common
/// Charlotte launch ABI; these constants keep launchers and the service in
/// lockstep.
pub mod manifest {
    pub const IP: u64 = u64::from_le_bytes(*b"s3_ip\0\0\0");
    pub const HOST: u64 = u64::from_le_bytes(*b"s3_host\0");
    pub const PORT: u64 = u64::from_le_bytes(*b"s3_port\0");
    pub const TLS: u64 = u64::from_le_bytes(*b"s3_tls\0\0");
    /// DER-encoded X.509 trust anchor used to authenticate the endpoint.
    pub const CA_DER: u64 = u64::from_le_bytes(*b"s3_ca\0\0\0");
    pub const REGION: u64 = u64::from_le_bytes(*b"s3regn\0\0");
    pub const BUCKET: u64 = u64::from_le_bytes(*b"s3buck\0\0");
    pub const PREFIX: u64 = u64::from_le_bytes(*b"s3pref\0\0");
    pub const ACCESS_KEY: u64 = u64::from_le_bytes(*b"s3access");
    pub const SECRET_KEY: u64 = u64::from_le_bytes(*b"s3secret");
    pub const NAMESPACE: u64 = u64::from_le_bytes(*b"s3_ns\0\0\0");
    pub const RIGHTS: u64 = u64::from_le_bytes(*b"s3rights");
}

/// Begin a streaming GET. The moved memory contains [`ObjectRequest`]. The
/// result is a positive operation ID and the reply memory contains
/// [`ObjectMetadata`].
pub const OP_GET_BEGIN: u32 = 1;
/// Read the next GET body chunk. `arg0` is the operation ID. A positive result
/// is the byte length in the moved reply page; zero is EOF.
pub const OP_GET_READ: u32 = 2;
/// Close a GET operation before EOF, or release it explicitly after EOF.
pub const OP_GET_CLOSE: u32 = 3;
/// Begin a streaming PUT. The moved memory contains [`ObjectRequest`] with
/// `content_length` and `payload_sha256` set. Result is a positive operation ID.
pub const OP_PUT_BEGIN: u32 = 4;
/// Move a PUT chunk. `arg0` packs operation ID in the low 32 bits and exact
/// byte length in the high 32 bits.
pub const OP_PUT_WRITE: u32 = 5;
/// Finish a PUT. `arg0` is the operation ID; reply memory contains metadata.
pub const OP_PUT_FINISH: u32 = 6;
/// Abort a PUT and release its remote socket.
pub const OP_PUT_ABORT: u32 = 7;
/// HEAD an object. The moved request is returned as [`ObjectMetadata`].
pub const OP_HEAD: u32 = 8;
/// DELETE an object. The moved request returns zero on a 2xx response.
pub const OP_DELETE: u32 = 9;
/// Return a bounded service status/profile summary without credentials.
pub const OP_STATUS: u32 = 10;

pub const RIGHT_GET: u64 = 1 << 0;
pub const RIGHT_PUT: u64 = 1 << 1;
pub const RIGHT_DELETE: u64 = 1 << 2;
pub const RIGHT_LIST: u64 = 1 << 3;

pub const ERR_INVALID: i64 = -1;
pub const ERR_DENIED: i64 = -2;
pub const ERR_UNSYNCHRONIZED: i64 = -3;
pub const ERR_TRANSPORT: i64 = -4;
pub const ERR_PROTOCOL: i64 = -5;
pub const ERR_NOT_FOUND: i64 = -6;
pub const ERR_PRECONDITION: i64 = -7;
pub const ERR_REMOTE: i64 = -8;
pub const ERR_BUSY: i64 = -9;
pub const ERR_TLS_REQUIRED: i64 = -10;
pub const ERR_BAD_OPCODE: i64 = -11;

pub const REQUEST_MAGIC: u32 = 0x3152_3353; // "S3R1" LE
pub const REQUEST_HEADER_LEN: usize = 72;
pub const MAX_KEY_LEN: usize = 3_000;
pub const MAX_CHUNK_LEN: usize = 4_096;

pub const FLAG_IF_NONE_MATCH: u16 = 1 << 0;
pub const FLAG_RANGE: u16 = 1 << 1;
pub const VALID_FLAGS: u16 = FLAG_IF_NONE_MATCH | FLAG_RANGE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectRequest<'a> {
    pub flags: u16,
    pub content_length: u64,
    /// Inclusive range start. Only meaningful with [`FLAG_RANGE`].
    pub range_start: u64,
    /// Exclusive range end; zero means open-ended. Only meaningful with
    /// [`FLAG_RANGE`].
    pub range_end: u64,
    pub payload_sha256: [u8; 32],
    /// Key relative to the service instance's configured prefix.
    pub key: &'a [u8],
}

impl ObjectRequest<'_> {
    pub const fn get(key: &[u8]) -> ObjectRequest<'_> {
        ObjectRequest {
            flags: 0,
            content_length: 0,
            range_start: 0,
            range_end: 0,
            payload_sha256: [0; 32],
            key,
        }
    }

    pub const fn put(
        key: &[u8],
        content_length: u64,
        payload_sha256: [u8; 32],
    ) -> ObjectRequest<'_> {
        ObjectRequest {
            flags: 0,
            content_length,
            range_start: 0,
            range_end: 0,
            payload_sha256,
            key,
        }
    }

    pub const fn with_range(mut self, start: u64, end_exclusive: u64) -> Self {
        self.flags |= FLAG_RANGE;
        self.range_start = start;
        self.range_end = end_exclusive;
        self
    }

    pub const fn create_only(mut self) -> Self {
        self.flags |= FLAG_IF_NONE_MATCH;
        self
    }

    pub fn encoded_len(&self) -> usize {
        REQUEST_HEADER_LEN + self.key.len()
    }

    pub fn encode(&self, output: &mut [u8]) -> Option<usize> {
        let len = self.encoded_len();
        if self.key.is_empty()
            || self.key.len() > MAX_KEY_LEN
            || self.key.len() > u16::MAX as usize
            || self.flags & !VALID_FLAGS != 0
            || self.flags & FLAG_RANGE == 0 && (self.range_start != 0 || self.range_end != 0)
            || output.len() < len
            || self.range_end != 0 && self.range_end <= self.range_start
        {
            return None;
        }
        output[..len].fill(0);
        output[0..4].copy_from_slice(&REQUEST_MAGIC.to_le_bytes());
        output[4..6].copy_from_slice(&(VERSION as u16).to_le_bytes());
        output[6..8].copy_from_slice(&self.flags.to_le_bytes());
        output[8..16].copy_from_slice(&self.content_length.to_le_bytes());
        output[16..24].copy_from_slice(&self.range_start.to_le_bytes());
        output[24..32].copy_from_slice(&self.range_end.to_le_bytes());
        output[32..64].copy_from_slice(&self.payload_sha256);
        output[64..66].copy_from_slice(&(self.key.len() as u16).to_le_bytes());
        output[REQUEST_HEADER_LEN..len].copy_from_slice(self.key);
        Some(len)
    }

    pub fn decode(input: &'_ [u8]) -> Option<ObjectRequest<'_>> {
        if input.len() < REQUEST_HEADER_LEN
            || u32::from_le_bytes(input[0..4].try_into().ok()?) != REQUEST_MAGIC
            || u16::from_le_bytes(input[4..6].try_into().ok()?) != VERSION as u16
        {
            return None;
        }
        let key_len = u16::from_le_bytes(input[64..66].try_into().ok()?) as usize;
        let end = REQUEST_HEADER_LEN.checked_add(key_len)?;
        if key_len == 0 || key_len > MAX_KEY_LEN || end != input.len() {
            return None;
        }
        let request = ObjectRequest {
            flags: u16::from_le_bytes(input[6..8].try_into().ok()?),
            content_length: u64::from_le_bytes(input[8..16].try_into().ok()?),
            range_start: u64::from_le_bytes(input[16..24].try_into().ok()?),
            range_end: u64::from_le_bytes(input[24..32].try_into().ok()?),
            payload_sha256: input[32..64].try_into().ok()?,
            key: &input[REQUEST_HEADER_LEN..end],
        };
        if request.flags & !VALID_FLAGS != 0
            || request.flags & FLAG_RANGE == 0
                && (request.range_start != 0 || request.range_end != 0)
            || request.range_end != 0 && request.range_end <= request.range_start
        {
            return None;
        }
        Some(request)
    }
}

pub const METADATA_MAGIC: u32 = 0x314d_3353; // "S3M1" LE
pub const METADATA_HEADER_LEN: usize = 32;
pub const MAX_ETAG_LEN: usize = 128;
pub const MAX_VERSION_ID_LEN: usize = 256;
pub const MAX_REQUEST_ID_LEN: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectMetadata<'a> {
    pub status: u16,
    pub content_length: u64,
    pub etag: &'a [u8],
    pub version_id: &'a [u8],
    pub request_id: &'a [u8],
}

impl ObjectMetadata<'_> {
    pub fn encoded_len(&self) -> usize {
        METADATA_HEADER_LEN + self.etag.len() + self.version_id.len() + self.request_id.len()
    }

    pub fn encode(&self, output: &mut [u8]) -> Option<usize> {
        let len = self.encoded_len();
        if self.etag.len() > MAX_ETAG_LEN
            || self.version_id.len() > MAX_VERSION_ID_LEN
            || self.request_id.len() > MAX_REQUEST_ID_LEN
            || output.len() < len
        {
            return None;
        }
        output[..len].fill(0);
        output[0..4].copy_from_slice(&METADATA_MAGIC.to_le_bytes());
        output[4..6].copy_from_slice(&(VERSION as u16).to_le_bytes());
        output[6..8].copy_from_slice(&self.status.to_le_bytes());
        output[8..16].copy_from_slice(&self.content_length.to_le_bytes());
        output[16..18].copy_from_slice(&(self.etag.len() as u16).to_le_bytes());
        output[18..20].copy_from_slice(&(self.version_id.len() as u16).to_le_bytes());
        output[20..22].copy_from_slice(&(self.request_id.len() as u16).to_le_bytes());
        let mut offset = METADATA_HEADER_LEN;
        for value in [self.etag, self.version_id, self.request_id] {
            output[offset..offset + value.len()].copy_from_slice(value);
            offset += value.len();
        }
        Some(len)
    }

    pub fn decode(input: &'_ [u8]) -> Option<ObjectMetadata<'_>> {
        if input.len() < METADATA_HEADER_LEN
            || u32::from_le_bytes(input[0..4].try_into().ok()?) != METADATA_MAGIC
            || u16::from_le_bytes(input[4..6].try_into().ok()?) != VERSION as u16
        {
            return None;
        }
        let etag_len = u16::from_le_bytes(input[16..18].try_into().ok()?) as usize;
        let version_len = u16::from_le_bytes(input[18..20].try_into().ok()?) as usize;
        let request_len = u16::from_le_bytes(input[20..22].try_into().ok()?) as usize;
        if etag_len > MAX_ETAG_LEN
            || version_len > MAX_VERSION_ID_LEN
            || request_len > MAX_REQUEST_ID_LEN
        {
            return None;
        }
        let etag_start = METADATA_HEADER_LEN;
        let version_start = etag_start.checked_add(etag_len)?;
        let request_start = version_start.checked_add(version_len)?;
        let end = request_start.checked_add(request_len)?;
        if end > input.len() {
            return None;
        }
        Some(ObjectMetadata {
            status: u16::from_le_bytes(input[6..8].try_into().ok()?),
            content_length: u64::from_le_bytes(input[8..16].try_into().ok()?),
            etag: &input[etag_start..version_start],
            version_id: &input[version_start..request_start],
            request_id: &input[request_start..end],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_request_round_trip() {
        let request = ObjectRequest {
            flags: FLAG_RANGE,
            content_length: 123,
            range_start: 10,
            range_end: 20,
            payload_sha256: [0x5a; 32],
            key: b"reports/hello world.txt",
        };
        let mut bytes = [0u8; 256];
        let len = request.encode(&mut bytes).unwrap();
        assert_eq!(ObjectRequest::decode(&bytes[..len]), Some(request));
    }

    #[test]
    fn metadata_round_trip() {
        let metadata = ObjectMetadata {
            status: 200,
            content_length: 456,
            etag: b"abc",
            version_id: b"v1",
            request_id: b"request",
        };
        let mut bytes = [0u8; 128];
        let len = metadata.encode(&mut bytes).unwrap();
        assert_eq!(ObjectMetadata::decode(&bytes[..len]), Some(metadata));
    }

    #[test]
    fn object_request_rejects_unknown_flags() {
        let mut bytes = [0u8; 128];
        let len = ObjectRequest::get(b"key").encode(&mut bytes).unwrap();
        bytes[6..8].copy_from_slice(&0x8000u16.to_le_bytes());
        assert_eq!(ObjectRequest::decode(&bytes[..len]), None);
    }
}
