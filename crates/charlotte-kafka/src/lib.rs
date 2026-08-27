//! Bounded Kafka wire-protocol primitives for CharlotteOS.
//!
//! This crate deliberately implements a small, stable set of legacy request
//! versions understood by modern brokers. `ApiVersions` is still negotiated
//! before use; a broker that no longer accepts one of these versions is
//! rejected instead of receiving a guessed schema.
#![no_std]

extern crate alloc;

use alloc::{
    string::String,
    vec::Vec,
};

pub mod api {
    pub const PRODUCE: i16 = 0;
    pub const FETCH: i16 = 1;
    pub const LIST_OFFSETS: i16 = 2;
    pub const METADATA: i16 = 3;
    pub const OFFSET_COMMIT: i16 = 8;
    pub const OFFSET_FETCH: i16 = 9;
    pub const FIND_COORDINATOR: i16 = 10;
    pub const API_VERSIONS: i16 = 18;
    pub const INIT_PRODUCER_ID: i16 = 22;
    pub const ADD_PARTITIONS_TO_TXN: i16 = 24;
    pub const ADD_OFFSETS_TO_TXN: i16 = 25;
    pub const END_TXN: i16 = 26;
    pub const TXN_OFFSET_COMMIT: i16 = 28;
}

pub mod version {
    pub const PRODUCE: i16 = 3;
    pub const FETCH: i16 = 4;
    pub const LIST_OFFSETS: i16 = 1;
    pub const METADATA: i16 = 1;
    pub const OFFSET_COMMIT: i16 = 2;
    pub const OFFSET_FETCH: i16 = 1;
    pub const FIND_COORDINATOR: i16 = 1;
    pub const API_VERSIONS: i16 = 0;
    pub const INIT_PRODUCER_ID: i16 = 0;
    pub const ADD_PARTITIONS_TO_TXN: i16 = 0;
    pub const ADD_OFFSETS_TO_TXN: i16 = 0;
    pub const END_TXN: i16 = 0;
    pub const TXN_OFFSET_COMMIT: i16 = 0;
}

pub const NO_ERROR: i16 = 0;
pub const UNKNOWN_TOPIC_OR_PARTITION: i16 = 3;
pub const NOT_LEADER_OR_FOLLOWER: i16 = 6;
pub const REQUEST_TIMED_OUT: i16 = 7;
pub const COORDINATOR_LOAD_IN_PROGRESS: i16 = 14;
pub const COORDINATOR_NOT_AVAILABLE: i16 = 15;
pub const NOT_COORDINATOR: i16 = 16;
pub const NOT_ENOUGH_REPLICAS: i16 = 19;
pub const NOT_ENOUGH_REPLICAS_AFTER_APPEND: i16 = 20;
pub const INVALID_PRODUCER_EPOCH: i16 = 47;
pub const CONCURRENT_TRANSACTIONS: i16 = 51;
pub const TRANSACTION_COORDINATOR_FENCED: i16 = 52;
pub const PRODUCER_FENCED: i16 = 90;

pub const fn is_retriable_broker_error(error: i16) -> bool {
    matches!(
        error,
        REQUEST_TIMED_OUT
            | COORDINATOR_LOAD_IN_PROGRESS
            | COORDINATOR_NOT_AVAILABLE
            | NOT_COORDINATOR
            | CONCURRENT_TRANSACTIONS
    )
}

