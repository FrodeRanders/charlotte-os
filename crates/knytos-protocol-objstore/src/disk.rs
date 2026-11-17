//! Sole current on-disk grammar; contains no live capabilities.

pub const SB_MAGIC: u64 = 0x3452_5453_4a42_4f43; // "COBJSTR4"
pub const SB_VERSION: u32 = 4;
pub const SB_SLOTS: u32 = 2;
pub const SB_LEN: usize = 80;
pub const SB_CHECKSUM_OFFSET: usize = 76;

pub const DIR_ENTRY_SIZE: usize = 32;
pub const DIR_CRC_SALT: u32 = 0x3444_4952; // Current directory CRC discriminator.
pub const MAX_DIRECTORY_BLOCKS: u32 = 4096;
pub const FLAG_ALLOCATED: u32 = 1;

pub const HEADER_MAGIC: u64 = 0x3444_484a_424f_4343; // "CCOBJHD4"
pub const HEADER_VERSION: u16 = 4;
pub const HEADER_LEN: usize = 384;
pub const HEADER_CHECKSUM_OFFSET: usize = 112;
pub const EXTENTS_OFFSET: usize = 128;
pub const EXTENT_SIZE: usize = 16;
pub const MAX_EXTENTS: usize = 16;
pub const HASH_FNV1A64: u16 = 2;

#[derive(Clone, Copy)]
pub struct Extent {
    pub lba: u64,
    pub blocks: u32,
}

pub const EMPTY_EXTENT: Extent = Extent {
    lba: 0,
    blocks: 0,
};

#[derive(Clone, Copy)]
pub struct ObjectHeader {
    pub id: u64,
    pub generation: u64,
    pub data_len: u64,
    pub allocated_len: u64,
    pub data_offset: u64,
    pub header_blocks: u32,
    pub extent_count: u16,
    pub extents: [Extent; MAX_EXTENTS],
    pub data_hash: u64,
    /// Explicit nonzero creating principal.
    pub owner: u64,
    /// `ATTR_*` policy bits (`objstore::ATTR_SHARED_READ`).
    pub attributes: u32,
}

impl ObjectHeader {
    pub fn empty(id: u64, generation: u64, block_size: u32, owner: u64, attributes: u32) -> Self {
        Self {
            id,
            generation,
            data_len: 0,
            allocated_len: 0,
            data_offset: block_size as u64,
            header_blocks: 1,
            extent_count: 0,
            extents: [EMPTY_EXTENT; MAX_EXTENTS],
            data_hash: 0xcbf2_9ce4_8422_2325,
            owner,
            attributes,
        }
    }

    pub fn extents(&self) -> &[Extent] {
        &self.extents[..self.extent_count as usize]
    }
}

pub fn encode_header(header: ObjectHeader, bytes: &mut [u8]) {
    bytes.fill(0);
    put_u64(bytes, 0, HEADER_MAGIC);
    put_u16(bytes, 8, HEADER_VERSION);
    put_u16(bytes, 10, HEADER_LEN as u16);
    put_u32(bytes, 12, FLAG_ALLOCATED);
    put_u64(bytes, 16, header.id);
    put_u64(bytes, 24, header.generation);
    put_u64(bytes, 32, header.data_len);
    put_u64(bytes, 40, header.allocated_len);
    put_u64(bytes, 48, header.data_offset);
    put_u16(bytes, 56, header.extent_count);
    put_u16(bytes, 58, HASH_FNV1A64);
    put_u32(bytes, 60, header.header_blocks);
    put_u64(bytes, 64, header.owner);
    put_u64(bytes, 80, header.data_hash);
    put_u32(bytes, 88, header.attributes);
    for (index, extent) in header.extents().iter().enumerate() {
        let offset = EXTENTS_OFFSET + index * EXTENT_SIZE;
        put_u64(bytes, offset, extent.lba);
        put_u32(bytes, offset + 8, extent.blocks);
    }
    put_u32(bytes, HEADER_CHECKSUM_OFFSET, 0);
    put_u32(bytes, HEADER_CHECKSUM_OFFSET, crc32(&bytes[..HEADER_LEN]));
}

