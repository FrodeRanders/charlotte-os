use std::vec::Vec;

use super::*;
use crate::{
    THREAD_STATISTICS_HEADER_U64S as HEADER,
    thread_statistics_header as h,
};

fn payload(observer: bool) -> Vec<u8> {
    let mut header = [0; HEADER];
    header[h::MAGIC] = crate::THREAD_STATISTICS_MAGIC;
    header[h::VERSION] = crate::THREAD_STATISTICS_VERSION;
    header[h::RECORD_BYTES] = (crate::THREAD_STATISTICS_RECORD_U64S * 8) as u64;
    header[h::DOMAIN_RECORD_BYTES] = (crate::THREAD_STATISTICS_DOMAIN_RECORD_U64S * 8) as u64;
    header[h::METADATA_RECORD_BYTES] = (WORDS * 8) as u64;
    header[h::METADATA_RECORD_COUNT] = if observer {
        COUNT as u64
    } else {
        0
    };
    let mut bytes: Vec<_> = header.into_iter().flat_map(u64::to_le_bytes).collect();
    if observer {
        for (index, id) in ALL.iter().enumerate() {
            let mut sample = Sample {
                id: *id,
                total_limit: [100; 2],
                ordinary_limit: [75; 2],
                used: Some(([20; 2], [10; 2])),
            };
            if id.dimensions() == 1 {
                sample.total_limit[1] = 0;
                sample.ordinary_limit[1] = 0;
                sample.used = Some(([20, 0], [10, 0]));
            }
            if index == 0 {
                sample.used = None;
            }
            bytes.extend(sample.words().into_iter().flat_map(u64::to_le_bytes));
        }
    }
    bytes
}
fn replace(bytes: &mut [u8], index: usize, value: u64) {
    bytes[index * 8..(index + 1) * 8].copy_from_slice(&value.to_le_bytes());
}
#[test]
fn observer_and_self_sections() {
    assert_eq!(from_statistics(&payload(false)).unwrap().len(), 0);
    let bytes = payload(true);
    let samples: Vec<_> = from_statistics(&bytes).unwrap().collect();
    assert_eq!(samples.len(), COUNT);
    assert_eq!(samples[0].used, None);
    assert_eq!(samples[1].used, Some(([20; 2], [10; 2])));
    assert_eq!(samples.last().unwrap().id, Id::IpcNodes);
}
#[test]
fn reject_wrong_versions_lengths_and_counts() {
    let bytes = payload(true);
    for length in 0..bytes.len() {
        assert!(from_statistics(&bytes[..length]).is_none());
    }
    let mut padded = bytes.clone();
    padded.push(0);
    assert!(from_statistics(&padded).is_none());
    for (index, value) in [
        (h::VERSION, 8),
        (h::VERSION, 9),
        (h::RECORD_BYTES, 0),
        (h::DOMAIN_RECORD_BYTES, 0),
        (h::METADATA_RECORD_BYTES, 0),
        (h::RECORD_COUNT, u64::MAX),
        (h::DOMAIN_RECORD_COUNT, u64::MAX),
        (h::METADATA_RECORD_COUNT, 1),
        (h::METADATA_RECORD_COUNT, u64::MAX),
    ] {
        let mut damaged = bytes.clone();
        replace(&mut damaged, index, value);
        assert!(from_statistics(&damaged).is_none(), "field {index}");
    }
}
#[test]
fn reject_record_identity_state_limits_and_usage() {
    let bytes = payload(true);
    for (index, value) in [
        (HEADER, 1),
        (HEADER + 1, 1),
        (HEADER + 2, 2),
        (HEADER + 3, 0),
        (HEADER + 5, 101),
        (HEADER + 7, 1), // busy usage
        (HEADER + WORDS + 7, 101),
        (HEADER + WORDS + 9, 76),
        (HEADER + WORDS + 9, 21),
        (HEADER + (COUNT - 2) * WORDS + 4, 1), // one-dimensional ObserverLists
    ] {
        let mut damaged = bytes.clone();
        replace(&mut damaged, index, value);
        assert!(from_statistics(&damaged).is_none(), "field {index}");
    }
}