pub const MAX_FRAME_LEN: usize = 1024 * 1024;
pub const MAX_STRING_LEN: usize = 8 * 1024;
pub const MAX_ARRAY_LEN: usize = 16 * 1024;
pub const MAX_RECORDS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Incomplete,
    Invalid,
    TooLarge,
    Correlation,
    UnsupportedVersion,
    Checksum,
    Broker(i16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApiVersion {
    pub api_key: i16,
    pub min: i16,
    pub max: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiVersions {
    pub error: i16,
    pub versions: Vec<ApiVersion>,
}

impl ApiVersions {
    pub fn supports(&self, api_key: i16, wanted: i16) -> bool {
        self.error == NO_ERROR
            && self.versions.iter().any(|version| {
                version.api_key == api_key && version.min <= wanted && wanted <= version.max
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Broker {
    pub node_id: i32,
    pub host: String,
    pub port: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartitionMetadata {
    pub error: i16,
    pub partition: i32,
    pub leader: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Metadata {
    pub brokers: Vec<Broker>,
    pub topic_error: i16,
    pub partitions: Vec<PartitionMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicMetadata {
    pub topic: Vec<u8>,
    pub error: i16,
    pub partitions: Vec<PartitionMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataBatch {
    pub brokers: Vec<Broker>,
    pub topics: Vec<TopicMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Coordinator {
    pub error: i16,
    pub node_id: i32,
    pub host: String,
    pub port: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProducerIdentity {
    pub producer_id: i64,
    pub producer_epoch: i16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProduceResult {
    pub error: i16,
    pub base_offset: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
    pub offset: i64,
    pub timestamp: i64,
    pub key: Option<Vec<u8>>,
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchResult {
    pub error: i16,
    pub high_watermark: i64,
    pub last_stable_offset: i64,
    pub records: Vec<Record>,
}

pub struct RecordInput<'a> {
    pub timestamp_ms: i64,
    pub key: Option<&'a [u8]>,
    pub value: Option<&'a [u8]>,
}

struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn request(
        api_key: i16,
        api_version: i16,
        correlation: i32,
        client_id: &[u8],
    ) -> Result<Self, Error> {
        let mut encoder = Self {
            bytes: Vec::new(),
        };
        encoder.i32(0);
        encoder.i16(api_key);
        encoder.i16(api_version);
        encoder.i32(correlation);
        encoder.string(client_id)?;
        Ok(encoder)
    }

    fn finish(mut self) -> Result<Vec<u8>, Error> {
        let payload = self.bytes.len().checked_sub(4).ok_or(Error::Invalid)?;
        if payload > MAX_FRAME_LEN || payload > i32::MAX as usize {
            return Err(Error::TooLarge);
        }
        self.bytes[..4].copy_from_slice(&(payload as i32).to_be_bytes());
        Ok(self.bytes)
    }

    fn i8(&mut self, value: i8) {
        self.bytes.push(value as u8);
    }

    fn i16(&mut self, value: i16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn string(&mut self, value: &[u8]) -> Result<(), Error> {
        if value.len() > MAX_STRING_LEN || value.len() > i16::MAX as usize {
            return Err(Error::TooLarge);
        }
        self.i16(value.len() as i16);
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn nullable_string(&mut self, value: Option<&[u8]>) -> Result<(), Error> {
        match value {
            Some(value) => self.string(value),
            None => {
                self.i16(-1);
                Ok(())
            }
        }
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), Error> {
        if value.len() > MAX_FRAME_LEN || value.len() > i32::MAX as usize {
            return Err(Error::TooLarge);
        }
        self.i32(value.len() as i32);
        self.bytes.extend_from_slice(value);
        Ok(())
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self.offset.checked_add(len).ok_or(Error::Invalid)?;
        let value = self.bytes.get(self.offset..end).ok_or(Error::Incomplete)?;
        self.offset = end;
        Ok(value)
    }

    fn i8(&mut self) -> Result<i8, Error> {
        Ok(self.take(1)?[0] as i8)
    }

    fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().map_err(|_| Error::Invalid)?))
    }

    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().map_err(|_| Error::Invalid)?))
    }

    fn i64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().map_err(|_| Error::Invalid)?))
    }

    fn bool(&mut self) -> Result<bool, Error> {
        match self.i8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Invalid),
        }
    }

    fn string_bytes(&mut self) -> Result<&'a [u8], Error> {
        let len = self.i16()?;
        if len < 0 {
            return Err(Error::Invalid);
        }
        let len = len as usize;
        if len > MAX_STRING_LEN {
            return Err(Error::TooLarge);
        }
        self.take(len)
    }

    fn string(&mut self) -> Result<String, Error> {
        let bytes = self.string_bytes()?;
        let text = core::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
        Ok(String::from(text))
    }

    fn nullable_string(&mut self) -> Result<Option<&'a [u8]>, Error> {
        let len = self.i16()?;
        if len == -1 {
            return Ok(None);
        }
        if len < 0 {
            return Err(Error::Invalid);
        }
        let len = len as usize;
        if len > MAX_STRING_LEN {
            return Err(Error::TooLarge);
        }
        Ok(Some(self.take(len)?))
    }

    fn bytes(&mut self) -> Result<Option<&'a [u8]>, Error> {
        let len = self.i32()?;
        if len == -1 {
            return Ok(None);
        }
        if len < 0 {
            return Err(Error::Invalid);
        }
        let len = len as usize;
        if len > MAX_FRAME_LEN {
            return Err(Error::TooLarge);
        }
        Ok(Some(self.take(len)?))
    }

    fn array_len(&mut self) -> Result<usize, Error> {
        let len = self.i32()?;
        if len < 0 {
            return Err(Error::Invalid);
        }
        let len = len as usize;
        if len > MAX_ARRAY_LEN {
            return Err(Error::TooLarge);
        }
        Ok(len)
    }

    fn nullable_array_len(&mut self) -> Result<Option<usize>, Error> {
        let len = self.i32()?;
        if len == -1 {
            return Ok(None);
        }
        if len < 0 {
            return Err(Error::Invalid);
        }
        let len = len as usize;
        if len > MAX_ARRAY_LEN {
            return Err(Error::TooLarge);
        }
        Ok(Some(len))
    }

    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

fn response<'a>(frame: &'a [u8], correlation: i32) -> Result<Decoder<'a>, Error> {
    if frame.len() < 8 {
        return Err(Error::Incomplete);
    }
    let payload = i32::from_be_bytes(frame[..4].try_into().map_err(|_| Error::Invalid)?);
    if payload < 4 || payload as usize > MAX_FRAME_LEN {
        return Err(Error::TooLarge);
    }
    if payload as usize + 4 != frame.len() {
        return Err(Error::Incomplete);
    }
    let mut decoder = Decoder::new(&frame[4..]);
    if decoder.i32()? != correlation {
        return Err(Error::Correlation);
    }
    Ok(decoder)
}