pub fn decode_header(bytes: &[u8]) -> Option<ObjectHeader> {
    if bytes.len() < HEADER_LEN
        || get_u64(bytes, 0)? != HEADER_MAGIC
        || get_u16(bytes, 8)? != HEADER_VERSION
        || get_u16(bytes, 10)? as usize != HEADER_LEN
        || get_u32(bytes, 12)? != FLAG_ALLOCATED
        || get_u64(bytes, 16)? == 0
        || get_u64(bytes, 24)? == 0
        || get_u32(bytes, 60)? != 1
        || get_u64(bytes, 64)? == 0
        || get_u32(bytes, 88)? & !super::ATTR_SHARED_READ != 0
        || get_u16(bytes, 56)? as usize > MAX_EXTENTS
        || get_u16(bytes, 58)? != HASH_FNV1A64
        || bytes[72..80].iter().any(|byte| *byte != 0)
        || bytes[92..112].iter().any(|byte| *byte != 0)
        || bytes[116..128].iter().any(|byte| *byte != 0)
        || bytes[HEADER_LEN..].iter().any(|byte| *byte != 0)
        || get_u32(bytes, HEADER_CHECKSUM_OFFSET)? != header_crc(bytes)
    {
        return None;
    }
    let extent_count = get_u16(bytes, 56)?;
    let mut extents = [EMPTY_EXTENT; MAX_EXTENTS];
    for (index, extent) in extents.iter_mut().enumerate().take(extent_count as usize) {
        let offset = EXTENTS_OFFSET + index * EXTENT_SIZE;
        *extent = Extent {
            lba: get_u64(bytes, offset)?,
            blocks: get_u32(bytes, offset + 8)?,
        };
        if extent.blocks == 0 || get_u32(bytes, offset + 12)? != 0 {
            return None;
        }
    }
    let unused = EXTENTS_OFFSET + extent_count as usize * EXTENT_SIZE;
    if bytes[unused..HEADER_LEN].iter().any(|byte| *byte != 0) {
        return None;
    }
    Some(ObjectHeader {
        id: get_u64(bytes, 16)?,
        generation: get_u64(bytes, 24)?,
        data_len: get_u64(bytes, 32)?,
        allocated_len: get_u64(bytes, 40)?,
        data_offset: get_u64(bytes, 48)?,
        header_blocks: get_u32(bytes, 60)?,
        extent_count,
        extents,
        data_hash: get_u64(bytes, 80)?,
        owner: get_u64(bytes, 64)?,
        attributes: get_u32(bytes, 88)?,
    })
}

/// A successfully read slot is not necessarily an empty slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuperblockState {
    Blank,
    Current,
    Corrupt,
    Unsupported,
}

pub fn classify_superblock(bytes: &[u8]) -> SuperblockState {
    if bytes.len() < SB_LEN {
        return SuperblockState::Corrupt;
    }
    if bytes.iter().all(|byte| *byte == 0) {
        return SuperblockState::Blank;
    }
    if (bytes.get(..7) == Some(b"COBJSTR") || bytes.get(..7) == Some(b"COJBSTR"))
        && (get_u64(bytes, 0) != Some(SB_MAGIC) || get_u32(bytes, 8) != Some(SB_VERSION))
    {
        return SuperblockState::Unsupported;
    }
    if get_u64(bytes, 0) != Some(SB_MAGIC)
        || get_u32(bytes, 8) != Some(SB_VERSION)
        || get_u32(bytes, 12) != Some(SB_LEN as u32)
        || get_u64(bytes, 16) == Some(0)
        || get_u64(bytes, 32) == Some(0)
        || get_u32(bytes, 72) != Some(0)
        || bytes[SB_LEN..].iter().any(|byte| *byte != 0)
        || get_u32(bytes, SB_CHECKSUM_OFFSET) != Some(crc32(&bytes[..SB_CHECKSUM_OFFSET]))
    {
        SuperblockState::Corrupt
    } else {
        SuperblockState::Current
    }
}

