//! Complete management TLS startup profiles. Parsing grants no authority;
//! [`sealed`] binds a resource to the already installed admission policy.

mod limits;
pub mod sealed;

/// Native Rustls/P-256 call frames exceed the general four-page service stack.
/// Reserve this centrally for both management services; pages commit on demand.
/// Requalify both architectures when changing providers or this build-time knob.
pub const USER_STACK_PAGES: usize = 32;
const _: () = assert!(USER_STACK_PAGES <= crate::MAX_USER_STACK_PAGES);

pub use limits::Limits;

pub const MAGIC: &[u8; 8] = b"CMGMTL01";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 128;
/// Central boot-resource capacity; unrelated to application request sizes.
pub const MAX_PROFILE_BYTES: usize = 64 * 1024;
const _: () = assert!(MAX_PROFILE_BYTES <= u32::MAX as usize);
pub const LIMIT_NAMES: [&str; 11] = [
    "max_roots",
    "max_enrollments",
    "max_chain_certificates",
    "max_certificate_bytes",
    "max_input_bytes",
    "max_output_bytes",
    "max_plaintext_bytes",
    "send_fragment_bytes",
    "handshake_ms",
    "session_ms",
    "work_per_step",
];
const LIMIT_FIELDS: usize = LIMIT_NAMES.len();
const ENROLLMENT_BYTES: usize = 40;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Reader = 1,
    Administrator = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Enrollment {
    pub certificate_sha256: [u8; 32],
    pub role: Role,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Malformed,
    InvalidLimits,
    Capacity,
    InvalidEnrollment,
    DuplicateEnrollment,
    Allocation,
}

pub struct ProfileInput<'a> {
    pub limits: Limits,
    pub server_chain: &'a [&'a [u8]],
    pub server_key: &'a [u8],
    pub client_roots: &'a [&'a [u8]],
    pub operators: &'a [Enrollment],
}

/// Borrowed, structurally complete profile. This is not a signature receipt.
/// Credential DER semantics are additionally checked by the TLS constructor.
pub struct Profile<'a> {
    pub limits: Limits,
    chain: &'a [u8],
    roots: &'a [u8],
    operators: &'a [u8],
    key: &'a [u8],
}

impl<'a> Profile<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_LEN
            || bytes.len() > MAX_PROFILE_BYTES
            || bytes.get(..8) != Some(MAGIC)
            || u16_at(bytes, 8)? != VERSION
            || usize::from(u16_at(bytes, 10)?) != HEADER_LEN
            || usize::try_from(u32_at(bytes, 12)?).ok() != Some(bytes.len())
            || bytes[116..HEADER_LEN].iter().any(|byte| *byte != 0)
        {
            return Err(Error::Malformed);
        }
        let limits = decode_limits(&bytes[16..104])?.validate()?;
        let chain_count = usize::from(u16_at(bytes, 104)?);
        let root_count = usize::from(u16_at(bytes, 106)?);
        let operator_count = usize::try_from(u32_at(bytes, 108)?).map_err(|_| Error::Capacity)?;
        let key_len = usize::try_from(u32_at(bytes, 112)?).map_err(|_| Error::Capacity)?;
        if chain_count == 0
            || chain_count > limits.max_chain_certificates
            || root_count == 0
            || root_count > limits.max_roots
            || operator_count == 0
            || operator_count > limits.max_enrollments
            || key_len == 0
            || key_len > limits.max_certificate_bytes
        {
            return Err(Error::Capacity);
        }
        let mut cursor = HEADER_LEN;
        let chain =
            take_certificates(bytes, &mut cursor, chain_count, limits.max_certificate_bytes)?;
        let roots =
            take_certificates(bytes, &mut cursor, root_count, limits.max_certificate_bytes)?;
        let operator_bytes = operator_count.checked_mul(ENROLLMENT_BYTES).ok_or(Error::Capacity)?;
        let operators = take(bytes, &mut cursor, operator_bytes)?;
        let key = take(bytes, &mut cursor, key_len)?;
        if cursor != bytes.len() {
            return Err(Error::Malformed);
        }
        let result = Self {
            limits,
            chain,
            roots,
            operators,
            key,
        };
        for (index, entry) in result.operators().enumerate() {
            let entry = entry?;
            if entry.certificate_sha256 == [0; 32] {
                return Err(Error::InvalidEnrollment);
            }
            if result
                .operators()
                .take(index)
                .any(|old| old.is_ok_and(|old| old.certificate_sha256 == entry.certificate_sha256))
            {
                return Err(Error::DuplicateEnrollment);
            }
        }
        Ok(result)
    }

    pub fn server_chain(&self) -> impl Iterator<Item = &'a [u8]> {
        certificates(self.chain)
    }

    pub fn client_roots(&self) -> impl Iterator<Item = &'a [u8]> {
        certificates(self.roots)
    }

    pub fn server_key(&self) -> &'a [u8] {
        self.key
    }

    pub fn operators(&self) -> impl Iterator<Item = Result<Enrollment, Error>> + '_ {
        self.operators.as_chunks::<ENROLLMENT_BYTES>().0.iter().map(|record| {
            if record[33..].iter().any(|byte| *byte != 0) {
                return Err(Error::Malformed);
            }
            let role = match record[32] {
                1 => Role::Reader,
                2 => Role::Administrator,
                _ => return Err(Error::InvalidEnrollment),
            };
            Ok(Enrollment {
                certificate_sha256: record[..32].try_into().unwrap(),
                role,
            })
        })
    }
}

