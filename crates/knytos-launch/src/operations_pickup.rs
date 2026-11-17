//! Bounded wire contracts between the replicated operational catalog, the
//! node reconciler, and the kernel's privileged connector-launch gate.
//!
//! These records contain references and authenticated digests only. The
//! encrypted envelope is fetched separately and plaintext is never returned
//! through this catalog protocol.

use crate::{
    deployment,
    operations,
    operations_bundle,
    release,
};

pub const LIST_MAGIC: &[u8; 8] = b"COPSLST1";
pub const LIST_VERSION: u16 = 1;
pub const LIST_HEADER_LEN: usize = 16;
pub const RECORD_HEADER_LEN: usize = 256;

pub const PICKUP_MAGIC: &[u8; 8] = b"COPSPK01";
pub const PICKUP_VERSION: u16 = 1;
pub const PICKUP_HEADER_LEN: usize = 288;
pub const MAX_PICKUP_LEN: usize = PICKUP_HEADER_LEN
    + release::MAX_RELEASE_NAME_LEN
    + operations::MAX_PROFILE_NAME_LEN
    + deployment::MAX_ARTIFACT_NAME_LEN
    + operations_bundle::MAX_OBJECT_KEY_LEN
    + release::MAX_RELEASE_LEN
    + crate::MAX_ARTIFACT_ELF_SIZE
    + deployment::MAX_DESCRIPTOR_LEN
    + operations::MAX_ENVELOPE_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogBinding<'a> {
    pub generation: u64,
    pub bundle_sequence: u64,
    pub sequence: u64,
    pub expires_unix_seconds: u64,
    pub profile_kind: u16,
    pub release_name: &'a [u8],
    pub profile_name: &'a [u8],
    pub target_artifact: &'a [u8],
    pub object_key: &'a [u8],
    pub release_digest: [u8; 32],
    pub bundle_digest: [u8; 32],
    pub envelope_digest: [u8; 32],
    pub recipient_key_id: [u8; operations::KEY_ID_LEN],
    pub signing_key_id: [u8; operations::KEY_ID_LEN],
    pub authorization_signature: [u8; operations_bundle::BINDING_SIGNATURE_LEN],
}

impl CatalogBinding<'_> {
    fn encoded_len(&self) -> Option<usize> {
        if self.generation == 0
            || self.bundle_sequence == 0
            || self.sequence == 0
            || self.expires_unix_seconds == 0
            || !operations::valid_profile_kind(self.profile_kind)
            || !release::valid_release_name(self.release_name)
            || !operations::valid_profile_name(self.profile_name)
            || !deployment::valid_artifact_name(self.target_artifact)
            || !operations_bundle::valid_object_key(self.object_key)
            || self.authorization_signature.iter().all(|byte| *byte == 0)
        {
            return None;
        }
        [self.release_name, self.profile_name, self.target_artifact, self.object_key]
            .iter()
            .try_fold(RECORD_HEADER_LEN, |len, value| len.checked_add(value.len()))
    }
}

