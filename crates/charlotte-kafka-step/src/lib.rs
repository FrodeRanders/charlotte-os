//! Bounded profile and procedure ABI for a generic transactional Kafka step.
#![no_std]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};

use charlotte_protocol_kafka::{
    MAX_KEY_BYTES,
    MAX_RECORD_BYTES,
};

pub const INTERFACE: u64 = u64::from_le_bytes(*b"KSTEP\0\0\0");
pub const VERSION: u32 = 1;
pub const OP_INVOKE: u32 = 1;

/// The procedure asked for redelivery after the configured backoff.
pub const RESULT_RETRY: i64 = -1;
/// The procedure rejected the input permanently; route it to the DLQ now.
pub const RESULT_TERMINAL: i64 = -2;
pub const RESULT_INVALID: i64 = -3;

pub const MAX_NAME_BYTES: usize = 256;
pub const MAX_ALLOWED_ROUTES: usize = 64;
pub const MAX_OUTPUTS: usize = 16;
pub const MAX_TRACKED_ATTEMPTS: usize = 32;
pub const MAX_PROCEDURE_TIMEOUT_MS: u32 = 10 * 60 * 1_000;

pub const PROFILE_MAGIC: [u8; 8] = *b"CHKSTP1\0";
pub const PROFILE_VERSION: u16 = 1;
pub const PROFILE_HEADER_LEN: usize = 80;
pub const PROFILE_DIGEST_OFFSET: usize = 16;
pub const PROFILE_DIGEST_LEN: usize = 32;
pub const MAX_PROFILE_BYTES: usize = 1_024;

pub const OUTPUT_MAGIC: u32 = 0x3154_554f; // "OUT1" LE
pub const OUTPUT_VERSION: u16 = 1;
pub const OUTPUT_HEADER_LEN: usize = 16;
pub const OUTPUT_DESCRIPTOR_LEN: usize = 16;
pub const OUTPUT_FLAG_NULL_KEY: u16 = 1;
pub const OUTPUT_FLAG_NULL_VALUE: u16 = 2;
const OUTPUT_VALID_FLAGS: u16 = OUTPUT_FLAG_NULL_KEY | OUTPUT_FLAG_NULL_VALUE;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile<'a> {
    pub procedure_name: &'a [u8],
    pub kafka_connector_name: &'a [u8],
    pub allowed_routes: Vec<u16>,
    pub dlq_route: u16,
    pub max_outputs: u16,
    pub max_attempts: u16,
    pub procedure_timeout_ms: u32,
    pub retry_backoff_ms: u32,
    pub idle_poll_ms: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileError {
    TooLarge,
    InvalidHeader,
    UnsupportedVersion,
    DigestMismatch,
    InvalidField,
    DuplicateRoute,
}

