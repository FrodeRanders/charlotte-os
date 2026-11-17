//! Complete, bounded observations of the locally applied active DNS catalog.
//!
//! This is an observability grammar, separate from persistent catalog state
//! and linearizable lookup. Repeated names represent distinct replica rows.

use alloc::vec::Vec;

pub const MAGIC: &[u8; 8] = b"CDNSCAT1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 24;
pub const RECORD_HEADER_LEN: usize = 16;
pub const MAX_SNAPSHOT_LEN: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Record<'a> {
    pub name: &'a [u8],
    pub node: &'a [u8],
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    InvalidRecord,
    TooLarge,
    AllocationFailed,
}

fn record_len(record: Record<'_>) -> Result<usize, EncodeError> {
    if record.name.is_empty() || record.node.is_empty() || record.generation == 0 {
        return Err(EncodeError::InvalidRecord);
    }
    RECORD_HEADER_LEN
        .checked_add(record.name.len())
        .and_then(|len| len.checked_add(record.node.len()))
        .filter(|len| *len <= MAX_SNAPSHOT_LEN - HEADER_LEN)
        .ok_or(EncodeError::TooLarge)
}

/// Plan the whole set before fallible output allocation. The iterator must
/// yield the same records on both passes; DNS borrows its existing registries
/// throughout. No identity bytes, rows or counts are truncated.
pub fn encode<'a>(
    records: impl Iterator<Item = Record<'a>> + Clone,
) -> Result<Vec<u8>, EncodeError> {
    let mut length = HEADER_LEN;
    let mut count = 0u32;
    for record in records.clone() {
        length = length.checked_add(record_len(record)?).ok_or(EncodeError::TooLarge)?;
        if length > MAX_SNAPSHOT_LEN {
            return Err(EncodeError::TooLarge);
        }
        count = count.checked_add(1).ok_or(EncodeError::TooLarge)?;
    }
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| EncodeError::AllocationFailed)?;
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&VERSION.to_le_bytes());
    output.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    output.extend_from_slice(&(length as u32).to_le_bytes());
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    let mut written = 0u32;
    for record in records {
        // Keep even an inconsistent iterator within the planned allocation.
        if output.len().checked_add(record_len(record)?).is_none_or(|end| end > length) {
            return Err(EncodeError::InvalidRecord);
        }
        output.extend_from_slice(&(record.name.len() as u32).to_le_bytes());
        output.extend_from_slice(&(record.node.len() as u32).to_le_bytes());
        output.extend_from_slice(&record.generation.to_le_bytes());
        output.extend_from_slice(record.name);
        output.extend_from_slice(record.node);
        written += 1;
    }
    if written != count || output.len() != length {
        return Err(EncodeError::InvalidRecord);
    }
    Ok(output)
}

/// Validated borrowed record set. Decoding allocates no row snapshot.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot<'a> {
    payload: &'a [u8],
    count: usize,
}

impl<'a> Snapshot<'a> {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn records(&self) -> Records<'a> {
        Records {
            remaining: self.payload,
            count: self.count,
        }
    }
}

pub struct Records<'a> {
    remaining: &'a [u8],
    count: usize,
}

fn take_record(bytes: &[u8]) -> Option<(Record<'_>, &[u8])> {
    let name_len = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
    let node_len = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
    let generation = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
    let name_end = RECORD_HEADER_LEN.checked_add(name_len)?;
    let end = name_end.checked_add(node_len)?;
    let name = bytes.get(RECORD_HEADER_LEN..name_end)?;
    let node = bytes.get(name_end..end)?;
    if name.is_empty() || node.is_empty() || generation == 0 {
        return None;
    }
    Some((
        Record {
            name,
            node,
            generation,
        },
        bytes.get(end..)?,
    ))
}

impl<'a> Iterator for Records<'a> {
    type Item = Record<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count == 0 {
            return None;
        }
        let (record, remaining) = take_record(self.remaining)?;
        self.remaining = remaining;
        self.count -= 1;
        Some(record)
    }
}