pub fn catalog_list_encoded_len(bindings: &[CatalogBinding<'_>]) -> Option<usize> {
    if bindings.len() > operations_bundle::MAX_BINDINGS || bindings.len() > u16::MAX as usize {
        return None;
    }
    bindings
        .iter()
        .try_fold(LIST_HEADER_LEN, |len, binding| len.checked_add(binding.encoded_len()?))
}

pub fn encode_catalog_list(bindings: &[CatalogBinding<'_>], output: &mut [u8]) -> Option<usize> {
    let total_len = catalog_list_encoded_len(bindings)?;
    if output.len() < total_len {
        return None;
    }
    output[..total_len].fill(0);
    output[..8].copy_from_slice(LIST_MAGIC);
    output[8..10].copy_from_slice(&LIST_VERSION.to_le_bytes());
    output[10..12].copy_from_slice(&(bindings.len() as u16).to_le_bytes());
    output[12..16].copy_from_slice(&(total_len as u32).to_le_bytes());
    let mut offset = LIST_HEADER_LEN;
    for binding in bindings {
        let record_len = binding.encoded_len()?;
        let record = &mut output[offset..offset + record_len];
        record[0..4].copy_from_slice(&(record_len as u32).to_le_bytes());
        record[8..16].copy_from_slice(&binding.generation.to_le_bytes());
        record[16..24].copy_from_slice(&binding.bundle_sequence.to_le_bytes());
        record[24..32].copy_from_slice(&binding.sequence.to_le_bytes());
        record[32..40].copy_from_slice(&binding.expires_unix_seconds.to_le_bytes());
        record[40..42].copy_from_slice(&binding.profile_kind.to_le_bytes());
        for (range, value) in [
            (42..44, binding.release_name.len()),
            (44..46, binding.profile_name.len()),
            (46..48, binding.target_artifact.len()),
            (48..50, binding.object_key.len()),
        ] {
            record[range].copy_from_slice(&(value as u16).to_le_bytes());
        }
        record[64..96].copy_from_slice(&binding.release_digest);
        record[96..128].copy_from_slice(&binding.bundle_digest);
        record[128..160].copy_from_slice(&binding.envelope_digest);
        record[160..176].copy_from_slice(&binding.recipient_key_id);
        record[176..192].copy_from_slice(&binding.signing_key_id);
        record[192..256].copy_from_slice(&binding.authorization_signature);
        let names_start = RECORD_HEADER_LEN;
        let mut cursor = names_start;
        for value in [
            binding.release_name,
            binding.profile_name,
            binding.target_artifact,
            binding.object_key,
        ] {
            record[cursor..cursor + value.len()].copy_from_slice(value);
            cursor += value.len();
        }
        offset += record_len;
    }
    Some(total_len)
}

pub struct CatalogBindings<'a> {
    bytes: &'a [u8],
    offset: usize,
    remaining: usize,
}

pub fn decode_catalog_list(bytes: &[u8]) -> Option<CatalogBindings<'_>> {
    if bytes.len() < LIST_HEADER_LEN
        || bytes.get(..8)? != LIST_MAGIC
        || read_u16(bytes, 8)? != LIST_VERSION
        || usize::try_from(read_u32(bytes, 12)?).ok()? != bytes.len()
    {
        return None;
    }
    let count = usize::from(read_u16(bytes, 10)?);
    if count > operations_bundle::MAX_BINDINGS {
        return None;
    }
    Some(CatalogBindings {
        bytes,
        offset: LIST_HEADER_LEN,
        remaining: count,
    })
}

impl<'a> Iterator for CatalogBindings<'a> {
    type Item = CatalogBinding<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let record_len = usize::try_from(read_u32(self.bytes, self.offset)?).ok()?;
        let end = self.offset.checked_add(record_len)?;
        if self.remaining == 1 && end != self.bytes.len() {
            self.remaining = 0;
            return None;
        }
        let record = self.bytes.get(self.offset..end)?;
        if record_len < RECORD_HEADER_LEN
            || record.get(4..8)?.iter().any(|byte| *byte != 0)
            || record.get(50..64)?.iter().any(|byte| *byte != 0)
        {
            self.remaining = 0;
            return None;
        }
        let lengths = [
            usize::from(read_u16(record, 42)?),
            usize::from(read_u16(record, 44)?),
            usize::from(read_u16(record, 46)?),
            usize::from(read_u16(record, 48)?),
        ];
        let mut cursor = RECORD_HEADER_LEN;
        let mut values: [&[u8]; 4] = [&[]; 4];
        for (value, len) in values.iter_mut().zip(lengths) {
            let value_end = cursor.checked_add(len)?;
            *value = record.get(cursor..value_end)?;
            cursor = value_end;
        }
        let binding = CatalogBinding {
            generation: read_u64(record, 8)?,
            bundle_sequence: read_u64(record, 16)?,
            sequence: read_u64(record, 24)?,
            expires_unix_seconds: read_u64(record, 32)?,
            profile_kind: read_u16(record, 40)?,
            release_name: values[0],
            profile_name: values[1],
            target_artifact: values[2],
            object_key: values[3],
            release_digest: record.get(64..96)?.try_into().ok()?,
            bundle_digest: record.get(96..128)?.try_into().ok()?,
            envelope_digest: record.get(128..160)?.try_into().ok()?,
            recipient_key_id: record.get(160..176)?.try_into().ok()?,
            signing_key_id: record.get(176..192)?.try_into().ok()?,
            authorization_signature: record.get(192..256)?.try_into().ok()?,
        };
        if cursor != record.len() || binding.encoded_len() != Some(record.len()) {
            self.remaining = 0;
            return None;
        }
        self.offset = end;
        self.remaining -= 1;
        Some(binding)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pickup<'a> {
    pub binding: CatalogBinding<'a>,
    pub now_unix_seconds: u64,
    pub release: &'a [u8],
    pub artifact: &'a [u8],
    pub descriptor: &'a [u8],
    pub envelope: &'a [u8],
}