/// CRC of the fixed header, with its checksum field interpreted as zero.
fn header_crc(bytes: &[u8]) -> u32 {
    let mut header = [0u8; HEADER_LEN];
    header.copy_from_slice(&bytes[..HEADER_LEN]);
    header[HEADER_CHECKSUM_OFFSET..HEADER_CHECKSUM_OFFSET + 4].fill(0);
    crc32(&header)
}
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn get_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}
fn get_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}
fn get_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}
fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recheck_header(bytes: &mut [u8]) {
        put_u32(bytes, HEADER_CHECKSUM_OFFSET, header_crc(bytes));
    }

    #[test]
    fn current_header_roundtrip_preserves_owner_and_empty_extent() {
        let mut bytes = [0u8; 512];
        encode_header(
            ObjectHeader::empty(42, 7, 512, 9, super::super::ATTR_SHARED_READ),
            &mut bytes,
        );
        let decoded = decode_header(&bytes).unwrap();
        assert_eq!(
            (decoded.id, decoded.generation, decoded.owner, decoded.attributes),
            (42, 7, 9, 1)
        );
        assert!(decoded.extents().is_empty());
        assert_eq!(decoded.data_hash, 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn valid_checksum_cannot_restore_retired_or_missing_header_fields() {
        let mut valid = [0u8; 512];
        encode_header(ObjectHeader::empty(42, 1, 512, 9, 0), &mut valid);
        for (offset, data) in [
            (0, &0x3244_484a_424f_4343u64.to_le_bytes()[..]),
            (8, &3u16.to_le_bytes()[..]),
            (10, &400u16.to_le_bytes()[..]),
            (12, &3u32.to_le_bytes()[..]),
            (16, &0u64.to_le_bytes()[..]),
            (24, &0u64.to_le_bytes()[..]),
            (60, &2u32.to_le_bytes()[..]),
            (64, &0u64.to_le_bytes()[..]),
            (72, &[1u8][..]),
            (88, &2u32.to_le_bytes()[..]),
            (92, &[1u8][..]),
            (116, &[1u8][..]),
            (128, &[1u8][..]),
            (384, &[1u8][..]),
        ] {
            let mut bytes = valid;
            bytes[offset..offset + data.len()].copy_from_slice(data);
            recheck_header(&mut bytes);
            assert!(decode_header(&bytes).is_none(), "accepted altered field {offset}");
        }
        for length in [0, 8, HEADER_LEN - 1] {
            assert!(decode_header(&valid[..length]).is_none());
        }
        valid[80] ^= 1;
        assert!(decode_header(&valid).is_none());
    }

    #[test]
    fn extent_roundtrip_and_reserved_padding_are_exact() {
        let mut h = ObjectHeader::empty(42, 1, 512, 9, 0);
        h.extent_count = 1;
        h.extents[0] = Extent {
            lba: 100,
            blocks: 2,
        };
        h.data_len = 123;
        h.allocated_len = 1024;
        let mut bytes = [0u8; 512];
        encode_header(h, &mut bytes);
        assert_eq!(decode_header(&bytes).unwrap().extents()[0].lba, 100);
        for offset in [EXTENTS_OFFSET + 8, EXTENTS_OFFSET + 12] {
            let mut altered = bytes;
            put_u32(
                &mut altered,
                offset,
                if offset == EXTENTS_OFFSET + 8 {
                    0
                } else {
                    1
                },
            );
            recheck_header(&mut altered);
            assert!(decode_header(&altered).is_none());
        }
    }

    #[test]
    fn superblock_distinguishes_blank_retired_current_and_corrupt() {
        let mut bytes = [0u8; 512];
        assert_eq!(classify_superblock(&bytes), SuperblockState::Blank);
        put_u64(&mut bytes, 0, SB_MAGIC);
        put_u32(&mut bytes, 8, SB_VERSION);
        put_u32(&mut bytes, 12, SB_LEN as u32);
        put_u64(&mut bytes, 16, 1);
        put_u64(&mut bytes, 32, 1);
        let crc = crc32(&bytes[..SB_CHECKSUM_OFFSET]);
        put_u32(&mut bytes, SB_CHECKSUM_OFFSET, crc);
        assert_eq!(classify_superblock(&bytes), SuperblockState::Current);
        let valid = bytes;
        for version in [1u32, 2, 3] {
            let mut old = valid;
            old[7] = b'0' + version as u8;
            put_u32(&mut old, 8, version);
            let crc = crc32(&old[..SB_CHECKSUM_OFFSET]);
            put_u32(&mut old, SB_CHECKSUM_OFFSET, crc);
            assert_eq!(classify_superblock(&old), SuperblockState::Unsupported);
        }
        let mut historical = valid;
        put_u64(&mut historical, 0, 0x3352_5453_424a_4f43);
        put_u32(&mut historical, 8, 3);
        let checksum = crc32(&historical[..SB_CHECKSUM_OFFSET]);
        put_u32(&mut historical, SB_CHECKSUM_OFFSET, checksum);
        assert_eq!(classify_superblock(&historical), SuperblockState::Unsupported);
        bytes[76] ^= 1;
        assert_eq!(classify_superblock(&bytes), SuperblockState::Corrupt);
        assert_eq!(classify_superblock(&[0; 16]), SuperblockState::Corrupt);
        for offset in [16, 32, 72, 80] {
            let mut bad = valid;
            bad[offset] = if offset == 16 || offset == 32 {
                0
            } else {
                1
            };
            let crc = crc32(&bad[..SB_CHECKSUM_OFFSET]);
            put_u32(&mut bad, SB_CHECKSUM_OFFSET, crc);
            assert_eq!(classify_superblock(&bad), SuperblockState::Corrupt);
        }
    }

    #[test]
    fn directory_rejection_cannot_become_absence_and_tombstones_win() {
        assert!(decode_directory(&[0; DIR_ENTRY_SIZE + 1]).is_none());
        let entry = DirectoryEntry {
            id: 42,
            flags: FLAG_ALLOCATED,
            generation: 3,
            header_lba: 100,
            header_blocks: 1,
        };
        let mut valid = [0u8; DIR_ENTRY_SIZE];
        encode_directory(entry, &mut valid);
        assert_eq!(decode_directory(&valid).unwrap().id, 42);
        let empty = decode_directory(&[0; DIR_ENTRY_SIZE]);
        assert_eq!(newest_directory_entry(decode_directory(&valid), empty).unwrap().id, 42);
        let tombstone = DirectoryEntry {
            generation: 4,
            ..DirectoryEntry::default()
        };
        let mut deleted = [0u8; DIR_ENTRY_SIZE];
        encode_directory(tombstone, &mut deleted);
        assert_eq!(
            newest_directory_entry(decode_directory(&valid), decode_directory(&deleted))
                .unwrap()
                .id,
            0
        );
        for (offset, value) in [(8, 3u32), (12, 0), (24, 2)] {
            let mut bad = valid;
            put_u32(&mut bad, offset, value);
            let crc = crc32(&bad[..28]) ^ DIR_CRC_SALT;
            put_u32(&mut bad, 28, crc);
            assert!(decode_directory(&bad).is_none());
            assert!(newest_directory_entry(decode_directory(&bad), None).is_none());
        }
        let mut retired = valid;
        let crc = crc32(&retired[..28]) ^ 0x3344_4952;
        put_u32(&mut retired, 28, crc);
        assert!(decode_directory(&retired).is_none());
        assert!(newest_directory_entry(None, None).is_none());
    }
}

#[derive(Clone, Copy, Default)]
pub struct DirectoryEntry {
    pub id: u64,
    pub flags: u32,
    pub generation: u32,
    pub header_lba: u64,
    pub header_blocks: u32,
}

pub fn encode_directory(entry: DirectoryEntry, bytes: &mut [u8]) {
    bytes.fill(0);
    put_u64(bytes, 0, entry.id);
    put_u32(bytes, 8, entry.flags);
    put_u32(bytes, 12, entry.generation);
    put_u64(bytes, 16, entry.header_lba);
    put_u32(bytes, 24, entry.header_blocks);
    put_u32(bytes, 28, crc32(&bytes[..28]) ^ DIR_CRC_SALT);
}

pub fn decode_directory(bytes: &[u8]) -> Option<DirectoryEntry> {
    if bytes.len() != DIR_ENTRY_SIZE {
        return None;
    }
    let id = get_u64(bytes, 0)?;
    if id == 0 && bytes.iter().all(|byte| *byte == 0) {
        return Some(DirectoryEntry::default());
    }
    if get_u32(bytes, 28)? != crc32(&bytes[..28]) ^ DIR_CRC_SALT {
        return None;
    }
    let flags = get_u32(bytes, 8)?;
    let generation = get_u32(bytes, 12)?;
    let header_lba = get_u64(bytes, 16)?;
    let header_blocks = get_u32(bytes, 24)?;
    if generation == 0
        || if id == 0 {
            flags != 0 || header_lba != 0 || header_blocks != 0
        } else {
            flags != FLAG_ALLOCATED || header_lba == 0 || header_blocks != 1
        }
    {
        return None;
    }
    Some(DirectoryEntry {
        id,
        flags,
        generation,
        header_lba,
        header_blocks,
    })
}

pub fn newest_directory_entry(
    first: Option<DirectoryEntry>,
    second: Option<DirectoryEntry>,
) -> Option<DirectoryEntry> {
    match (first, second) {
        (Some(a), Some(b)) => Some(
            if a.generation >= b.generation {
                a
            } else {
                b
            },
        ),
        (Some(entry), None) | (None, Some(entry)) => Some(entry),
        (None, None) => None,
    }
}