pub fn api_versions_request(correlation: i32, client_id: &[u8]) -> Result<Vec<u8>, Error> {
    Encoder::request(api::API_VERSIONS, version::API_VERSIONS, correlation, client_id)?.finish()
}

pub fn parse_api_versions(frame: &[u8], correlation: i32) -> Result<ApiVersions, Error> {
    let mut decoder = response(frame, correlation)?;
    let error = decoder.i16()?;
    let count = decoder.array_len()?;
    let mut versions = Vec::with_capacity(count);
    for _ in 0..count {
        versions.push(ApiVersion {
            api_key: decoder.i16()?,
            min: decoder.i16()?,
            max: decoder.i16()?,
        });
    }
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    Ok(ApiVersions {
        error,
        versions,
    })
}

pub fn metadata_request(
    correlation: i32,
    client_id: &[u8],
    topic: &[u8],
) -> Result<Vec<u8>, Error> {
    metadata_request_many(correlation, client_id, &[topic])
}

pub fn metadata_request_many(
    correlation: i32,
    client_id: &[u8],
    topics: &[&[u8]],
) -> Result<Vec<u8>, Error> {
    if topics.is_empty() || topics.len() > i32::MAX as usize {
        return Err(Error::Invalid);
    }
    let mut encoder = Encoder::request(api::METADATA, version::METADATA, correlation, client_id)?;
    encoder.i32(topics.len() as i32);
    for topic in topics {
        encoder.string(topic)?;
    }
    encoder.finish()
}

pub fn parse_metadata(
    frame: &[u8],
    correlation: i32,
    wanted_topic: &[u8],
) -> Result<Metadata, Error> {
    let batch = parse_metadata_many(frame, correlation)?;
    let topic = batch.topics.into_iter().find(|topic| topic.topic == wanted_topic).unwrap_or(
        TopicMetadata {
            topic: wanted_topic.to_vec(),
            error: UNKNOWN_TOPIC_OR_PARTITION,
            partitions: Vec::new(),
        },
    );
    Ok(Metadata {
        brokers: batch.brokers,
        topic_error: topic.error,
        partitions: topic.partitions,
    })
}

pub fn parse_metadata_many(frame: &[u8], correlation: i32) -> Result<MetadataBatch, Error> {
    let mut decoder = response(frame, correlation)?;
    let broker_count = decoder.array_len()?;
    let mut brokers = Vec::with_capacity(broker_count);
    for _ in 0..broker_count {
        brokers.push(Broker {
            node_id: decoder.i32()?,
            host: decoder.string()?,
            port: decoder.i32()?,
        });
        let _ = decoder.nullable_string()?;
    }
    let _controller_id = decoder.i32()?;
    let topic_count = decoder.array_len()?;
    let mut topics = Vec::with_capacity(topic_count);
    for _ in 0..topic_count {
        let error = decoder.i16()?;
        let topic = decoder.string_bytes()?.to_vec();
        let _internal = decoder.bool()?;
        let partition_count = decoder.array_len()?;
        let mut partitions = Vec::with_capacity(partition_count);
        for _ in 0..partition_count {
            let metadata = PartitionMetadata {
                error: decoder.i16()?,
                partition: decoder.i32()?,
                leader: decoder.i32()?,
            };
            let replicas = decoder.array_len()?;
            for _ in 0..replicas {
                let _ = decoder.i32()?;
            }
            let isr = decoder.array_len()?;
            for _ in 0..isr {
                let _ = decoder.i32()?;
            }
            partitions.push(metadata);
        }
        topics.push(TopicMetadata {
            topic,
            error,
            partitions,
        });
    }
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    Ok(MetadataBatch {
        brokers,
        topics,
    })
}

