use super::*;

const MASK: u64 = (1u64 << 48) - 1;
fn region(index: u16) -> Region {
    Region {
        base: (u64::from(index) + 1) << 28,
        segment: index,
        start: 0,
        end: 255,
    }
}
fn checksum(bytes: &mut [u8]) {
    bytes[9] = 0;
    bytes[9] = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
}
fn table(entries: &[Region]) -> Vec<u8> {
    let mut bytes = vec![0u8; HEADER_BYTES + entries.len() * ENTRY_BYTES];
    bytes[..4].copy_from_slice(b"MCFG");
    let length = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    for (slot, entry) in
        bytes[HEADER_BYTES..].as_chunks_mut::<ENTRY_BYTES>().0.iter_mut().zip(entries)
    {
        slot[..8].copy_from_slice(&entry.base.to_le_bytes());
        slot[8..10].copy_from_slice(&entry.segment.to_le_bytes());
        slot[10] = entry.start;
        slot[11] = entry.end;
    }
    checksum(&mut bytes);
    bytes
}

#[test]
fn exact_table_capacity_and_empty_tables_are_accepted() {
    let mut regions = Regions::new();
    regions.append_table(&table(&[]), MASK).unwrap();
    let entries: Vec<_> = (0..MAX_REGIONS as u16).map(region).collect();
    regions.append_table(&table(&entries), MASK).unwrap();
    assert_eq!(regions.iter().copied().collect::<Vec<_>>(), entries);
    assert_eq!(regions.append_table(&table(&[region(32)]), MASK), Err(Error::Capacity));
    assert_eq!(regions.len(), MAX_REGIONS);
}

#[test]
fn short_partial_oversized_wrong_signature_and_checksum_reject() {
    for len in 0..HEADER_BYTES {
        assert_eq!(Regions::new().append_table(&vec![0; len], MASK), Err(Error::TableGeometry));
    }
    let valid = table(&[region(0)]);
    for len in [HEADER_BYTES + 1, MAX_TABLE_BYTES + 1] {
        let mut bytes = vec![0; len];
        bytes[..4].copy_from_slice(b"MCFG");
        bytes[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        checksum(&mut bytes);
        assert_eq!(Regions::new().append_table(&bytes, MASK), Err(Error::TableGeometry));
    }
    let mut bytes = valid.clone();
    bytes[0] = b'X';
    checksum(&mut bytes);
    assert_eq!(Regions::new().append_table(&bytes, MASK), Err(Error::TableGeometry));
    let mut bytes = valid.clone();
    bytes[4..8].copy_from_slice(&44u32.to_le_bytes());
    checksum(&mut bytes);
    assert_eq!(Regions::new().append_table(&bytes, MASK), Err(Error::TableGeometry));
    let mut bytes = valid;
    bytes[16] ^= 1;
    assert_eq!(Regions::new().append_table(&bytes, MASK), Err(Error::Checksum));
}

#[test]
fn rejected_later_entry_or_table_preserves_complete_original_prefix() {
    let mut regions = Regions::new();
    regions.append_table(&table(&[region(0)]), MASK).unwrap();
    let bad = Region {
        base: region(2).base + 4096,
        ..region(2)
    };
    assert_eq!(regions.append_table(&table(&[region(1), bad]), MASK), Err(Error::PhysicalRange));
    assert_eq!(regions.iter().copied().collect::<Vec<_>>(), vec![region(0)]);
    let mut reserved = table(&[region(1)]);
    reserved[HEADER_BYTES + 12] = 1;
    checksum(&mut reserved);
    assert_eq!(regions.append_table(&reserved, MASK), Err(Error::Reserved));
    reserved[HEADER_BYTES + 12] = 0;
    reserved[36] = 1;
    checksum(&mut reserved);
    assert_eq!(regions.append_table(&reserved, MASK), Err(Error::Reserved));
    assert_eq!(regions.len(), 1);
}

#[test]
fn invalid_ranges_aliases_and_bus_zero_relative_addresses() {
    for bad in [
        Region {
            start: 10,
            end: 9,
            ..region(0)
        },
        Region {
            base: u64::MAX & !((1 << 20) - 1),
            ..region(0)
        },
        Region {
            base: 1 << 48,
            ..region(0)
        },
    ] {
        assert!(Regions::new().append_table(&table(&[bad]), MASK).is_err());
    }
    let a = Region {
        base: 0xe010_0000,
        segment: 3,
        start: 8,
        end: 15,
    };
    let b = Region {
        start: 16,
        end: 31,
        ..a
    };
    let mut regions = Regions::new();
    regions.append_table(&table(&[a, b]), MASK).unwrap();
    assert_eq!(a.span(), (0xe090_0000, 0xe110_0000));
    assert_eq!(
        regions.append_table(
            &table(&[Region {
                start: 14,
                end: 16,
                base: 0xd000_0000,
                ..a
            }]),
            MASK
        ),
        Err(Error::Overlap)
    );
    assert_eq!(
        regions.append_table(
            &table(&[Region {
                segment: 4,
                ..a
            }]),
            MASK
        ),
        Err(Error::Overlap)
    );
    assert_eq!(regions.len(), 2);
}
