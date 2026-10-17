//! Complete active-publication observations; retired NSST pages are unsupported.

use alloc::vec::Vec;

use crate::{
    MAX_LIVE_PUBLICATIONS,
    MAX_NAME_LEN,
};

pub const MAGIC: &[u8; 8] = b"CNSSTAT1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 24;
pub const RECORD_HEADER_LEN: usize = 12;
pub const MAX_SNAPSHOT_LEN: usize =
    HEADER_LEN + MAX_LIVE_PUBLICATIONS * (RECORD_HEADER_LEN + MAX_NAME_LEN);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Record<'a> {
    pub name: &'a [u8],
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    InvalidRecord,
    TooLarge,
    AllocationFailed,
}

fn valid(record: Record<'_>, previous: Option<&[u8]>) -> bool {
    !record.name.is_empty()
        && record.name.len() <= MAX_NAME_LEN
        && (1..=i64::MAX as u64).contains(&record.generation)
        && previous.is_none_or(|name| name < record.name)
}

/// NS borrows its ordered registry through both passes. Plan the complete set
/// before allocation; neither a prefix nor duplicate/unsorted names are valid.
pub fn encode<'a>(
    records: impl Iterator<Item = Record<'a>> + Clone,
    pending: u32,
) -> Result<Vec<u8>, EncodeError> {
    let mut length = HEADER_LEN;
    let mut count = 0usize;
    let mut previous = None;
    for record in records.clone() {
        if !valid(record, previous) {
            return Err(EncodeError::InvalidRecord);
        }
        count += 1;
        if count > MAX_LIVE_PUBLICATIONS {
            return Err(EncodeError::TooLarge);
        }
        length += RECORD_HEADER_LEN + record.name.len();
        previous = Some(record.name);
    }
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| EncodeError::AllocationFailed)?;
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&VERSION.to_le_bytes());
    output.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    output.extend_from_slice(&(length as u32).to_le_bytes());
    output.extend_from_slice(&(count as u32).to_le_bytes());
    output.extend_from_slice(&pending.to_le_bytes());
    let mut written = 0;
    previous = None;
    for record in records {
        if !valid(record, previous)
            || written >= count
            || output.len() + RECORD_HEADER_LEN + record.name.len() > length
        {
            return Err(EncodeError::InvalidRecord);
        }
        output.extend_from_slice(&(record.name.len() as u32).to_le_bytes());
        output.extend_from_slice(&record.generation.to_le_bytes());
        output.extend_from_slice(record.name);
        written += 1;
        previous = Some(record.name);
    }
    if written != count || output.len() != length {
        return Err(EncodeError::InvalidRecord);
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot<'a> {
    payload: &'a [u8],
    count: usize,
    pending: u32,
}