impl Profile<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, ProfileError> {
        validate_profile(self)?;
        let total_len = PROFILE_HEADER_LEN
            .checked_add(self.procedure_name.len())
            .and_then(|len| len.checked_add(self.kafka_connector_name.len()))
            .and_then(|len| len.checked_add(self.allowed_routes.len() * 2))
            .ok_or(ProfileError::TooLarge)?;
        if total_len > MAX_PROFILE_BYTES {
            return Err(ProfileError::TooLarge);
        }
        let mut output = alloc::vec![0; total_len];
        output[..8].copy_from_slice(&PROFILE_MAGIC);
        put_u16(&mut output, 8, PROFILE_VERSION);
        put_u16(&mut output, 10, PROFILE_HEADER_LEN as u16);
        put_u32(&mut output, 12, total_len as u32);
        put_u16(&mut output, 48, self.procedure_name.len() as u16);
        put_u16(&mut output, 50, self.kafka_connector_name.len() as u16);
        put_u16(&mut output, 52, self.allowed_routes.len() as u16);
        put_u16(&mut output, 54, self.max_outputs);
        put_u16(&mut output, 56, self.max_attempts);
        put_u16(&mut output, 58, self.dlq_route);
        put_u32(&mut output, 60, self.procedure_timeout_ms);
        put_u32(&mut output, 64, self.retry_backoff_ms);
        put_u32(&mut output, 68, self.idle_poll_ms);
        let mut offset = PROFILE_HEADER_LEN;
        for field in [self.procedure_name, self.kafka_connector_name] {
            output[offset..offset + field.len()].copy_from_slice(field);
            offset += field.len();
        }
        for route in &self.allowed_routes {
            put_u16(&mut output, offset, *route);
            offset += 2;
        }
        let digest = charlotte_launch::sha256::digest(&output);
        output[PROFILE_DIGEST_OFFSET..PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN]
            .copy_from_slice(&digest);
        Ok(output)
    }

    pub fn decode(input: &'_ [u8]) -> Result<Profile<'_>, ProfileError> {
        if input.len() < PROFILE_HEADER_LEN
            || input.len() > MAX_PROFILE_BYTES
            || input[..8] != PROFILE_MAGIC
        {
            return Err(ProfileError::InvalidHeader);
        }
        if get_u16(input, 8)? != PROFILE_VERSION {
            return Err(ProfileError::UnsupportedVersion);
        }
        if get_u16(input, 10)? as usize != PROFILE_HEADER_LEN
            || get_u32(input, 12)? as usize != input.len()
        {
            return Err(ProfileError::InvalidHeader);
        }
        let expected: [u8; 32] = input
            [PROFILE_DIGEST_OFFSET..PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN]
            .try_into()
            .map_err(|_| ProfileError::InvalidHeader)?;
        let mut hasher = charlotte_launch::sha256::Sha256::new();
        hasher.update(&input[..PROFILE_DIGEST_OFFSET]);
        hasher.update(&[0; PROFILE_DIGEST_LEN]);
        hasher.update(&input[PROFILE_DIGEST_OFFSET + PROFILE_DIGEST_LEN..]);
        if hasher.finalize() != expected {
            return Err(ProfileError::DigestMismatch);
        }
        if input[72..80] != [0; 8] {
            return Err(ProfileError::InvalidField);
        }
        let procedure_len = get_u16(input, 48)? as usize;
        let kafka_len = get_u16(input, 50)? as usize;
        let route_count = get_u16(input, 52)? as usize;
        if route_count > MAX_ALLOWED_ROUTES {
            return Err(ProfileError::InvalidField);
        }
        let mut offset = PROFILE_HEADER_LEN;
        let procedure_end = offset.checked_add(procedure_len).ok_or(ProfileError::TooLarge)?;
        let procedure_name = input.get(offset..procedure_end).ok_or(ProfileError::InvalidField)?;
        offset = procedure_end;
        let kafka_end = offset.checked_add(kafka_len).ok_or(ProfileError::TooLarge)?;
        let kafka_connector_name =
            input.get(offset..kafka_end).ok_or(ProfileError::InvalidField)?;
        offset = kafka_end;
        let mut allowed_routes = Vec::with_capacity(route_count);
        for _ in 0..route_count {
            allowed_routes.push(get_u16(input, offset)?);
            offset += 2;
        }
        if offset != input.len() {
            return Err(ProfileError::InvalidField);
        }
        let profile = Profile {
            procedure_name,
            kafka_connector_name,
            allowed_routes,
            dlq_route: get_u16(input, 58)?,
            max_outputs: get_u16(input, 54)?,
            max_attempts: get_u16(input, 56)?,
            procedure_timeout_ms: get_u32(input, 60)?,
            retry_backoff_ms: get_u32(input, 64)?,
            idle_poll_ms: get_u32(input, 68)?,
        };
        validate_profile(&profile)?;
        Ok(profile)
    }
}

