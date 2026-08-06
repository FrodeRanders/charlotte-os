//! Identity-bearing signatures for CharlotteOS ELF artifacts.
//!
//! A `SHT_NOTE` record named `charlotte` carries a fixed-width `CLS2`
//! descriptor.  Unlike the original `CLS1` record, the signed descriptor
//! binds the bytes to a logical artifact name, artifact class, release
//! version, rollback counter, and signing-key identifier.  A valid ELF
//! signed as `dns` therefore cannot be substituted for `raft`, even when
//! both keys are trusted by the cluster.
//!
//! The Ed25519 signature covers SHA-256 of the entire ELF with only the
//! 64-byte signature field zeroed.  All of the identity metadata remains in
//! the digest.  The fixed representation is deliberately small and
//! allocation-free so the same parser is used by the host signer, EL0
//! services, and the kernel loader.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use super::sha256;

/// ELF note owner, without the required trailing NUL byte.
pub const NOTE_NAME: &[u8] = b"charlotte";
/// `CLS2`: identity-bearing Charlotte cluster artifact signature.
pub const NOTE_TYPE_SIGNATURE: u32 = 0x434c_5332;
pub const SIGNATURE_LEN: usize = 64;
pub const ARTIFACT_NAME_CAPACITY: usize = 48;
pub const KEY_ID_LEN: usize = 16;
pub const SIGNED_METADATA_LEN: usize = 144;
pub const DESCRIPTOR_LEN: usize = SIGNED_METADATA_LEN + SIGNATURE_LEN;
pub const SIGNATURE_OFFSET_IN_DESCRIPTOR: usize = SIGNED_METADATA_LEN;

/// The signer asserts that more than one instance may execute concurrently.
pub const FLAG_PARALLEL_INSTANCES: u32 = 1 << 0;
/// The artifact is designed not to retain authoritative local state.
pub const FLAG_STATELESS: u32 = 1 << 1;
/// The artifact does not fetch executable dependencies at runtime.
pub const FLAG_NO_RUNTIME_CODE_FETCH: u32 = 1 << 2;

const DESCRIPTOR_MAGIC: &[u8; 8] = b"CARTIF2\0";
const DESCRIPTOR_VERSION: u16 = 2;
const SHT_NOTE: u32 = 7;

/// Coarse policy class carried by a blessed artifact.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactClass {
    Service = 1,
    Driver = 2,
    Bootstrap = 3,
    Administration = 4,
}

impl ArtifactClass {
    fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Service),
            2 => Some(Self::Driver),
            3 => Some(Self::Bootstrap),
            4 => Some(Self::Administration),
            _ => None,
        }
    }
}

/// Metadata that an offline cluster signing operation blesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactMetadata {
    name: [u8; ARTIFACT_NAME_CAPACITY],
    name_len: u16,
    pub class: ArtifactClass,
    pub flags: u32,
    pub artifact_version: u64,
    pub rollback_counter: u64,
    pub key_id: [u8; KEY_ID_LEN],
    /// Digest of an SBOM, source attestation, or other build-provenance
    /// statement. All-zero means no provenance statement was supplied.
    pub provenance_digest: [u8; 32],
}

impl ArtifactMetadata {
    /// Construct metadata before signing. The key id is filled by the signer.
    pub fn new(
        name: &[u8],
        class: ArtifactClass,
        artifact_version: u64,
        rollback_counter: u64,
        flags: u32,
    ) -> Option<Self> {
        if name.is_empty() || name.len() > ARTIFACT_NAME_CAPACITY || name.contains(&0) {
            return None;
        }
        let mut fixed_name = [0u8; ARTIFACT_NAME_CAPACITY];
        fixed_name[..name.len()].copy_from_slice(name);
        Some(Self {
            name: fixed_name,
            name_len: name.len() as u16,
            class,
            flags,
            artifact_version,
            rollback_counter,
            key_id: [0; KEY_ID_LEN],
            provenance_digest: [0; 32],
        })
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.name_len)]
    }

    pub fn with_key_id(mut self, public_key: &[u8; 32]) -> Self {
        self.key_id.copy_from_slice(&sha256::digest(public_key)[..KEY_ID_LEN]);
        self
    }

    pub fn with_provenance_digest(mut self, digest: [u8; 32]) -> Self {
        self.provenance_digest = digest;
        self
    }
}

/// Outcome of verifying an ELF image against a cluster public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    Valid,
    Invalid,
    Unsigned,
    /// The signature is valid, but it blesses a different logical artifact.
    ArtifactMismatch,
}