pub fn find_coordinator_request(
    correlation: i32,
    client_id: &[u8],
    key: &[u8],
    transaction: bool,
) -> Result<Vec<u8>, Error> {
    let mut encoder =
        Encoder::request(api::FIND_COORDINATOR, version::FIND_COORDINATOR, correlation, client_id)?;
    encoder.string(key)?;
    encoder.i8(if transaction {
        1
    } else {
        0
    });
    encoder.finish()
}

pub fn parse_find_coordinator(frame: &[u8], correlation: i32) -> Result<Coordinator, Error> {
    let mut decoder = response(frame, correlation)?;
    let _throttle = decoder.i32()?;
    let error = decoder.i16()?;
    let _message = decoder.nullable_string()?;
    let coordinator = Coordinator {
        error,
        node_id: decoder.i32()?,
        host: decoder.string()?,
        port: decoder.i32()?,
    };
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    Ok(coordinator)
}

pub fn init_producer_id_request(
    correlation: i32,
    client_id: &[u8],
    transactional_id: Option<&[u8]>,
    timeout_ms: i32,
) -> Result<Vec<u8>, Error> {
    let mut encoder =
        Encoder::request(api::INIT_PRODUCER_ID, version::INIT_PRODUCER_ID, correlation, client_id)?;
    encoder.nullable_string(transactional_id)?;
    encoder.i32(timeout_ms);
    encoder.finish()
}

pub fn parse_init_producer_id(frame: &[u8], correlation: i32) -> Result<ProducerIdentity, Error> {
    let mut decoder = response(frame, correlation)?;
    let _throttle = decoder.i32()?;
    let error = decoder.i16()?;
    if error != NO_ERROR {
        return Err(Error::Broker(error));
    }
    let identity = ProducerIdentity {
        producer_id: decoder.i64()?,
        producer_epoch: decoder.i16()?,
    };
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    Ok(identity)
}

pub fn add_partitions_to_txn_request(
    correlation: i32,
    client_id: &[u8],
    transactional_id: &[u8],
    producer: ProducerIdentity,
    topic: &[u8],
    partition: i32,
) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::request(
        api::ADD_PARTITIONS_TO_TXN,
        version::ADD_PARTITIONS_TO_TXN,
        correlation,
        client_id,
    )?;
    encoder.string(transactional_id)?;
    encoder.i64(producer.producer_id);
    encoder.i16(producer.producer_epoch);
    encoder.i32(1);
    encoder.string(topic)?;
    encoder.i32(1);
    encoder.i32(partition);
    encoder.finish()
}

pub fn parse_partition_error(
    frame: &[u8],
    correlation: i32,
    expected_topic: &[u8],
    expected_partition: i32,
) -> Result<(), Error> {
    let mut decoder = response(frame, correlation)?;
    let _throttle = decoder.i32()?;
    let topics = decoder.array_len()?;
    let mut found = None;
    for _ in 0..topics {
        let topic = decoder.string_bytes()?;
        let partitions = decoder.array_len()?;
        for _ in 0..partitions {
            let partition = decoder.i32()?;
            let error = decoder.i16()?;
            if topic == expected_topic && partition == expected_partition {
                found = Some(error);
            }
        }
    }
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    match found {
        Some(NO_ERROR) => Ok(()),
        Some(error) => Err(Error::Broker(error)),
        None => Err(Error::Invalid),
    }
}

pub fn add_offsets_to_txn_request(
    correlation: i32,
    client_id: &[u8],
    transactional_id: &[u8],
    producer: ProducerIdentity,
    group_id: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::request(
        api::ADD_OFFSETS_TO_TXN,
        version::ADD_OFFSETS_TO_TXN,
        correlation,
        client_id,
    )?;
    encoder.string(transactional_id)?;
    encoder.i64(producer.producer_id);
    encoder.i16(producer.producer_epoch);
    encoder.string(group_id)?;
    encoder.finish()
}

pub fn parse_top_level_error(frame: &[u8], correlation: i32) -> Result<(), Error> {
    let mut decoder = response(frame, correlation)?;
    let _throttle = decoder.i32()?;
    let error = decoder.i16()?;
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    if error == NO_ERROR {
        Ok(())
    } else {
        Err(Error::Broker(error))
    }
}

pub struct TxnOffsetCommit<'a> {
    pub transactional_id: &'a [u8],
    pub group_id: &'a [u8],
    pub producer: ProducerIdentity,
    pub topic: &'a [u8],
    pub partition: i32,
    pub next_offset: i64,
}