fn validate_profile(profile: &Profile<'_>) -> Result<(), ProfileError> {
    if profile.procedure_name.is_empty()
        || profile.procedure_name.len() > MAX_NAME_BYTES
        || profile.kafka_connector_name.is_empty()
        || profile.kafka_connector_name.len() > MAX_NAME_BYTES
        || core::str::from_utf8(profile.procedure_name).is_err()
        || core::str::from_utf8(profile.kafka_connector_name).is_err()
        || profile.allowed_routes.is_empty()
        || profile.allowed_routes.len() > MAX_ALLOWED_ROUTES
        || profile.max_outputs == 0
        || usize::from(profile.max_outputs) > MAX_OUTPUTS
        || profile.max_attempts == 0
        || profile.procedure_timeout_ms == 0
        || profile.procedure_timeout_ms > MAX_PROCEDURE_TIMEOUT_MS
        || profile.retry_backoff_ms == 0
        || profile.idle_poll_ms == 0
        || !profile.allowed_routes.contains(&profile.dlq_route)
    {
        return Err(ProfileError::InvalidField);
    }
    for (index, route) in profile.allowed_routes.iter().enumerate() {
        if profile.allowed_routes[..index].contains(route) {
            return Err(ProfileError::DuplicateRoute);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputRecord<'a> {
    pub route: u16,
    pub key: Option<&'a [u8]>,
    pub value: Option<&'a [u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputBatch<'a> {
    pub records: Vec<OutputRecord<'a>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputError {
    TooLarge,
    Invalid,
    TooManyRecords,
}

impl OutputBatch<'_> {
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, OutputError> {
        if self.records.len() > MAX_OUTPUTS {
            return Err(OutputError::TooManyRecords);
        }
        let mut data_offset = OUTPUT_HEADER_LEN + self.records.len() * OUTPUT_DESCRIPTOR_LEN;
        if data_offset > output.len() || data_offset > u16::MAX as usize {
            return Err(OutputError::TooLarge);
        }
        output[..data_offset].fill(0);
        put_u32(output, 0, OUTPUT_MAGIC);
        put_u16(output, 4, OUTPUT_VERSION);
        put_u16(output, 6, self.records.len() as u16);
        for (index, record) in self.records.iter().enumerate() {
            if record.key.is_some_and(|key| key.len() > MAX_KEY_BYTES)
                || record.value.is_some_and(|value| value.len() > MAX_RECORD_BYTES)
            {
                return Err(OutputError::TooLarge);
            }
            let descriptor = OUTPUT_HEADER_LEN + index * OUTPUT_DESCRIPTOR_LEN;
            put_u16(output, descriptor, record.route);
            let mut flags = 0;
            for (value, null_flag, offset_slot, len_slot) in [
                (record.key, OUTPUT_FLAG_NULL_KEY, descriptor + 4, descriptor + 6),
                (record.value, OUTPUT_FLAG_NULL_VALUE, descriptor + 8, descriptor + 10),
            ] {
                if let Some(value) = value {
                    let end = data_offset.checked_add(value.len()).ok_or(OutputError::TooLarge)?;
                    if end > output.len() || end > u16::MAX as usize {
                        return Err(OutputError::TooLarge);
                    }
                    put_u16(output, offset_slot, data_offset as u16);
                    put_u16(output, len_slot, value.len() as u16);
                    output[data_offset..end].copy_from_slice(value);
                    data_offset = end;
                } else {
                    flags |= null_flag;
                }
            }
            put_u16(output, descriptor + 2, flags);
        }
        put_u32(output, 8, data_offset as u32);
        Ok(data_offset)
    }

    pub fn decode<'a>(input: &'a [u8]) -> Result<OutputBatch<'a>, OutputError> {
        if input.len() < OUTPUT_HEADER_LEN
            || get_u32_output(input, 0)? != OUTPUT_MAGIC
            || get_u16_output(input, 4)? != OUTPUT_VERSION
            || input[12..16] != [0; 4]
        {
            return Err(OutputError::Invalid);
        }
        let count = get_u16_output(input, 6)? as usize;
        let total_len = get_u32_output(input, 8)? as usize;
        let data_start = OUTPUT_HEADER_LEN
            .checked_add(count * OUTPUT_DESCRIPTOR_LEN)
            .ok_or(OutputError::TooLarge)?;
        if count > MAX_OUTPUTS || data_start > total_len || total_len != input.len() {
            return Err(OutputError::Invalid);
        }
        let mut records = Vec::with_capacity(count);
        for index in 0..count {
            let descriptor = OUTPUT_HEADER_LEN + index * OUTPUT_DESCRIPTOR_LEN;
            let flags = get_u16_output(input, descriptor + 2)?;
            if flags & !OUTPUT_VALID_FLAGS != 0 || input[descriptor + 12..descriptor + 16] != [0; 4]
            {
                return Err(OutputError::Invalid);
            }
            let key = decode_field(
                input,
                data_start,
                flags & OUTPUT_FLAG_NULL_KEY != 0,
                descriptor + 4,
                descriptor + 6,
                MAX_KEY_BYTES,
            )?;
            let value = decode_field(
                input,
                data_start,
                flags & OUTPUT_FLAG_NULL_VALUE != 0,
                descriptor + 8,
                descriptor + 10,
                MAX_RECORD_BYTES,
            )?;
            records.push(OutputRecord {
                route: get_u16_output(input, descriptor)?,
                key,
                value,
            });
        }
        Ok(OutputBatch {
            records,
        })
    }
}