/// The canonical descriptor and the signature's exact byte range in the ELF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignatureLocation {
    pub descriptor_offset: usize,
    pub signature_offset: usize,
}

/// Encode a canonical descriptor with an all-zero signature field.
pub fn encode_descriptor(metadata: ArtifactMetadata) -> [u8; DESCRIPTOR_LEN] {
    let mut desc = [0u8; DESCRIPTOR_LEN];
    desc[0..8].copy_from_slice(DESCRIPTOR_MAGIC);
    desc[8..10].copy_from_slice(&DESCRIPTOR_VERSION.to_le_bytes());
    desc[10..12].copy_from_slice(&(SIGNED_METADATA_LEN as u16).to_le_bytes());
    desc[12..14].copy_from_slice(&(metadata.class as u16).to_le_bytes());
    desc[14..16].copy_from_slice(&metadata.name_len.to_le_bytes());
    desc[16..20].copy_from_slice(&metadata.flags.to_le_bytes());
    // 20..24 is reserved and remains zero.
    desc[24..32].copy_from_slice(&metadata.artifact_version.to_le_bytes());
    desc[32..40].copy_from_slice(&metadata.rollback_counter.to_le_bytes());
    desc[40..88].copy_from_slice(&metadata.name);
    desc[88..104].copy_from_slice(&metadata.key_id);
    desc[104..136].copy_from_slice(&metadata.provenance_digest);
    // 136..144 is reserved and remains zero; 144..208 is the signature.
    desc
}

/// Decode and validate the canonical signed metadata representation.
pub fn decode_metadata(desc: &[u8]) -> Option<ArtifactMetadata> {
    if desc.len() != DESCRIPTOR_LEN
        || desc.get(0..8)? != DESCRIPTOR_MAGIC
        || u16::from_le_bytes(desc[8..10].try_into().ok()?) != DESCRIPTOR_VERSION
        || usize::from(u16::from_le_bytes(desc[10..12].try_into().ok()?)) != SIGNED_METADATA_LEN
        || desc[20..24].iter().any(|byte| *byte != 0)
        || desc[136..144].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let class = ArtifactClass::from_raw(u16::from_le_bytes(desc[12..14].try_into().ok()?))?;
    let name_len = u16::from_le_bytes(desc[14..16].try_into().ok()?);
    let name_len_usize = usize::from(name_len);
    if name_len_usize == 0
        || name_len_usize > ARTIFACT_NAME_CAPACITY
        || desc[40..40 + name_len_usize].contains(&0)
        || desc[40 + name_len_usize..88].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let mut name = [0u8; ARTIFACT_NAME_CAPACITY];
    name.copy_from_slice(&desc[40..88]);
    let mut key_id = [0u8; KEY_ID_LEN];
    key_id.copy_from_slice(&desc[88..104]);
    let mut provenance_digest = [0u8; 32];
    provenance_digest.copy_from_slice(&desc[104..136]);
    Some(ArtifactMetadata {
        name,
        name_len,
        class,
        flags: u32::from_le_bytes(desc[16..20].try_into().ok()?),
        artifact_version: u64::from_le_bytes(desc[24..32].try_into().ok()?),
        rollback_counter: u64::from_le_bytes(desc[32..40].try_into().ok()?),
        key_id,
        provenance_digest,
    })
}

/// Parse enough ELF64 metadata to walk the section header table.
fn section_table(image: &[u8]) -> Option<(usize, usize, usize)> {
    if image.len() < 64 || &image[0..4] != b"\x7fELF" || image[4] != 2 || image[5] != 1 {
        return None;
    }
    let shoff = usize::try_from(u64::from_le_bytes(image[0x28..0x30].try_into().ok()?)).ok()?;
    let shentsize = usize::from(u16::from_le_bytes(image[0x3a..0x3c].try_into().ok()?));
    let shnum = usize::from(u16::from_le_bytes(image[0x3c..0x3e].try_into().ok()?));
    if shentsize < 64 {
        return None;
    }
    let end = shoff.checked_add(shentsize.checked_mul(shnum)?)?;
    (end <= image.len()).then_some((shoff, shentsize, shnum))
}

fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