impl<'a> Snapshot<'a> {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Count every retained lookup request, including requests for the same missing name.
    pub fn pending(&self) -> u32 {
        self.pending
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
    let length = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
    let end = RECORD_HEADER_LEN.checked_add(length)?;
    let record = Record {
        name: bytes.get(RECORD_HEADER_LEN..end)?,
        generation: u64::from_le_bytes(bytes.get(4..12)?.try_into().ok()?),
    };
    valid(record, None).then_some((record, bytes.get(end..)?))
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

/// Validate every row and exact framing before exposing any observation.
pub fn decode(bytes: &[u8]) -> Option<Snapshot<'_>> {
    if !(HEADER_LEN..=MAX_SNAPSHOT_LEN).contains(&bytes.len())
        || bytes.get(..8)? != MAGIC
        || u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?) != VERSION
        || u16::from_le_bytes(bytes.get(10..12)?.try_into().ok()?) as usize != HEADER_LEN
        || u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize != bytes.len()
    {
        return None;
    }
    let count = u32::from_le_bytes(bytes.get(16..20)?.try_into().ok()?) as usize;
    let pending = u32::from_le_bytes(bytes.get(20..24)?.try_into().ok()?);
    let mut remaining = bytes.get(HEADER_LEN..)?;
    if count > MAX_LIVE_PUBLICATIONS || count > remaining.len() / (RECORD_HEADER_LEN + 1) {
        return None;
    }
    let mut previous = None;
    for _ in 0..count {
        let (record, rest) = take_record(remaining)?;
        if !valid(record, previous) {
            return None;
        }
        previous = Some(record.name);
        remaining = rest;
    }
    remaining.is_empty().then_some(Snapshot {
        payload: &bytes[HEADER_LEN..],
        count,
        pending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_and_empty_snapshots() {
        let record = Record {
            name: b"echo",
            generation: 7,
        };
        let bytes = encode([record].into_iter(), 2).unwrap();
        assert_eq!(
            bytes,
            [
                MAGIC.as_slice(),
                &1u16.to_le_bytes(),
                &24u16.to_le_bytes(),
                &40u32.to_le_bytes(),
                &1u32.to_le_bytes(),
                &2u32.to_le_bytes(),
                &4u32.to_le_bytes(),
                &7u64.to_le_bytes(),
                b"echo"
            ]
            .concat()
        );
        let snapshot = decode(&bytes).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.pending(), 2);
        assert_eq!(snapshot.records().collect::<Vec<_>>(), [record]);
        let empty = encode(core::iter::empty(), 9).unwrap();
        assert_eq!(empty.len(), HEADER_LEN);
        assert!(decode(&empty).unwrap().is_empty());
        assert_eq!(decode(&empty).unwrap().pending(), 9);
    }

    #[test]
    fn complete_maximum_set_and_full_binary_names() {
        let names: Vec<_> = (0..MAX_LIVE_PUBLICATIONS)
            .map(|n| {
                let mut name = alloc::vec![b'a'; MAX_NAME_LEN];
                name[MAX_NAME_LEN - 1] = n as u8;
                name
            })
            .collect();
        let records: Vec<_> = names
            .iter()
            .map(|name| Record {
                name,
                generation: i64::MAX as u64,
            })
            .collect();
        let bytes = encode(records.iter().copied(), u32::MAX).unwrap();
        assert_eq!(bytes.len(), MAX_SNAPSHOT_LEN);
        assert!(bytes.len() > 4096);
        assert_eq!(decode(&bytes).unwrap().records().collect::<Vec<_>>(), records);
        let name = alloc::vec![0xff; 255];
        let record = Record {
            name: &name,
            generation: 1,
        };
        let bytes = encode([record].into_iter(), 0).unwrap();
        assert_eq!(decode(&bytes).unwrap().records().next(), Some(record));
        let mut too_many = records;
        too_many.push(Record {
            name: b"z",
            generation: 1,
        });
        assert_eq!(encode(too_many.into_iter(), 0), Err(EncodeError::TooLarge));
    }

    #[test]
    fn invalid_identities_generations_and_order_reject() {
        let long = alloc::vec![0; MAX_NAME_LEN + 1];
        for record in [
            Record {
                name: b"",
                generation: 1,
            },
            Record {
                name: &long,
                generation: 1,
            },
            Record {
                name: b"a",
                generation: 0,
            },
            Record {
                name: b"a",
                generation: u64::MAX,
            },
        ] {
            assert_eq!(encode([record].into_iter(), 0), Err(EncodeError::InvalidRecord));
        }
        let a = Record {
            name: b"a",
            generation: 1,
        };
        let b = Record {
            name: b"b",
            generation: 2,
        };
        for records in [[a, a], [b, a]] {
            assert_eq!(encode(records.into_iter(), 0), Err(EncodeError::InvalidRecord));
        }
    }

    #[test]
    fn malformed_prefixes_counts_and_retired_pages_reject_before_rows() {
        let bytes = encode(
            [
                Record {
                    name: b"a",
                    generation: 1,
                },
                Record {
                    name: b"b",
                    generation: 2,
                },
            ]
            .into_iter(),
            3,
        )
        .unwrap();
        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end]).is_none(), "prefix={end}");
        }
        assert!(decode(b"NSST\x01\0\0\0\0\0\0\0\x04echo").is_none());
        for (offset, value) in [
            (0, b'X'),
            (8, 2),
            (10, 0),
            (12, 0),
            (16, 0),
            (16, 1),
            (16, 3),
            (16, 65),
            (24, 0),
            (28, 0),
            (24, 255),
            (49, b'a'),
        ] {
            let mut corrupt = bytes.clone();
            corrupt[offset] = value;
            assert!(decode(&corrupt).is_none(), "offset={offset}");
        }
        let mut extended = bytes;
        extended.push(0);
        assert!(decode(&extended).is_none());
        let len = extended.len() as u32;
        extended[12..16].copy_from_slice(&len.to_le_bytes());
        assert!(decode(&extended).is_none());
        assert!(decode(&alloc::vec![0; MAX_SNAPSHOT_LEN + 1]).is_none());
    }
}