pub fn txn_offset_commit_request(
    correlation: i32,
    client_id: &[u8],
    commit: TxnOffsetCommit<'_>,
) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::request(
        api::TXN_OFFSET_COMMIT,
        version::TXN_OFFSET_COMMIT,
        correlation,
        client_id,
    )?;
    encoder.string(commit.transactional_id)?;
    encoder.string(commit.group_id)?;
    encoder.i64(commit.producer.producer_id);
    encoder.i16(commit.producer.producer_epoch);
    encoder.i32(1);
    encoder.string(commit.topic)?;
    encoder.i32(1);
    encoder.i32(commit.partition);
    encoder.i64(commit.next_offset);
    encoder.nullable_string(None)?;
    encoder.finish()
}

pub fn end_txn_request(
    correlation: i32,
    client_id: &[u8],
    transactional_id: &[u8],
    producer: ProducerIdentity,
    commit: bool,
) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::request(api::END_TXN, version::END_TXN, correlation, client_id)?;
    encoder.string(transactional_id)?;
    encoder.i64(producer.producer_id);
    encoder.i16(producer.producer_epoch);
    encoder.i8(commit as i8);
    encoder.finish()
}

pub fn offset_fetch_request(
    correlation: i32,
    client_id: &[u8],
    group_id: &[u8],
    topic: &[u8],
    partition: i32,
) -> Result<Vec<u8>, Error> {
    let mut encoder =
        Encoder::request(api::OFFSET_FETCH, version::OFFSET_FETCH, correlation, client_id)?;
    encoder.string(group_id)?;
    encoder.i32(1);
    encoder.string(topic)?;
    encoder.i32(1);
    encoder.i32(partition);
    encoder.finish()
}

pub fn parse_offset_fetch(
    frame: &[u8],
    correlation: i32,
    expected_topic: &[u8],
    expected_partition: i32,
) -> Result<Option<i64>, Error> {
    let mut decoder = response(frame, correlation)?;
    let topics = decoder.array_len()?;
    let mut found = None;
    for _ in 0..topics {
        let topic = decoder.string_bytes()?;
        let partitions = decoder.array_len()?;
        for _ in 0..partitions {
            let partition = decoder.i32()?;
            let offset = decoder.i64()?;
            let _metadata = decoder.nullable_string()?;
            let error = decoder.i16()?;
            if topic == expected_topic && partition == expected_partition {
                if error != NO_ERROR {
                    return Err(Error::Broker(error));
                }
                found = Some((offset >= 0).then_some(offset));
            }
        }
    }
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    found.ok_or(Error::Invalid)
}

pub fn offset_commit_request(
    correlation: i32,
    client_id: &[u8],
    group_id: &[u8],
    topic: &[u8],
    partition: i32,
    next_offset: i64,
) -> Result<Vec<u8>, Error> {
    let mut encoder =
        Encoder::request(api::OFFSET_COMMIT, version::OFFSET_COMMIT, correlation, client_id)?;
    encoder.string(group_id)?;
    encoder.i32(-1);
    encoder.string(&[])?;
    encoder.i64(-1);
    encoder.i32(1);
    encoder.string(topic)?;
    encoder.i32(1);
    encoder.i32(partition);
    encoder.i64(next_offset);
    encoder.nullable_string(None)?;
    encoder.finish()
}

pub fn parse_offset_commit(
    frame: &[u8],
    correlation: i32,
    expected_topic: &[u8],
    expected_partition: i32,
) -> Result<(), Error> {
    let mut decoder = response(frame, correlation)?;
    let topics = decoder.array_len()?;
    let mut found = None;
    for _ in 0..topics {
        let topic = decoder.string_bytes()?;
        let partitions = decoder.array_len()?;
        for _ in 0..partitions {
            let partition = decoder.i32()?;
            let error = decoder.i16()?;
            if topic == expected_topic && partition == expected_partition {
                found = Some(error);
            }
        }
    }
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    match found {
        Some(NO_ERROR) => Ok(()),
        Some(error) => Err(Error::Broker(error)),
        None => Err(Error::Invalid),
    }
}

pub fn list_offsets_request(
    correlation: i32,
    client_id: &[u8],
    topic: &[u8],
    partition: i32,
    earliest: bool,
) -> Result<Vec<u8>, Error> {
    let mut encoder =
        Encoder::request(api::LIST_OFFSETS, version::LIST_OFFSETS, correlation, client_id)?;
    encoder.i32(-1);
    encoder.i32(1);
    encoder.string(topic)?;
    encoder.i32(1);
    encoder.i32(partition);
    encoder.i64(
        if earliest {
            -2
        } else {
            -1
        },
    );
    encoder.finish()
}