impl Pickup<'_> {
    pub fn encoded_len(&self) -> Option<usize> {
        if self.now_unix_seconds == 0
            || !(release::HEADER_LEN..=release::MAX_RELEASE_LEN).contains(&self.release.len())
            || self.artifact.is_empty()
            || self.artifact.len() > crate::MAX_ARTIFACT_ELF_SIZE
            || !(deployment::HEADER_LEN..=deployment::MAX_DESCRIPTOR_LEN)
                .contains(&self.descriptor.len())
            || !(operations::HEADER_LEN..=operations::MAX_ENVELOPE_LEN)
                .contains(&self.envelope.len())
            || self.binding.encoded_len().is_none()
        {
            return None;
        }
        [
            self.binding.release_name.len(),
            self.binding.profile_name.len(),
            self.binding.target_artifact.len(),
            self.binding.object_key.len(),
            self.release.len(),
            self.artifact.len(),
            self.descriptor.len(),
            self.envelope.len(),
        ]
        .into_iter()
        .try_fold(PICKUP_HEADER_LEN, usize::checked_add)
        .filter(|len| *len <= MAX_PICKUP_LEN)
    }

    pub fn encode(&self, output: &mut [u8]) -> Option<usize> {
        let len = self.encoded_len()?;
        if output.len() < len {
            return None;
        }
        output[..len].fill(0);
        output[..8].copy_from_slice(PICKUP_MAGIC);
        output[8..10].copy_from_slice(&PICKUP_VERSION.to_le_bytes());
        output[10..12].copy_from_slice(&(PICKUP_HEADER_LEN as u16).to_le_bytes());
        output[12..16].copy_from_slice(&(len as u32).to_le_bytes());
        output[16..24].copy_from_slice(&self.binding.generation.to_le_bytes());
        output[24..32].copy_from_slice(&self.binding.bundle_sequence.to_le_bytes());
        output[32..40].copy_from_slice(&self.binding.sequence.to_le_bytes());
        output[40..48].copy_from_slice(&self.binding.expires_unix_seconds.to_le_bytes());
        output[48..56].copy_from_slice(&self.now_unix_seconds.to_le_bytes());
        output[56..58].copy_from_slice(&self.binding.profile_kind.to_le_bytes());
        for (range, value) in [
            (58..60, self.binding.release_name.len()),
            (60..62, self.binding.profile_name.len()),
            (62..64, self.binding.target_artifact.len()),
        ] {
            output[range].copy_from_slice(&(value as u16).to_le_bytes());
        }
        output[76..78].copy_from_slice(&(self.binding.object_key.len() as u16).to_le_bytes());
        for (range, value) in [
            (64..68, self.artifact.len()),
            (68..72, self.descriptor.len()),
            (72..76, self.envelope.len()),
        ] {
            output[range].copy_from_slice(&(value as u32).to_le_bytes());
        }
        output[80..112].copy_from_slice(&self.binding.release_digest);
        output[112..144].copy_from_slice(&self.binding.bundle_digest);
        output[144..176].copy_from_slice(&self.binding.envelope_digest);
        output[176..192].copy_from_slice(&self.binding.recipient_key_id);
        output[192..208].copy_from_slice(&self.binding.signing_key_id);
        output[208..272].copy_from_slice(&self.binding.authorization_signature);
        output[272..276].copy_from_slice(&(self.release.len() as u32).to_le_bytes());
        let mut offset = PICKUP_HEADER_LEN;
        for value in [
            self.binding.release_name,
            self.binding.profile_name,
            self.binding.target_artifact,
            self.binding.object_key,
            self.release,
            self.artifact,
            self.descriptor,
            self.envelope,
        ] {
            output[offset..offset + value.len()].copy_from_slice(value);
            offset += value.len();
        }
        Some(len)
    }

    pub fn decode(bytes: &'_ [u8]) -> Option<Pickup<'_>> {
        if bytes.len() < PICKUP_HEADER_LEN
            || bytes.len() > MAX_PICKUP_LEN
            || bytes.get(..8)? != PICKUP_MAGIC
            || read_u16(bytes, 8)? != PICKUP_VERSION
            || usize::from(read_u16(bytes, 10)?) != PICKUP_HEADER_LEN
            || usize::try_from(read_u32(bytes, 12)?).ok()? != bytes.len()
            || bytes.get(78..80)?.iter().any(|byte| *byte != 0)
            || bytes.get(276..PICKUP_HEADER_LEN)?.iter().any(|byte| *byte != 0)
        {
            return None;
        }
        let lengths = [
            usize::from(read_u16(bytes, 58)?),
            usize::from(read_u16(bytes, 60)?),
            usize::from(read_u16(bytes, 62)?),
            usize::from(read_u16(bytes, 76)?),
            usize::try_from(read_u32(bytes, 272)?).ok()?,
            usize::try_from(read_u32(bytes, 64)?).ok()?,
            usize::try_from(read_u32(bytes, 68)?).ok()?,
            usize::try_from(read_u32(bytes, 72)?).ok()?,
        ];
        let mut offset = PICKUP_HEADER_LEN;
        let mut values: [&[u8]; 8] = [&[]; 8];
        for (value, len) in values.iter_mut().zip(lengths) {
            let end = offset.checked_add(len)?;
            *value = bytes.get(offset..end)?;
            offset = end;
        }
        let pickup = Pickup {
            binding: CatalogBinding {
                generation: read_u64(bytes, 16)?,
                bundle_sequence: read_u64(bytes, 24)?,
                sequence: read_u64(bytes, 32)?,
                expires_unix_seconds: read_u64(bytes, 40)?,
                profile_kind: read_u16(bytes, 56)?,
                release_name: values[0],
                profile_name: values[1],
                target_artifact: values[2],
                object_key: values[3],
                release_digest: bytes.get(80..112)?.try_into().ok()?,
                bundle_digest: bytes.get(112..144)?.try_into().ok()?,
                envelope_digest: bytes.get(144..176)?.try_into().ok()?,
                recipient_key_id: bytes.get(176..192)?.try_into().ok()?,
                signing_key_id: bytes.get(192..208)?.try_into().ok()?,
                authorization_signature: bytes.get(208..272)?.try_into().ok()?,
            },
            now_unix_seconds: read_u64(bytes, 48)?,
            release: values[4],
            artifact: values[5],
            descriptor: values[6],
            envelope: values[7],
        };
        (offset == bytes.len() && pickup.encoded_len() == Some(bytes.len())).then_some(pickup)
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}
