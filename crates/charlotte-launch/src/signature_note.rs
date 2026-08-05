//! ELF signature notes: the official-ish way to carry a cryptographic
//! signature inside an executable.
//!
//! ELF has no standardized in-file signature section (unlike PE
//! Authenticode or Mach-O code signing), but the ELF note mechanism —
//! `SHT_NOTE` sections — is the sanctioned generic extension point. The
//! signer adds a note
//!
//! ```text
//! namesz = 9, descsz = 64, type = 1
//! name:  "charlotte\0"
//! desc:  the 64-byte Ed25519 signature
//! ```
//!
//! The signature covers the SHA-256 of the *entire file* with the note's
//! descriptor bytes treated as zeros (a signature cannot cover itself —
//! the same trick Authenticode uses). Both the signer (`tools/cluster-sign
//! elf-sign`) and every verifier agree on the zeroed region, so the signed
//! bytes are deterministic.
//!
//! The kernel's EL0 loader verifies the note before mapping a domain
//! (`crate::service::loader`), and the deploy agent verifies the note of
//! fetched artifacts before serving them — both through
//! [`verify_elf`], which returns the outcome without panicking.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use super::sha256;

/// The note's name: "charlotte" plus the NUL terminator.
pub const NOTE_NAME: &[u8] = b"charlotte";
/// The note type: 1 = cluster signature (this scheme).
pub const NOTE_TYPE_SIGNATURE: u32 = 0x434c_5331; // "CLS1": cluster signature, revision 1
/// The signature length, and therefore the descriptor length.
pub const SIGNATURE_LEN: usize = 64;

/// Outcome of verifying an ELF image against the cluster public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The image carries a valid cluster signature.
    Valid,
    /// The image carries a note but the signature does not verify (or is
    /// malformed). Loading it must be refused.
    Invalid,
    /// The image has no cluster signature note. Policy decides whether that
    /// is acceptable.
    Unsigned,
}

const SHT_NOTE: u32 = 7;

/// Parse the ELF64 header fields needed to walk the section header table:
/// `(section_table_offset, section_entry_size, section_count)`.
fn section_table(image: &[u8]) -> Option<(usize, usize, usize)> {
    if image.len() < 64 {
        return None;
    }
    if &image[0..4] != b"\x7fELF" || image[4] != 2 || image[5] != 1 {
        return None;
    }
    let shoff = u64::from_le_bytes(image[0x28..0x30].try_into().ok()?) as usize;
    let shentsize = u16::from_le_bytes(image[0x3a..0x3c].try_into().ok()?) as usize;
    let shnum = u16::from_le_bytes(image[0x3c..0x3e].try_into().ok()?) as usize;
    if shentsize < 40 {
        return None;
    }
    let end = shoff.checked_add(shentsize.saturating_mul(shnum))?;
    if end > image.len() {
        return None;
    }
    Some((shoff, shentsize, shnum))
}

/// Locate the cluster signature note's descriptor within an ELF image.
///
/// Returns `(file_offset_of_desc, desc_len)`. Scans every `SHT_NOTE`
/// section (identifying the note by its name and type, not by section
/// name, so the section can be called anything).
pub fn find_signature_desc(image: &[u8]) -> Option<(usize, usize)> {
    let (shoff, shentsize, shnum) = section_table(image)?;
    for index in 0..shnum {
        let header = shoff + index * shentsize;
        let sh_type = u32::from_le_bytes(image[header + 0x04..header + 0x08].try_into().ok()?);
        if sh_type != SHT_NOTE {
            continue;
        }
        let sh_offset =
            u64::from_le_bytes(image[header + 0x18..header + 0x20].try_into().ok()?) as usize;
        let sh_size =
            u64::from_le_bytes(image[header + 0x20..header + 0x28].try_into().ok()?) as usize;
        let note_end = sh_offset.checked_add(sh_size)?;
        if note_end > image.len() {
            continue;
        }
        let note = &image[sh_offset..note_end];
        let (desc, desc_within_note) = parse_note(note)?;
        if let Some(desc) = desc {
            return Some((sh_offset + desc_within_note, desc.len()));
        }
    }
    None
}

/// Parse one ELF note: returns `(desc, descriptor_offset_within_note)` when
/// the note is a cluster signature note, or `(None, descriptor_offset)`
/// otherwise.
fn parse_note(note: &[u8]) -> Option<(Option<&[u8]>, usize)> {
    if note.len() < 12 {
        return None;
    }
    let namesz = u32::from_le_bytes(note[0..4].try_into().ok()?) as usize;
    let descsz = u32::from_le_bytes(note[4..8].try_into().ok()?) as usize;
    let note_type = u32::from_le_bytes(note[8..12].try_into().ok()?);
    let name_start: usize = 12;
    let name_end = name_start.checked_add(namesz)?;
    let desc_start = align4(name_end);
    let desc_end = desc_start.checked_add(descsz)?;
    if desc_end > note.len() {
        return None;
    }
    let is_signature = note_type == NOTE_TYPE_SIGNATURE
        && namesz == NOTE_NAME.len() + 1
        && note.get(name_start..name_start + NOTE_NAME.len()) == Some(NOTE_NAME)
        && descsz >= SIGNATURE_LEN;
    let desc = if is_signature {
        Some(&note[desc_start..desc_end])
    } else {
        None
    };
    Some((desc, desc_start))
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

/// Verify an ELF image's cluster signature note against the cluster public
/// key. The image is hashed with the note's 64 descriptor bytes zeroed.
pub fn verify_elf(image: &[u8], public_key: &[u8; 32]) -> VerifyOutcome {
    let Some((desc_offset, desc_len)) = find_signature_desc(image) else {
        return VerifyOutcome::Unsigned;
    };
    if desc_len < SIGNATURE_LEN {
        return VerifyOutcome::Invalid;
    }
    let Ok(public_key) = PublicKey::from_slice(public_key) else {
        return VerifyOutcome::Invalid;
    };
    let Ok(signature) = Signature::from_slice(&image[desc_offset..desc_offset + SIGNATURE_LEN])
    else {
        return VerifyOutcome::Invalid;
    };
    let digest = sha256::digest_skipping(image, desc_offset, SIGNATURE_LEN);
    if public_key.verify(digest, &signature).is_ok() {
        VerifyOutcome::Valid
    } else {
        VerifyOutcome::Invalid
    }
}