pub fn parse_list_offsets(
    frame: &[u8],
    correlation: i32,
    expected_topic: &[u8],
    expected_partition: i32,
) -> Result<i64, Error> {
    let mut decoder = response(frame, correlation)?;
    let topics = decoder.array_len()?;
    let mut found = None;
    for _ in 0..topics {
        let topic = decoder.string_bytes()?;
        let partitions = decoder.array_len()?;
        for _ in 0..partitions {
            let partition = decoder.i32()?;
            let error = decoder.i16()?;
            let _timestamp = decoder.i64()?;
            let offset = decoder.i64()?;
            if topic == expected_topic && partition == expected_partition {
                if error != NO_ERROR {
                    return Err(Error::Broker(error));
                }
                found = Some(offset);
            }
        }
    }
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    found.ok_or(Error::Invalid)
}

pub fn produce_request(
    correlation: i32,
    client_id: &[u8],
    transactional_id: Option<&[u8]>,
    topic: &[u8],
    partition: i32,
    record_batch: &[u8],
    timeout_ms: i32,
) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::request(api::PRODUCE, version::PRODUCE, correlation, client_id)?;
    encoder.nullable_string(transactional_id)?;
    encoder.i16(-1);
    encoder.i32(timeout_ms);
    encoder.i32(1);
    encoder.string(topic)?;
    encoder.i32(1);
    encoder.i32(partition);
    encoder.bytes(record_batch)?;
    encoder.finish()
}

pub fn parse_produce(
    frame: &[u8],
    correlation: i32,
    expected_topic: &[u8],
    expected_partition: i32,
) -> Result<ProduceResult, Error> {
    let mut decoder = response(frame, correlation)?;
    let topics = decoder.array_len()?;
    let mut found = None;
    for _ in 0..topics {
        let topic = decoder.string_bytes()?;
        let partitions = decoder.array_len()?;
        for _ in 0..partitions {
            let partition = decoder.i32()?;
            let result = ProduceResult {
                error: decoder.i16()?,
                base_offset: decoder.i64()?,
            };
            let _log_append_time = decoder.i64()?;
            if topic == expected_topic && partition == expected_partition {
                found = Some(result);
            }
        }
    }
    let _throttle = decoder.i32()?;
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    found.ok_or(Error::Invalid)
}

pub struct Fetch<'a> {
    pub topic: &'a [u8],
    pub partition: i32,
    pub offset: i64,
    pub max_wait_ms: i32,
    pub max_bytes: i32,
    pub read_committed: bool,
}

pub fn fetch_request(
    correlation: i32,
    client_id: &[u8],
    fetch: Fetch<'_>,
) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::request(api::FETCH, version::FETCH, correlation, client_id)?;
    encoder.i32(-1);
    encoder.i32(fetch.max_wait_ms);
    encoder.i32(1);
    encoder.i32(fetch.max_bytes);
    encoder.i8(fetch.read_committed as i8);
    encoder.i32(1);
    encoder.string(fetch.topic)?;
    encoder.i32(1);
    encoder.i32(fetch.partition);
    encoder.i64(fetch.offset);
    encoder.i32(fetch.max_bytes);
    encoder.finish()
}

pub fn parse_fetch(
    frame: &[u8],
    correlation: i32,
    expected_topic: &[u8],
    expected_partition: i32,
) -> Result<FetchResult, Error> {
    let mut decoder = response(frame, correlation)?;
    let _throttle = decoder.i32()?;
    let topics = decoder.array_len()?;
    let mut found = None;
    for _ in 0..topics {
        let topic = decoder.string_bytes()?;
        let partitions = decoder.array_len()?;
        for _ in 0..partitions {
            let partition = decoder.i32()?;
            let error = decoder.i16()?;
            let high_watermark = decoder.i64()?;
            let last_stable_offset = decoder.i64()?;
            let aborted_len = decoder.nullable_array_len()?.unwrap_or(0);
            let mut aborted = Vec::with_capacity(aborted_len);
            for _ in 0..aborted_len {
                aborted.push((decoder.i64()?, decoder.i64()?));
            }
            let record_set = decoder.bytes()?.unwrap_or(&[]);
            if topic == expected_topic && partition == expected_partition {
                found = Some(FetchResult {
                    error,
                    high_watermark,
                    last_stable_offset,
                    records: decode_record_set(record_set, &aborted)?,
                });
            }
        }
    }
    if !decoder.done() {
        return Err(Error::Invalid);
    }
    found.ok_or(Error::Invalid)
}

fn put_varint(output: &mut Vec<u8>, value: i32) {
    put_uvarint(output, ((value << 1) ^ (value >> 31)) as u32 as u64);
}

fn put_varlong(output: &mut Vec<u8>, value: i64) {
    put_uvarint(output, ((value << 1) ^ (value >> 63)) as u64);
}

