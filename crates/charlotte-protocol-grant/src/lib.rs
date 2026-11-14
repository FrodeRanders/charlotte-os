//! Grant-controller wire data; this crate owns no live capabilities.
#![no_std]

pub const INTERFACE: u64 = charlotte_launch::manifest_key(b"GRANT");
pub const VERSION: u32 = 1;
pub const OP_ACQUIRE: u32 = 1;
/// Publish an application-owned endpoint under an exact descriptor grant.
pub const OP_PUBLISH: u32 = 2;

pub const ERR_INVALID: i64 = -1;
pub const ERR_UNAUTHORIZED: i64 = -2;
pub const ERR_UNAVAILABLE: i64 = -3;

pub const REQUEST_MAGIC: u32 = 0x3151_5247; // "GRQ1"
pub const REQUEST_HEADER_LEN: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcquireRequest<'a> {
    pub service: &'a [u8],
    pub rights: u16,
    pub descriptor: &'a [u8],
}

pub fn encode_request(
    service: &[u8],
    rights: u16,
    descriptor: &[u8],
    output: &mut [u8],
) -> Option<usize> {
    if service.is_empty()
        || service.len() > charlotte_launch::deployment::MAX_SERVICE_NAME_LEN
        || rights == 0
        || rights & !charlotte_launch::deployment::ALL_GRANT_RIGHTS != 0
        || charlotte_launch::deployment::decode(descriptor).is_none()
    {
        return None;
    }
    let len = REQUEST_HEADER_LEN.checked_add(service.len())?.checked_add(descriptor.len())?;
    let bytes = output.get_mut(..len)?;
    bytes.fill(0);
    bytes[0..4].copy_from_slice(&REQUEST_MAGIC.to_le_bytes());
    bytes[4..6].copy_from_slice(&(VERSION as u16).to_le_bytes());
    bytes[6..8].copy_from_slice(&rights.to_le_bytes());
    bytes[8..10].copy_from_slice(&(service.len() as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(descriptor.len() as u32).to_le_bytes());
    let service_end = REQUEST_HEADER_LEN + service.len();
    bytes[REQUEST_HEADER_LEN..service_end].copy_from_slice(service);
    bytes[service_end..len].copy_from_slice(descriptor);
    Some(len)
}

pub fn decode_request(bytes: &[u8]) -> Option<AcquireRequest<'_>> {
    if bytes.len() < REQUEST_HEADER_LEN
        || u32::from_le_bytes(bytes[0..4].try_into().ok()?) != REQUEST_MAGIC
        || u16::from_le_bytes(bytes[4..6].try_into().ok()?) != VERSION as u16
        || u16::from_le_bytes(bytes[10..12].try_into().ok()?) != 0
    {
        return None;
    }
    let rights = u16::from_le_bytes(bytes[6..8].try_into().ok()?);
    let service_len = usize::from(u16::from_le_bytes(bytes[8..10].try_into().ok()?));
    let descriptor_len =
        usize::try_from(u32::from_le_bytes(bytes[12..16].try_into().ok()?)).ok()?;
    let service_end = REQUEST_HEADER_LEN.checked_add(service_len)?;
    let descriptor_end = service_end.checked_add(descriptor_len)?;
    if service_len == 0
        || service_len > charlotte_launch::deployment::MAX_SERVICE_NAME_LEN
        || rights == 0
        || rights & !charlotte_launch::deployment::ALL_GRANT_RIGHTS != 0
        || descriptor_end != bytes.len()
    {
        return None;
    }
    let service = bytes.get(REQUEST_HEADER_LEN..service_end)?;
    let descriptor = bytes.get(service_end..descriptor_end)?;
    charlotte_launch::deployment::decode(descriptor)?;
    Some(AcquireRequest {
        service,
        rights,
        descriptor,
    })
}