/// Reject retired, incomplete, oversized or extended observations before
/// exposing any row, even when a prefix would be individually readable.
pub fn decode(bytes: &[u8]) -> Option<Snapshot<'_>> {
    if bytes.len() < HEADER_LEN
        || bytes.len() > MAX_SNAPSHOT_LEN
        || bytes.get(..8)? != MAGIC
        || u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?) != VERSION
        || u16::from_le_bytes(bytes.get(10..12)?.try_into().ok()?) as usize != HEADER_LEN
        || u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize != bytes.len()
        || bytes.get(20..24)?.iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let count = u32::from_le_bytes(bytes.get(16..20)?.try_into().ok()?) as usize;
    let mut remaining = bytes.get(HEADER_LEN..)?;
    // Every row needs nonempty name/node plus its fixed header.
    if count > remaining.len() / (RECORD_HEADER_LEN + 2) {
        return None;
    }
    for _ in 0..count {
        remaining = take_record(remaining)?.1;
    }
    if !remaining.is_empty() {
        return None;
    }
    Some(Snapshot {
        payload: &bytes[HEADER_LEN..],
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(records: &[Record<'_>]) -> Vec<u8> {
        encode(records.iter().copied()).unwrap()
    }

    #[test]
    fn golden_record_and_empty_set_have_exact_lengths() {
        let record = Record {
            name: b"a",
            node: b"n",
            generation: 7,
        };
        let bytes = encoded(&[record]);
        let mut expected = b"CDNSCAT1\x01\0\x18\0\x2a\0\0\0\x01\0\0\0\0\0\0\0".to_vec();
        expected.extend_from_slice(b"\x01\0\0\0\x01\0\0\0\x07\0\0\0\0\0\0\0an");
        assert_eq!(bytes, expected);
        let snapshot = decode(&bytes).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.records().collect::<Vec<_>>(), [record]);
        let empty = encoded(&[]);
        assert_eq!(empty.len(), HEADER_LEN);
        assert!(decode(&empty).unwrap().is_empty());
    }

    #[test]
    fn full_identities_and_replica_rows_survive_beyond_one_page() {
        let mut name_a = alloc::vec![b'x'; 300];
        let mut name_b = name_a.clone();
        name_a[299] = b'a';
        name_b[299] = b'b';
        let node = alloc::vec![b'n'; 300];
        let mut records = alloc::vec![Record { name: &name_a, node: &node, generation: 9 }; 8];
        records.push(Record {
            name: &name_b,
            node: &node,
            generation: 10,
        });
        records.push(Record {
            name: &name_a,
            node: b"other-replica",
            generation: 11,
        });
        let bytes = encoded(&records);
        assert!(bytes.len() > 4096);
        assert_eq!(decode(&bytes).unwrap().records().collect::<Vec<_>>(), records);
    }

    #[test]
    fn exact_wire_ceiling_and_invalid_records_reject_without_prefixes() {
        let name = alloc::vec![b'n'; MAX_SNAPSHOT_LEN - HEADER_LEN - RECORD_HEADER_LEN - 1];
        let record = Record {
            name: &name,
            node: b"x",
            generation: 1,
        };
        let bytes = encoded(&[record]);
        assert_eq!(bytes.len(), MAX_SNAPSHOT_LEN);
        assert_eq!(decode(&bytes).unwrap().records().next(), Some(record));
        let longer = alloc::vec![b'n'; name.len() + 1];
        assert_eq!(
            encode(
                [Record {
                    name: &longer,
                    ..record
                }]
                .into_iter()
            ),
            Err(EncodeError::TooLarge)
        );
        assert_eq!(encode([record, record].into_iter()), Err(EncodeError::TooLarge));
        for invalid in [
            Record {
                name: b"",
                ..record
            },
            Record {
                node: b"",
                ..record
            },
            Record {
                generation: 0,
                ..record
            },
        ] {
            assert_eq!(encode([invalid].into_iter()), Err(EncodeError::InvalidRecord));
        }
        let mut oversized = bytes;
        oversized.push(0);
        assert!(decode(&oversized).is_none());
    }

    #[test]
    fn retired_truncated_extended_and_inconsistent_sets_reject_before_rows() {
        let bytes = encoded(&[Record {
            name: b"alpha",
            node: b"node",
            generation: 1,
        }]);
        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end]).is_none(), "prefix={end}");
        }
        // The retired count/u8-string page, including its complete row.
        assert!(decode(b"\x01\0\0\0\x05alpha\x04node\x01\0\0\0\0\0\0\0").is_none());
        for (offset, value) in [
            (0, b'X'),
            (8, 2),
            (10, 16),
            (16, 0),
            (16, 2),
            (20, 1),
            (HEADER_LEN, 0),
            (HEADER_LEN + 4, 0),
            (HEADER_LEN + 8, 0),
        ] {
            let mut corrupt = bytes.clone();
            corrupt[offset] = value;
            assert!(decode(&corrupt).is_none(), "offset={offset}");
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(decode(&extended).is_none());
        let len = extended.len() as u32;
        extended[12..16].copy_from_slice(&len.to_le_bytes());
        assert!(decode(&extended).is_none());
        let mut short_payload = bytes;
        short_payload[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&short_payload).is_none());
    }
}