fn decode_field(
    input: &[u8],
    data_start: usize,
    is_null: bool,
    offset_slot: usize,
    len_slot: usize,
    maximum: usize,
) -> Result<Option<&[u8]>, OutputError> {
    let offset = get_u16_output(input, offset_slot)? as usize;
    let len = get_u16_output(input, len_slot)? as usize;
    if is_null {
        return (offset == 0 && len == 0).then_some(None).ok_or(OutputError::Invalid);
    }
    let end = offset.checked_add(len).ok_or(OutputError::TooLarge)?;
    if offset < data_start || len > maximum {
        return Err(OutputError::Invalid);
    }
    Ok(Some(input.get(offset..end).ok_or(OutputError::Invalid)?))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureAction {
    Retry,
    DeadLetter,
}

/// Bounded retry state keyed by the immutable Kafka partition/offset identity.
pub struct AttemptTracker {
    attempts: BTreeMap<(i32, i64), u16>,
}

impl AttemptTracker {
    pub const fn new() -> Self {
        Self {
            attempts: BTreeMap::new(),
        }
    }

    pub fn attempt(&mut self, partition: i32, offset: i64) -> Option<u16> {
        let key = (partition, offset);
        if let Some(attempt) = self.attempts.get(&key) {
            return Some(*attempt);
        }
        if self.attempts.len() >= MAX_TRACKED_ATTEMPTS {
            return None;
        }
        self.attempts.insert(key, 1);
        Some(1)
    }

    pub fn fail(&mut self, partition: i32, offset: i64, maximum: u16) -> FailureAction {
        let attempt = self.attempts.entry((partition, offset)).or_insert(1);
        if *attempt >= maximum {
            FailureAction::DeadLetter
        } else {
            *attempt += 1;
            FailureAction::Retry
        }
    }

    pub fn complete(&mut self, partition: i32, offset: i64) {
        self.attempts.remove(&(partition, offset));
    }
}

impl Default for AttemptTracker {
    fn default() -> Self {
        Self::new()
    }
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(input: &[u8], offset: usize) -> Result<u16, ProfileError> {
    Ok(u16::from_le_bytes(
        input.get(offset..offset + 2).ok_or(ProfileError::InvalidHeader)?.try_into().unwrap(),
    ))
}

fn get_u32(input: &[u8], offset: usize) -> Result<u32, ProfileError> {
    Ok(u32::from_le_bytes(
        input.get(offset..offset + 4).ok_or(ProfileError::InvalidHeader)?.try_into().unwrap(),
    ))
}

fn get_u16_output(input: &[u8], offset: usize) -> Result<u16, OutputError> {
    Ok(u16::from_le_bytes(
        input.get(offset..offset + 2).ok_or(OutputError::Invalid)?.try_into().unwrap(),
    ))
}

fn get_u32_output(input: &[u8], offset: usize) -> Result<u32, OutputError> {
    Ok(u32::from_le_bytes(
        input.get(offset..offset + 4).ok_or(OutputError::Invalid)?.try_into().unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_round_trip_and_digest() {
        let profile = Profile {
            procedure_name: b"claim-procedure",
            kafka_connector_name: b"kafka/claims/step",
            allowed_routes: alloc::vec![1, 2, 7],
            dlq_route: 7,
            max_outputs: 8,
            max_attempts: 3,
            procedure_timeout_ms: 5_000,
            retry_backoff_ms: 250,
            idle_poll_ms: 50,
        };
        let encoded = profile.encode().unwrap();
        assert_eq!(Profile::decode(&encoded).unwrap(), profile);
        let mut corrupt = encoded;
        *corrupt.last_mut().unwrap() ^= 1;
        assert_eq!(Profile::decode(&corrupt), Err(ProfileError::DigestMismatch));
    }

    #[test]
    fn output_batch_round_trip_preserves_null_and_empty() {
        let batch = OutputBatch {
            records: alloc::vec![
                OutputRecord {
                    route: 1,
                    key: None,
                    value: Some(b"approved"),
                },
                OutputRecord {
                    route: 2,
                    key: Some(b""),
                    value: None,
                },
            ],
        };
        let mut encoded = [0; 256];
        let len = batch.encode(&mut encoded).unwrap();
        assert_eq!(OutputBatch::decode(&encoded[..len]).unwrap(), batch);
    }

    #[test]
    fn attempts_reach_dlq_without_unbounded_state() {
        let mut tracker = AttemptTracker::new();
        assert_eq!(tracker.attempt(0, 42), Some(1));
        assert_eq!(tracker.fail(0, 42, 3), FailureAction::Retry);
        assert_eq!(tracker.attempt(0, 42), Some(2));
        assert_eq!(tracker.fail(0, 42, 3), FailureAction::Retry);
        assert_eq!(tracker.fail(0, 42, 3), FailureAction::DeadLetter);
        tracker.complete(0, 42);
        assert_eq!(tracker.attempt(0, 42), Some(1));
    }
}
