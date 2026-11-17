//! Bounded launch policy for cluster TCP service identities.
//!
//! The platform, rather than an application, supplies this table.  Each entry
//! binds an externally visible IPv4/TCP identity to the logical artifact whose
//! committed placement and exact-generation readiness authorize new flows.
//! Keeping one canonical encoding prevents DNS, the frame router, and the
//! TCP/IP service from interpreting independently assembled manifest fields.

pub const MAGIC: &[u8; 8] = b"CINGCFG1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 16;
pub const RECORD_HEADER_LEN: usize = 12;
/// The launch config page currently reserves 1024 bytes for manifest data.
pub const MAX_ENCODED_LEN: usize = 1024;
/// A deliberately bounded first transport. The bound follows the launch ABI;
/// it is not a packet-path or cluster-wide architectural limit.
pub const MAX_SERVICES: usize = 16;
pub const PROTOCOL_TCP: u8 = 6;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ServiceId {
    pub address: [u8; 4],
    pub protocol: u8,
    pub port: u16,
}

impl ServiceId {
    pub const fn tcp_v4(address: [u8; 4], port: u16) -> Self {
        Self {
            address,
            protocol: PROTOCOL_TCP,
            port,
        }
    }

    pub fn is_valid(self) -> bool {
        self.address != [0; 4]
            && self.address[0] < 224
            && self.protocol == PROTOCOL_TCP
            && self.port != 0
    }

    /// Compact scalar representation for local DNS membership queries.
    pub const fn pack(self) -> u64 {
        u32::from_be_bytes(self.address) as u64
            | ((self.protocol as u64) << 32)
            | ((self.port as u64) << 40)
    }

    pub fn unpack(value: u64) -> Option<Self> {
        if value >> 56 != 0 {
            return None;
        }
        let service = Self {
            address: (value as u32).to_be_bytes(),
            protocol: (value >> 32) as u8,
            port: (value >> 40) as u16,
        };
        service.is_valid().then_some(service)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceBinding<'a> {
    pub service: ServiceId,
    /// Empty means the platform-service compatibility policy: every admitted,
    /// non-draining member is eligible.
    pub backend_name: Option<&'a [u8]>,
}

impl ServiceBinding<'_> {
    pub fn is_valid(self) -> bool {
        self.service.is_valid()
            && self.backend_name.is_none_or(crate::deployment::valid_artifact_name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    BufferTooSmall,
    DuplicateService,
    InvalidBinding,
    TooLarge,
    TooManyServices,
}

pub fn encoded_len(bindings: &[ServiceBinding<'_>]) -> Result<usize, EncodeError> {
    if bindings.len() > MAX_SERVICES {
        return Err(EncodeError::TooManyServices);
    }
    let mut len = HEADER_LEN;
    for (index, binding) in bindings.iter().copied().enumerate() {
        if !binding.is_valid() {
            return Err(EncodeError::InvalidBinding);
        }
        if bindings[..index].iter().any(|other| other.service == binding.service) {
            return Err(EncodeError::DuplicateService);
        }
        len = len
            .checked_add(RECORD_HEADER_LEN)
            .and_then(|len| len.checked_add(binding.backend_name.map_or(0, <[u8]>::len)))
            .ok_or(EncodeError::TooLarge)?;
    }
    if len > MAX_ENCODED_LEN {
        return Err(EncodeError::TooLarge);
    }
    Ok(len)
}

pub fn encode(bindings: &[ServiceBinding<'_>], output: &mut [u8]) -> Result<usize, EncodeError> {
    let len = encoded_len(bindings)?;
    if output.len() < len {
        return Err(EncodeError::BufferTooSmall);
    }
    let bytes = &mut output[..len];
    bytes.fill(0);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[12..14].copy_from_slice(&(bindings.len() as u16).to_le_bytes());
    let mut offset = HEADER_LEN;
    for binding in bindings {
        let name = binding.backend_name.unwrap_or_default();
        let record_len = RECORD_HEADER_LEN + name.len();
        bytes[offset..offset + 4].copy_from_slice(&binding.service.address);
        bytes[offset + 4] = binding.service.protocol;
        bytes[offset + 6..offset + 8].copy_from_slice(&binding.service.port.to_le_bytes());
        bytes[offset + 8..offset + 10].copy_from_slice(&(name.len() as u16).to_le_bytes());
        bytes[offset + 10..offset + 12].copy_from_slice(&(record_len as u16).to_le_bytes());
        bytes[offset + RECORD_HEADER_LEN..offset + record_len].copy_from_slice(name);
        offset += record_len;
    }
    debug_assert_eq!(offset, len);
    Ok(len)
}

#[derive(Clone, Copy)]
pub struct ServiceBindings<'a> {
    bytes: &'a [u8],
    offset: usize,
    remaining: usize,
}

impl<'a> Iterator for ServiceBindings<'a> {
    type Item = ServiceBinding<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let record_len = usize::from(read_u16(self.bytes, self.offset + 10)?);
        let name_len = usize::from(read_u16(self.bytes, self.offset + 8)?);
        let end = self.offset.checked_add(record_len)?;
        let name_start = self.offset + RECORD_HEADER_LEN;
        let name_end = name_start.checked_add(name_len)?;
        if record_len != RECORD_HEADER_LEN + name_len || end != name_end {
            return None;
        }
        let name = self.bytes.get(name_start..name_end)?;
        let binding = ServiceBinding {
            service: ServiceId {
                address: self.bytes.get(self.offset..self.offset + 4)?.try_into().ok()?,
                protocol: *self.bytes.get(self.offset + 4)?,
                port: read_u16(self.bytes, self.offset + 6)?,
            },
            backend_name: (!name.is_empty()).then_some(name),
        };
        self.offset = end;
        self.remaining -= 1;
        Some(binding)
    }
}

pub fn decode(bytes: &[u8]) -> Option<ServiceBindings<'_>> {
    if bytes.len() < HEADER_LEN
        || bytes.len() > MAX_ENCODED_LEN
        || bytes.get(..8)? != MAGIC
        || read_u16(bytes, 8)? != VERSION
        || usize::from(read_u16(bytes, 10)?) != HEADER_LEN
        || bytes.get(14..16)?.iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let count = usize::from(read_u16(bytes, 12)?);
    if count > MAX_SERVICES {
        return None;
    }
    let mut bindings = ServiceBindings {
        bytes,
        offset: HEADER_LEN,
        remaining: count,
    };
    let mut seen = [None; MAX_SERVICES];
    for index in 0..count {
        let binding = bindings.next()?;
        if !binding.is_valid() || seen[..index].contains(&Some(binding.service)) {
            return None;
        }
        seen[index] = Some(binding.service);
    }
    (bindings.offset == bytes.len() && bindings.remaining == 0).then_some(ServiceBindings {
        bytes,
        offset: HEADER_LEN,
        remaining: count,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}
