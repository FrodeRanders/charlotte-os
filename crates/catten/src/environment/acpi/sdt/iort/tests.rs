use super::*;

fn table() -> Vec<u8> {
    let mut bytes = vec![0; IORT_HEADER_SIZE + 36 + 68];
    bytes[36..40].copy_from_slice(&2u32.to_le_bytes());
    bytes[40..44].copy_from_slice(&(IORT_HEADER_SIZE as u32).to_le_bytes());
    let root = IORT_HEADER_SIZE;
    bytes[root] = NODE_PCI_ROOT_COMPLEX;
    bytes[root + 1..root + 3].copy_from_slice(&36u16.to_le_bytes());
    bytes[root + 8..root + 12].copy_from_slice(&1u32.to_le_bytes());
    bytes[root + 12..root + 16].copy_from_slice(&16u32.to_le_bytes());
    let smmu = root + 36;
    bytes[root + 16..root + 20].copy_from_slice(&0x100u32.to_le_bytes());
    bytes[root + 20..root + 24].copy_from_slice(&16u32.to_le_bytes());
    bytes[root + 24..root + 28].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[root + 28..root + 32].copy_from_slice(&(smmu as u32).to_le_bytes());
    bytes[smmu] = NODE_SMMUV3;
    bytes[smmu + 1..smmu + 3].copy_from_slice(&68u16.to_le_bytes());
    bytes[smmu + 16..smmu + 24].copy_from_slice(&0x900_0000u64.to_le_bytes());
    bytes
}

#[test]
fn bounded_iort_nodes_preserve_requester_translation() {
    let bytes = table();
    let config = parse_table(&bytes).unwrap();
    assert_eq!(config.base, 0x900_0000);
    assert_eq!(config.stream_id(0xff), None);
    assert_eq!(config.stream_id(0x100), Some(0x200));
    assert_eq!(config.stream_id(0x10f), Some(0x20f));
    assert_eq!(config.stream_id(0x110), None);
}

#[test]
fn impossible_iort_counts_offsets_lengths_and_mappings_reject() {
    for (offset, value) in [
        (36, u32::MAX),
        (40, 0),
        (40, u32::MAX),
        (IORT_HEADER_SIZE + 8, u32::MAX),
        (IORT_HEADER_SIZE + 12, 0),
    ] {
        let mut bytes = table();
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(parse_table(&bytes).is_none());
    }
    let mut bytes = table();
    bytes[IORT_HEADER_SIZE + 1..IORT_HEADER_SIZE + 3].copy_from_slice(&0u16.to_le_bytes());
    assert!(parse_table(&bytes).is_none());
    assert!(parse_table(&bytes[..IORT_HEADER_SIZE - 1]).is_none());
}