/// Find the canonical `CLS2` record, scanning every record in every note
/// section rather than assuming that the desired record is first.
pub fn signature_location(image: &[u8]) -> Option<SignatureLocation> {
    let (shoff, shentsize, shnum) = section_table(image)?;
    for index in 0..shnum {
        let header = shoff.checked_add(index.checked_mul(shentsize)?)?;
        let sh_type = u32::from_le_bytes(image.get(header + 4..header + 8)?.try_into().ok()?);
        if sh_type != SHT_NOTE {
            continue;
        }
        let offset = usize::try_from(u64::from_le_bytes(
            image.get(header + 0x18..header + 0x20)?.try_into().ok()?,
        ))
        .ok()?;
        let size = usize::try_from(u64::from_le_bytes(
            image.get(header + 0x20..header + 0x28)?.try_into().ok()?,
        ))
        .ok()?;
        let end = offset.checked_add(size)?;
        if end > image.len() {
            continue;
        }
        let mut cursor = offset;
        while cursor < end {
            let Some(header_end) = cursor.checked_add(12) else {
                break;
            };
            if header_end > end {
                break;
            }
            let namesz = u32::from_le_bytes(image[cursor..cursor + 4].try_into().ok()?) as usize;
            let descsz =
                u32::from_le_bytes(image[cursor + 4..cursor + 8].try_into().ok()?) as usize;
            let note_type = u32::from_le_bytes(image[cursor + 8..cursor + 12].try_into().ok()?);
            let name_start = header_end;
            let Some(name_end) = name_start.checked_add(namesz) else {
                break;
            };
            let Some(desc_start) = align4(name_end) else {
                break;
            };
            let Some(desc_end) = desc_start.checked_add(descsz) else {
                break;
            };
            let Some(next) = align4(desc_end) else {
                break;
            };
            if name_end > end || desc_end > end || next > end {
                break;
            }
            let owner = &image[name_start..name_end];
            if note_type == NOTE_TYPE_SIGNATURE
                && owner == b"charlotte\0"
                && descsz == DESCRIPTOR_LEN
                && decode_metadata(&image[desc_start..desc_end]).is_some()
            {
                return Some(SignatureLocation {
                    descriptor_offset: desc_start,
                    signature_offset: desc_start + SIGNATURE_OFFSET_IN_DESCRIPTOR,
                });
            }
            cursor = next;
        }
    }
    None
}

/// Compatibility helper used by tooling that needs the descriptor range.
pub fn find_signature_desc(image: &[u8]) -> Option<(usize, usize)> {
    signature_location(image).map(|location| (location.descriptor_offset, DESCRIPTOR_LEN))
}

pub fn artifact_metadata(image: &[u8]) -> Option<ArtifactMetadata> {
    let location = signature_location(image)?;
    decode_metadata(&image[location.descriptor_offset..location.descriptor_offset + DESCRIPTOR_LEN])
}

/// Verify an artifact cryptographically without imposing a logical name.
pub fn verify_elf(image: &[u8], public_key: &[u8; 32]) -> VerifyOutcome {
    verify_elf_inner(image, public_key, None)
}

/// Verify both the signature and the logical identity blessed by it.
pub fn verify_elf_for_name(
    image: &[u8],
    public_key: &[u8; 32],
    expected_name: &[u8],
) -> VerifyOutcome {
    verify_elf_inner(image, public_key, Some(expected_name))
}

fn verify_elf_inner(
    image: &[u8],
    public_key_bytes: &[u8; 32],
    expected_name: Option<&[u8]>,
) -> VerifyOutcome {
    let Some(location) = signature_location(image) else {
        return VerifyOutcome::Unsigned;
    };
    let Some(metadata) = artifact_metadata(image) else {
        return VerifyOutcome::Invalid;
    };
    let expected_key_id = sha256::digest(public_key_bytes);
    if metadata.key_id != expected_key_id[..KEY_ID_LEN] {
        return VerifyOutcome::Invalid;
    }
    let Ok(public_key) = PublicKey::from_slice(public_key_bytes) else {
        return VerifyOutcome::Invalid;
    };
    let Ok(signature) = Signature::from_slice(
        &image[location.signature_offset..location.signature_offset + SIGNATURE_LEN],
    ) else {
        return VerifyOutcome::Invalid;
    };
    let digest = sha256::digest_skipping(image, location.signature_offset, SIGNATURE_LEN);
    if public_key.verify(digest, &signature).is_err() {
        return VerifyOutcome::Invalid;
    }
    if expected_name.is_some_and(|name| metadata.name() != name) {
        return VerifyOutcome::ArtifactMismatch;
    }
    VerifyOutcome::Valid
}