pub fn encoded_len(input: &ProfileInput<'_>) -> Result<usize, Error> {
    input.limits.validate()?;
    if input.server_chain.is_empty()
        || input.server_chain.len() > input.limits.max_chain_certificates
        || input.server_chain.len() > u16::MAX as usize
        || input.client_roots.is_empty()
        || input.client_roots.len() > input.limits.max_roots
        || input.client_roots.len() > u16::MAX as usize
        || input.operators.is_empty()
        || input.operators.len() > input.limits.max_enrollments
        || input.server_key.is_empty()
        || input.server_key.len() > input.limits.max_certificate_bytes
    {
        return Err(Error::Capacity);
    }
    for (index, entry) in input.operators.iter().enumerate() {
        if entry.certificate_sha256 == [0; 32] {
            return Err(Error::InvalidEnrollment);
        }
        if input.operators[..index]
            .iter()
            .any(|old| old.certificate_sha256 == entry.certificate_sha256)
        {
            return Err(Error::DuplicateEnrollment);
        }
    }
    let mut length = HEADER_LEN.checked_add(input.server_key.len()).ok_or(Error::Capacity)?;
    length = length
        .checked_add(input.operators.len().checked_mul(ENROLLMENT_BYTES).ok_or(Error::Capacity)?)
        .ok_or(Error::Capacity)?;
    for cert in input.server_chain.iter().chain(input.client_roots) {
        if cert.is_empty() || cert.len() > input.limits.max_certificate_bytes {
            return Err(Error::Capacity);
        }
        length = length
            .checked_add(4)
            .and_then(|value| value.checked_add(cert.len()))
            .ok_or(Error::Capacity)?;
    }
    if length > MAX_PROFILE_BYTES {
        return Err(Error::Capacity);
    }
    Ok(length)
}