fn put_uvarint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn read_uvarint(decoder: &mut Decoder<'_>, max_bytes: usize) -> Result<u64, Error> {
    let mut value = 0u64;
    for shift in (0..max_bytes * 7).step_by(7) {
        let byte = decoder.take(1)?[0];
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(Error::Invalid)
}

fn read_varint(decoder: &mut Decoder<'_>) -> Result<i32, Error> {
    let raw = read_uvarint(decoder, 5)? as u32;
    Ok(((raw >> 1) as i32) ^ -((raw & 1) as i32))
}

fn read_varlong(decoder: &mut Decoder<'_>) -> Result<i64, Error> {
    let raw = read_uvarint(decoder, 10)?;
    Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
}

pub fn encode_record_batch(
    records: &[RecordInput<'_>],
    producer: ProducerIdentity,
    base_sequence: i32,
    transactional: bool,
) -> Result<Vec<u8>, Error> {
    if records.is_empty() || records.len() > MAX_RECORDS {
        return Err(Error::Invalid);
    }
    let first_timestamp = records[0].timestamp_ms;
    let mut encoded_records = Vec::new();
    for (offset, record) in records.iter().enumerate() {
        let mut body = Vec::new();
        body.push(0);
        put_varlong(&mut body, record.timestamp_ms.saturating_sub(first_timestamp));
        put_varint(&mut body, offset as i32);
        match record.key {
            Some(key) => {
                put_varint(&mut body, key.len().try_into().map_err(|_| Error::TooLarge)?);
                body.extend_from_slice(key);
            }
            None => put_varint(&mut body, -1),
        }
        match record.value {
            Some(value) => {
                put_varint(&mut body, value.len().try_into().map_err(|_| Error::TooLarge)?);
                body.extend_from_slice(value);
            }
            None => put_varint(&mut body, -1),
        }
        put_varint(&mut body, 0);
        put_varint(&mut encoded_records, body.len().try_into().map_err(|_| Error::TooLarge)?);
        encoded_records.extend_from_slice(&body);
    }
    let max_timestamp =
        records.iter().map(|record| record.timestamp_ms).max().unwrap_or(first_timestamp);
    let mut body = Vec::new();
    body.extend_from_slice(&(-1i32).to_be_bytes());
    body.push(2);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(
        &(if transactional {
            0x10i16
        } else {
            0
        })
        .to_be_bytes(),
    );
    body.extend_from_slice(&((records.len() - 1) as i32).to_be_bytes());
    body.extend_from_slice(&first_timestamp.to_be_bytes());
    body.extend_from_slice(&max_timestamp.to_be_bytes());
    body.extend_from_slice(&producer.producer_id.to_be_bytes());
    body.extend_from_slice(&producer.producer_epoch.to_be_bytes());
    body.extend_from_slice(&base_sequence.to_be_bytes());
    body.extend_from_slice(&(records.len() as i32).to_be_bytes());
    body.extend_from_slice(&encoded_records);
    let crc = crc32c(&body[9..]);
    body[5..9].copy_from_slice(&crc.to_be_bytes());
    let mut batch = Vec::new();
    batch.extend_from_slice(&0i64.to_be_bytes());
    batch.extend_from_slice(&(body.len() as i32).to_be_bytes());
    batch.extend_from_slice(&body);
    if batch.len() > MAX_FRAME_LEN {
        return Err(Error::TooLarge);
    }
    Ok(batch)
}

fn decode_record_set(bytes: &[u8], aborted: &[(i64, i64)]) -> Result<Vec<Record>, Error> {
    let mut decoder = Decoder::new(bytes);
    let mut records = Vec::new();
    let mut pending_aborts = aborted
        .iter()
        .map(|(producer_id, first_offset)| (*producer_id, *first_offset, false))
        .collect::<Vec<_>>();
    let mut active_aborts = Vec::new();
    while decoder.offset < bytes.len() {
        if bytes.len() - decoder.offset < 12 {
            break;
        }
        let base_offset = decoder.i64()?;
        let batch_len = decoder.i32()?;
        if batch_len < 49 {
            return Err(Error::Invalid);
        }
        let batch = decoder.take(batch_len as usize)?;
        let mut batch_decoder = Decoder::new(batch);
        let _leader_epoch = batch_decoder.i32()?;
        if batch_decoder.i8()? != 2 {
            return Err(Error::UnsupportedVersion);
        }
        let expected_crc = batch_decoder.i32()? as u32;
        if crc32c(&batch[9..]) != expected_crc {
            return Err(Error::Checksum);
        }
        let attributes = batch_decoder.i16()?;
        if attributes & 0x07 != 0 {
            return Err(Error::UnsupportedVersion);
        }
        let _last_offset_delta = batch_decoder.i32()?;
        let first_timestamp = batch_decoder.i64()?;
        let _max_timestamp = batch_decoder.i64()?;
        let producer_id = batch_decoder.i64()?;
        let _producer_epoch = batch_decoder.i16()?;
        let _base_sequence = batch_decoder.i32()?;
        let count = batch_decoder.i32()?;
        if count < 0 || count as usize > MAX_RECORDS {
            return Err(Error::TooLarge);
        }
        let is_control = attributes & 0x20 != 0;
        for (id, first_offset, activated) in &mut pending_aborts {
            if !*activated && base_offset >= *first_offset {
                active_aborts.push(*id);
                *activated = true;
            }
        }
        let is_aborted = attributes & 0x10 != 0 && active_aborts.contains(&producer_id);
        for _ in 0..count {
            let len = read_varint(&mut batch_decoder)?;
            if len < 0 {
                return Err(Error::Invalid);
            }
            let body = batch_decoder.take(len as usize)?;
            let mut record = Decoder::new(body);
            let _attributes = record.i8()?;
            let timestamp_delta = read_varlong(&mut record)?;
            let offset_delta = read_varint(&mut record)?;
            let key_len = read_varint(&mut record)?;
            let key = if key_len == -1 {
                None
            } else if key_len >= 0 {
                Some(record.take(key_len as usize)?.to_vec())
            } else {
                return Err(Error::Invalid);
            };
            let value_len = read_varint(&mut record)?;
            let value = if value_len == -1 {
                None
            } else if value_len >= 0 {
                Some(record.take(value_len as usize)?.to_vec())
            } else {
                return Err(Error::Invalid);
            };
            let headers = read_varint(&mut record)?;
            if headers < 0 || headers as usize > MAX_ARRAY_LEN {
                return Err(Error::Invalid);
            }
            for _ in 0..headers {
                let key_len = read_varint(&mut record)?;
                if key_len < 0 {
                    return Err(Error::Invalid);
                }
                let _ = record.take(key_len as usize)?;
                let value_len = read_varint(&mut record)?;
                if value_len >= 0 {
                    let _ = record.take(value_len as usize)?;
                } else if value_len != -1 {
                    return Err(Error::Invalid);
                }
            }
            if !record.done() {
                return Err(Error::Invalid);
            }
            if !is_control && !is_aborted {
                records.push(Record {
                    offset: base_offset.saturating_add(i64::from(offset_delta)),
                    timestamp: first_timestamp.saturating_add(timestamp_delta),
                    key,
                    value,
                });
            }
        }
        if !batch_decoder.done() {
            return Err(Error::Invalid);
        }
        if is_control && let Some(index) = active_aborts.iter().position(|id| *id == producer_id) {
            active_aborts.remove(index);
        }
    }
    Ok(records)
}

pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x82f6_3b78
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_known_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn record_batch_round_trip() {
        let producer = ProducerIdentity {
            producer_id: 42,
            producer_epoch: 3,
        };
        let batch = encode_record_batch(
            &[
                RecordInput {
                    timestamp_ms: 1_000,
                    key: Some(b"a"),
                    value: Some(b"first"),
                },
                RecordInput {
                    timestamp_ms: 1_004,
                    key: None,
                    value: Some(b"second"),
                },
            ],
            producer,
            7,
            true,
        )
        .unwrap();
        let records = decode_record_set(&batch, &[]).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].offset, 0);
        assert_eq!(records[0].key.as_deref(), Some(b"a".as_slice()));
        assert_eq!(records[1].timestamp, 1_004);
        assert_eq!(records[1].value.as_deref(), Some(b"second".as_slice()));
    }

    #[test]
    fn aborted_transaction_is_filtered() {
        let producer = ProducerIdentity {
            producer_id: 7,
            producer_epoch: 0,
        };
        let batch = encode_record_batch(
            &[RecordInput {
                timestamp_ms: 1,
                key: None,
                value: Some(b"hidden"),
            }],
            producer,
            0,
            true,
        )
        .unwrap();
        assert!(decode_record_set(&batch, &[(7, 0)]).unwrap().is_empty());
    }

    #[test]
    fn request_length_and_header_are_big_endian() {
        let request = metadata_request(0x0102_0304, b"charlotte", b"events").unwrap();
        assert_eq!(
            i32::from_be_bytes(request[..4].try_into().unwrap()) as usize,
            request.len() - 4
        );
        assert_eq!(&request[4..6], &api::METADATA.to_be_bytes());
        assert_eq!(&request[8..12], &0x0102_0304i32.to_be_bytes());
    }

    #[test]
    fn metadata_request_batches_topics() {
        let topics: &[&[u8]] = &[b"events", b"results"];
        let request = metadata_request_many(7, b"charlotte", topics).unwrap();
        assert!(request.ends_with(&[
            0, 0, 0, 2, 0, 6, b'e', b'v', b'e', b'n', b't', b's', 0, 7, b'r', b'e', b's', b'u',
            b'l', b't', b's',
        ]));
    }
}