/// The destination already belongs to its caller; secret-bearing callers must
/// put it in an erasing owner before encoding. Validation rejection writes nothing.
pub fn encode(input: &ProfileInput<'_>, output: &mut [u8]) -> Result<usize, Error> {
    let length = encoded_len(input)?;
    if output.len() < length {
        return Err(Error::Capacity);
    }
    let bytes = &mut output[..length];
    bytes.fill(0);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(length as u32).to_le_bytes());
    for (slot, value) in
        bytes[16..104].as_chunks_mut::<8>().0.iter_mut().zip(limit_values(input.limits))
    {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    bytes[104..106].copy_from_slice(&(input.server_chain.len() as u16).to_le_bytes());
    bytes[106..108].copy_from_slice(&(input.client_roots.len() as u16).to_le_bytes());
    bytes[108..112].copy_from_slice(&(input.operators.len() as u32).to_le_bytes());
    bytes[112..116].copy_from_slice(&(input.server_key.len() as u32).to_le_bytes());
    let mut cursor = HEADER_LEN;
    for cert in input.server_chain.iter().chain(input.client_roots) {
        bytes[cursor..cursor + 4].copy_from_slice(&(cert.len() as u32).to_le_bytes());
        cursor += 4;
        bytes[cursor..cursor + cert.len()].copy_from_slice(cert);
        cursor += cert.len();
    }
    for entry in input.operators {
        bytes[cursor..cursor + 32].copy_from_slice(&entry.certificate_sha256);
        bytes[cursor + 32] = entry.role as u8;
        cursor += ENROLLMENT_BYTES;
    }
    bytes[cursor..].copy_from_slice(input.server_key);
    Ok(length)
}

pub fn limit_values(limits: Limits) -> [u64; LIMIT_FIELDS] {
    [
        limits.max_roots as u64,
        limits.max_enrollments as u64,
        limits.max_chain_certificates as u64,
        limits.max_certificate_bytes as u64,
        limits.max_input_bytes as u64,
        limits.max_output_bytes as u64,
        limits.max_plaintext_bytes as u64,
        limits.send_fragment_bytes as u64,
        limits.handshake_ms,
        limits.session_ms,
        limits.work_per_step as u64,
    ]
}

fn decode_limits(bytes: &[u8]) -> Result<Limits, Error> {
    let mut values = [0u64; LIMIT_FIELDS];
    for (slot, value) in values.iter_mut().zip(bytes.as_chunks::<8>().0.iter()) {
        *slot = u64::from_le_bytes(*value);
    }
    limits_from_values(values)
}

pub fn limits_from_values(values: [u64; LIMIT_FIELDS]) -> Result<Limits, Error> {
    let size = |index: usize| usize::try_from(values[index]).map_err(|_| Error::Capacity);
    Ok(Limits {
        max_roots: size(0)?,
        max_enrollments: size(1)?,
        max_chain_certificates: size(2)?,
        max_certificate_bytes: size(3)?,
        max_input_bytes: size(4)?,
        max_output_bytes: size(5)?,
        max_plaintext_bytes: size(6)?,
        send_fragment_bytes: size(7)?,
        handshake_ms: values[8],
        session_ms: values[9],
        work_per_step: size(10)?,
    })
}
fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], Error> {
    let end = cursor.checked_add(length).ok_or(Error::Capacity)?;
    let result = bytes.get(*cursor..end).ok_or(Error::Malformed)?;
    *cursor = end;
    Ok(result)
}
fn take_certificates<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    count: usize,
    maximum: usize,
) -> Result<&'a [u8], Error> {
    let start = *cursor;
    for _ in 0..count {
        let length = usize::try_from(u32_at(bytes, *cursor)?).map_err(|_| Error::Capacity)?;
        take(bytes, cursor, 4)?;
        if length == 0 || length > maximum {
            return Err(Error::Capacity);
        }
        take(bytes, cursor, length)?;
    }
    Ok(&bytes[start..*cursor])
}
fn certificates(mut bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    core::iter::from_fn(move || {
        if bytes.is_empty() {
            return None;
        }
        // Only validated ranges from Profile::decode construct this iterator.
        let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let cert = &bytes[4..4 + length];
        bytes = &bytes[4 + length..];
        Some(cert)
    })
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        bytes.get(offset..offset + 2).ok_or(Error::Malformed)?.try_into().unwrap(),
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        bytes.get(offset..offset + 4).ok_or(Error::Malformed)?.try_into().unwrap(),
    ))
}
