//! Capability-oriented Kafka producer, fixed-partition consumer, and
//! transactional consume-transform-produce service.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::String,
    vec::Vec,
};

use catten_rt::{
    Context,
    ManifestValue,
    config,
    owned::{
        Connection,
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        ReplyToken,
    },
};
use catten_services::{
    entropy,
    kafka as protocol,
    ns,
    sleep_ms,
    socket,
    time,
    tls_client,
    try_registered_name_owned,
    wait_for_local_ready_owned,
    wait_for_registered_name_owned,
};
use catten_syscall::{
    IpcRights,
    thread_exit,
};
use charlotte_kafka::{
    self as wire,
    ProducerIdentity,
    RecordInput,
};
use charlotte_protocol_kafka::{
    DeliveredRecord,
    RecordRequest,
};

catten_rt::entry!(main);

const CLIENT_ID: &[u8] = b"charlotte-os";
const SEND_ATTEMPTS: usize = 4_096;
const SEND_RETRY_MS: u64 = 10;
const RECEIVE_ATTEMPTS: usize = 3_000;
const RECEIVE_RETRY_MS: u64 = 10;
const PRODUCE_TIMEOUT_MS: i32 = 30_000;
const FETCH_WAIT_MS: i32 = 250;
const FETCH_MAX_BYTES: i32 = 64 * 1024;
const MAX_CONSUMERS: usize = 8;
const MAX_DELIVERIES: usize = 8;
const COORDINATOR_ATTEMPTS: usize = 120;
const COORDINATOR_RETRY_MS: u64 = 250;
const TLS_TIME_ATTEMPTS: usize = 120;
const TLS_TIME_RETRY_MS: u64 = 250;

mod status {
    pub const STAGE: usize = 0;
    pub const REQUESTS: usize = 4;
    pub const PRODUCED: usize = 8;
    pub const CONSUMED: usize = 12;
    pub const COMMITS: usize = 16;
    pub const ABORTS: usize = 20;
    pub const BACKPRESSURE: usize = 24;
    pub const ERROR: usize = 28;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClientIdentity {
    domain: u64,
    generation: u64,
}

struct Profile {
    ip: [u8; 4],
    host: String,
    port: u16,
    tls: bool,
    ca_der: Vec<u8>,
    topic: Vec<u8>,
    partition: i32,
    group: Vec<u8>,
    transactional_id: Vec<u8>,
    rights: u64,
    transaction_timeout_ms: i32,
}

impl Profile {
    fn from_context(ctx: &Context) -> Option<Self> {
        let ip = match ctx.manifest_value(protocol::manifest::IP)? {
            ManifestValue::Bytes(bytes) if bytes.len() == 4 => {
                [bytes[0], bytes[1], bytes[2], bytes[3]]
            }
            _ => return None,
        };
        let host = manifest_text(ctx, protocol::manifest::HOST)?;
        let port = manifest_unsigned(ctx, protocol::manifest::PORT)
            .and_then(|value| u16::try_from(value).ok())?;
        let tls_value = manifest_unsigned(ctx, protocol::manifest::TLS).unwrap_or(0);
        let tls = tls_value == 1;
        let ca_der = manifest_bytes(ctx, protocol::manifest::CA_DER).unwrap_or_default();
        let topic = manifest_bytes(ctx, protocol::manifest::TOPIC)?;
        let partition = manifest_unsigned(ctx, protocol::manifest::PARTITION)
            .and_then(|value| i32::try_from(value).ok())?;
        let group = manifest_bytes(ctx, protocol::manifest::GROUP)?;
        let transactional_id = manifest_bytes(ctx, protocol::manifest::TRANSACTIONAL_ID)?;
        let rights = manifest_unsigned(ctx, protocol::manifest::RIGHTS)?;
        let transaction_timeout_ms =
            manifest_unsigned(ctx, protocol::manifest::TRANSACTION_TIMEOUT_MS)
                .unwrap_or(60_000)
                .try_into()
                .ok()?;
        if port == 0
            || tls_value > 1
            || (tls && ca_der.is_empty())
            || host.is_empty()
            || host.len() > 255
            || topic.is_empty()
            || topic.len() > 249
            || group.is_empty()
            || transactional_id.is_empty()
            || rights == 0
            || rights & !protocol::ALL_RIGHTS != 0
            || !(1_000..=900_000).contains(&transaction_timeout_ms)
        {
            return None;
        }
        Some(Self {
            ip,
            host,
            port,
            tls,
            ca_der,
            topic,
            partition,
            group,
            transactional_id,
            rights,
            transaction_timeout_ms,
        })
    }

    fn has(&self, right: u64) -> bool {
        self.rights & right != 0
    }
}

fn manifest_bytes(ctx: &Context, key: u64) -> Option<Vec<u8>> {
    match ctx.manifest_value(key)? {
        ManifestValue::Bytes(bytes) => Some(bytes.to_vec()),
        _ => None,
    }
}

fn manifest_text(ctx: &Context, key: u64) -> Option<String> {
    String::from_utf8(manifest_bytes(ctx, key)?).ok()
}

fn manifest_unsigned(ctx: &Context, key: u64) -> Option<u64> {
    match ctx.manifest_value(key)? {
        ManifestValue::Unsigned(value) => Some(value),
        _ => None,
    }
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn time_snapshot(connection: ConnectionRef<'_>) -> Option<time::TimeSnapshot> {
    let result = connection.call(time::OP_NOW, 0).ok()?.wait().ok()?;
    if result.result != time::SNAPSHOT_LEN as i64 {
        return None;
    }
    let mapping = result.memory?.map_read_only().ok()?;
    time::TimeSnapshot::decode(mapping.as_slice())
}

fn unix_millis(connection: ConnectionRef<'_>) -> Option<i64> {
    let snapshot = time_snapshot(connection)?;
    if snapshot.unix_seconds <= 0 {
        return None;
    }
    snapshot
        .unix_seconds
        .checked_mul(1_000)?
        .checked_add(i64::from(snapshot.nanosecond / 1_000_000))
}

fn tls_unix_seconds(connection: ConnectionRef<'_>) -> Option<u64> {
    let snapshot = time_snapshot(connection)?;
    (snapshot.state == time::STATE_SYNCHRONIZED && snapshot.unix_seconds > 0)
        .then_some(snapshot.unix_seconds as u64)
}

fn tcpip_has_ipv4(connection: ConnectionRef<'_>) -> bool {
    let Ok(call) = connection.call(socket::OP_STATUS, 0) else {
        return false;
    };
    let Ok(result) = call.wait() else {
        return false;
    };
    if result.result < core::mem::size_of::<u32>() as i64 {
        return false;
    }
    result
        .memory
        .and_then(|memory| memory.map_read_only().ok())
        .filter(|mapping| mapping.len() >= core::mem::size_of::<u32>())
        .is_some_and(|mapping| {
            (unsafe { core::ptr::read_unaligned(mapping.as_ptr().cast::<u32>()) }) != 0
        })
}

struct BrokerTransport<'connection> {
    tcp: ConnectionRef<'connection>,
    entropy: Option<ConnectionRef<'connection>>,
    clock: ConnectionRef<'connection>,
    ip: [u8; 4],
    port: u16,
    tls: bool,
    host: String,
    ca_der: Vec<u8>,
    stream: Option<BrokerStream<'connection>>,
    received: Vec<u8>,
}

enum BrokerStream<'connection> {
    Plain(socket::OwnedSocket<'connection>),
    Tls(Box<tls_client::OwnedTlsStream<'connection>>),
}

impl BrokerStream<'_> {
    fn send_all(&mut self, bytes: &[u8]) -> Result<(), ()> {
        match self {
            Self::Plain(socket) => {
                socket.send_all(bytes, SEND_ATTEMPTS, SEND_RETRY_MS).map_err(|_| ())
            }
            Self::Tls(stream) => stream.send_all(bytes).map_err(|_| ()),
        }
    }

    fn receive(&mut self) -> Result<Vec<u8>, i64> {
        match self {
            Self::Plain(socket) => {
                let chunk = socket
                    .receive_timeout(RECEIVE_ATTEMPTS, RECEIVE_RETRY_MS)
                    .map_err(|_| protocol::ERR_TIMEOUT)?
                    .ok_or(protocol::ERR_TRANSPORT)?;
                let (memory, len) = chunk.into_parts();
                let mapping = memory.map_read_only().map_err(|_| protocol::ERR_TRANSPORT)?;
                Ok(mapping.as_slice()[..len].to_vec())
            }
            Self::Tls(stream) => stream.receive().map_err(|_| protocol::ERR_TRANSPORT),
        }
    }
}

impl<'connection> BrokerTransport<'connection> {
    fn new(
        tcp: ConnectionRef<'connection>,
        entropy: Option<ConnectionRef<'connection>>,
        clock: ConnectionRef<'connection>,
        profile: &Profile,
    ) -> Self {
        Self {
            tcp,
            entropy,
            clock,
            ip: profile.ip,
            port: profile.port,
            tls: profile.tls,
            host: profile.host.clone(),
            ca_der: profile.ca_der.clone(),
            stream: None,
            received: Vec::new(),
        }
    }

    fn connect(&mut self) -> Result<(), i64> {
        if self.stream.is_some() {
            return Ok(());
        }
        let socket = socket::OwnedSocket::open(self.tcp, socket::DOMAIN_TCP)
            .map_err(|_| protocol::ERR_TRANSPORT)?;
        socket.connect_ipv4(self.ip, self.port).map_err(|_| protocol::ERR_TRANSPORT)?;
        let stream = if self.tls {
            let unix_seconds = tls_unix_seconds(self.clock).ok_or(protocol::ERR_TLS_REQUIRED)?;
            let stream = tls_client::OwnedTlsStream::open(
                socket,
                self.entropy,
                tls_client::OpenConfig {
                    server_name: &self.host,
                    ca_certificate_der: &self.ca_der,
                    unix_seconds,
                    socket_bounds: tls_client::SocketBounds {
                        send_attempts: SEND_ATTEMPTS,
                        send_retry_ms: SEND_RETRY_MS,
                        receive_attempts: RECEIVE_ATTEMPTS,
                        receive_retry_ms: RECEIVE_RETRY_MS,
                        receive_chunk_len: 16 * 1024,
                    },
                },
            )
            .map_err(|error| {
                match error {
                    tls_client::OpenError::Handshake(code) => {
                        catten_rt::logln!("[kafka] TLS handshake verification failed ({})", code)
                    }
                    tls_client::OpenError::EntropyUnavailable => {
                        catten_rt::logln!("[kafka] TLS unavailable: system entropy source failed")
                    }
                    tls_client::OpenError::InvalidConfiguration => {
                        catten_rt::logln!("[kafka] invalid TLS transport profile")
                    }
                }
                protocol::ERR_TLS_REQUIRED
            })?;
            BrokerStream::Tls(Box::new(stream))
        } else {
            BrokerStream::Plain(socket)
        };
        self.stream = Some(stream);
        self.received.clear();
        Ok(())
    }

    fn request(&mut self, request: &[u8]) -> Result<Vec<u8>, i64> {
        self.connect()?;
        let stream = self.stream.as_mut().ok_or(protocol::ERR_TRANSPORT)?;
        if stream.send_all(request).is_err() {
            self.stream.take();
            return Err(protocol::ERR_TRANSPORT);
        }
        let result = self.read_frame();
        if result.is_err() {
            // A late reply must not be mistaken for the response to a later
            // correlation ID. Reconnect after every incomplete exchange.
            self.stream.take();
            self.received.clear();
        }
        result
    }

    fn read_frame(&mut self) -> Result<Vec<u8>, i64> {
        loop {
            if self.received.len() >= 4 {
                let payload = i32::from_be_bytes(
                    self.received[..4].try_into().map_err(|_| protocol::ERR_PROTOCOL)?,
                );
                if payload < 4 || payload as usize > wire::MAX_FRAME_LEN {
                    self.stream.take();
                    return Err(protocol::ERR_PROTOCOL);
                }
                let frame_len = payload as usize + 4;
                if self.received.len() >= frame_len {
                    let remainder = self.received.split_off(frame_len);
                    let frame = core::mem::replace(&mut self.received, remainder);
                    return Ok(frame);
                }
            }
            let stream = self.stream.as_mut().ok_or(protocol::ERR_TRANSPORT)?;
            self.received.extend_from_slice(&stream.receive()?);
            if self.received.len() > wire::MAX_FRAME_LEN + 4 {
                self.stream.take();
                return Err(protocol::ERR_TOO_LARGE);
            }
        }
    }
}

struct BrokerSession<'connection> {
    transport: BrokerTransport<'connection>,
    correlation: i32,
}

impl<'connection> BrokerSession<'connection> {
    fn new(
        tcp: ConnectionRef<'connection>,
        entropy: Option<ConnectionRef<'connection>>,
        clock: ConnectionRef<'connection>,
        profile: &Profile,
    ) -> Self {
        Self {
            transport: BrokerTransport::new(tcp, entropy, clock, profile),
            correlation: 1,
        }
    }

    fn next(&mut self) -> i32 {
        let value = self.correlation;
        self.correlation = self.correlation.wrapping_add(1).max(1);
        value
    }

    fn exchange(&mut self, request: Vec<u8>) -> Result<Vec<u8>, i64> {
        self.transport.request(&request)
    }

    fn bootstrap(
        &mut self,
        profile: &Profile,
    ) -> Result<(ProducerIdentity, ProducerIdentity), i64> {
        let correlation = self.next();
        let request = wire::api_versions_request(correlation, CLIENT_ID).map_err(map_wire)?;
        let response = self.exchange(request)?;
        let versions = wire::parse_api_versions(&response, correlation).map_err(map_wire)?;
        for (api, version) in [
            (wire::api::PRODUCE, wire::version::PRODUCE),
            (wire::api::FETCH, wire::version::FETCH),
            (wire::api::LIST_OFFSETS, wire::version::LIST_OFFSETS),
            (wire::api::METADATA, wire::version::METADATA),
            (wire::api::OFFSET_COMMIT, wire::version::OFFSET_COMMIT),
            (wire::api::OFFSET_FETCH, wire::version::OFFSET_FETCH),
            (wire::api::FIND_COORDINATOR, wire::version::FIND_COORDINATOR),
            (wire::api::INIT_PRODUCER_ID, wire::version::INIT_PRODUCER_ID),
            (wire::api::ADD_PARTITIONS_TO_TXN, wire::version::ADD_PARTITIONS_TO_TXN),
            (wire::api::ADD_OFFSETS_TO_TXN, wire::version::ADD_OFFSETS_TO_TXN),
            (wire::api::END_TXN, wire::version::END_TXN),
            (wire::api::TXN_OFFSET_COMMIT, wire::version::TXN_OFFSET_COMMIT),
        ] {
            if !versions.supports(api, version) {
                return Err(protocol::ERR_UNSUPPORTED);
            }
        }

        let correlation = self.next();
        let request =
            wire::metadata_request(correlation, CLIENT_ID, &profile.topic).map_err(map_wire)?;
        let response = self.exchange(request)?;
        let metadata =
            wire::parse_metadata(&response, correlation, &profile.topic).map_err(map_wire)?;
        if metadata.topic_error != wire::NO_ERROR {
            return Err(protocol::ERR_BROKER);
        }
        let leader = metadata
            .partitions
            .iter()
            .find(|partition| partition.partition == profile.partition)
            .ok_or(protocol::ERR_UNSUPPORTED)?;
        if leader.error != wire::NO_ERROR
            || metadata.brokers.len() != 1
            || metadata.brokers[0].node_id != leader.leader
        {
            return Err(protocol::ERR_UNSUPPORTED);
        }

        for (key, transaction) in
            [(profile.group.as_slice(), false), (profile.transactional_id.as_slice(), true)]
        {
            let mut coordinator = None;
            for _ in 0..COORDINATOR_ATTEMPTS {
                let correlation = self.next();
                let request =
                    wire::find_coordinator_request(correlation, CLIENT_ID, key, transaction)
                        .map_err(map_wire)?;
                let response = self.exchange(request)?;
                let response =
                    wire::parse_find_coordinator(&response, correlation).map_err(map_wire)?;
                if response.error == wire::NO_ERROR {
                    coordinator = Some(response);
                    break;
                }
                if !wire::is_retriable_broker_error(response.error) {
                    return Err(map_broker(response.error));
                }
                sleep_ms(COORDINATOR_RETRY_MS);
            }
            let coordinator = coordinator.ok_or(protocol::ERR_TIMEOUT)?;
            if coordinator.node_id != metadata.brokers[0].node_id {
                return Err(protocol::ERR_UNSUPPORTED);
            }
        }

        let non_transactional = self.init_producer(None, profile.transaction_timeout_ms)?;
        let transactional =
            self.init_producer(Some(&profile.transactional_id), profile.transaction_timeout_ms)?;
        Ok((non_transactional, transactional))
    }

    fn init_producer(
        &mut self,
        transactional_id: Option<&[u8]>,
        timeout_ms: i32,
    ) -> Result<ProducerIdentity, i64> {
        for _ in 0..COORDINATOR_ATTEMPTS {
            let correlation = self.next();
            let request = wire::init_producer_id_request(
                correlation,
                CLIENT_ID,
                transactional_id,
                timeout_ms,
            )
            .map_err(map_wire)?;
            let response = self.exchange(request)?;
            match wire::parse_init_producer_id(&response, correlation) {
                Ok(identity) => return Ok(identity),
                Err(wire::Error::Broker(error)) if wire::is_retriable_broker_error(error) => {
                    sleep_ms(COORDINATOR_RETRY_MS);
                }
                Err(error) => return Err(map_wire(error)),
            }
        }
        Err(protocol::ERR_TIMEOUT)
    }

    fn produce(
        &mut self,
        profile: &Profile,
        producer: ProducerIdentity,
        sequence: i32,
        transactional: bool,
        record: &OwnedRecord,
    ) -> Result<i64, i64> {
        let batch = wire::encode_record_batch(
            &[RecordInput {
                timestamp_ms: record.timestamp_ms,
                key: record.key.as_deref(),
                value: record.value.as_deref(),
            }],
            producer,
            sequence,
            transactional,
        )
        .map_err(map_wire)?;
        let correlation = self.next();
        let request = wire::produce_request(
            correlation,
            CLIENT_ID,
            transactional.then_some(profile.transactional_id.as_slice()),
            &profile.topic,
            profile.partition,
            &batch,
            PRODUCE_TIMEOUT_MS,
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        let result = wire::parse_produce(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)?;
        if result.error == wire::NO_ERROR {
            Ok(result.base_offset)
        } else {
            Err(map_broker(result.error))
        }
    }

    fn add_partition(&mut self, profile: &Profile, producer: ProducerIdentity) -> Result<(), i64> {
        for _ in 0..COORDINATOR_ATTEMPTS {
            let correlation = self.next();
            let request = wire::add_partitions_to_txn_request(
                correlation,
                CLIENT_ID,
                &profile.transactional_id,
                producer,
                &profile.topic,
                profile.partition,
            )
            .map_err(map_wire)?;
            let response = self.exchange(request)?;
            match wire::parse_partition_error(
                &response,
                correlation,
                &profile.topic,
                profile.partition,
            ) {
                Ok(()) => return Ok(()),
                Err(wire::Error::Broker(error)) if wire::is_retriable_broker_error(error) => {
                    sleep_ms(COORDINATOR_RETRY_MS);
                }
                Err(error) => return Err(map_wire(error)),
            }
        }
        Err(protocol::ERR_TIMEOUT)
    }

    fn committed_offset(&mut self, profile: &Profile) -> Result<Option<i64>, i64> {
        let correlation = self.next();
        let request = wire::offset_fetch_request(
            correlation,
            CLIENT_ID,
            &profile.group,
            &profile.topic,
            profile.partition,
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        wire::parse_offset_fetch(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)
    }

    fn earliest_offset(&mut self, profile: &Profile) -> Result<i64, i64> {
        let correlation = self.next();
        let request = wire::list_offsets_request(
            correlation,
            CLIENT_ID,
            &profile.topic,
            profile.partition,
            true,
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        wire::parse_list_offsets(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)
    }

    fn fetch(&mut self, profile: &Profile, offset: i64) -> Result<Vec<wire::Record>, i64> {
        let correlation = self.next();
        let request = wire::fetch_request(
            correlation,
            CLIENT_ID,
            wire::Fetch {
                topic: &profile.topic,
                partition: profile.partition,
                offset,
                max_wait_ms: FETCH_WAIT_MS,
                max_bytes: FETCH_MAX_BYTES,
                read_committed: true,
            },
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        let result = wire::parse_fetch(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)?;
        if result.error == wire::NO_ERROR {
            Ok(result.records)
        } else {
            Err(map_broker(result.error))
        }
    }

    fn commit_offset(&mut self, profile: &Profile, next_offset: i64) -> Result<(), i64> {
        let correlation = self.next();
        let request = wire::offset_commit_request(
            correlation,
            CLIENT_ID,
            &profile.group,
            &profile.topic,
            profile.partition,
            next_offset,
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        wire::parse_offset_commit(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)
    }

    fn add_transactional_offset(
        &mut self,
        profile: &Profile,
        producer: ProducerIdentity,
        next_offset: i64,
    ) -> Result<(), i64> {
        let correlation = self.next();
        let request = wire::add_offsets_to_txn_request(
            correlation,
            CLIENT_ID,
            &profile.transactional_id,
            producer,
            &profile.group,
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        wire::parse_top_level_error(&response, correlation).map_err(map_wire)?;

        let correlation = self.next();
        let request = wire::txn_offset_commit_request(
            correlation,
            CLIENT_ID,
            wire::TxnOffsetCommit {
                transactional_id: &profile.transactional_id,
                group_id: &profile.group,
                producer,
                topic: &profile.topic,
                partition: profile.partition,
                next_offset,
            },
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        wire::parse_partition_error(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)
    }

    fn end_transaction(
        &mut self,
        profile: &Profile,
        producer: ProducerIdentity,
        commit: bool,
    ) -> Result<(), i64> {
        let correlation = self.next();
        let request = wire::end_txn_request(
            correlation,
            CLIENT_ID,
            &profile.transactional_id,
            producer,
            commit,
        )
        .map_err(map_wire)?;
        let response = self.exchange(request)?;
        wire::parse_top_level_error(&response, correlation).map_err(map_wire)
    }
}

fn map_wire(error: wire::Error) -> i64 {
    match error {
        wire::Error::TooLarge => protocol::ERR_TOO_LARGE,
        wire::Error::UnsupportedVersion => protocol::ERR_UNSUPPORTED,
        wire::Error::Broker(error) => map_broker(error),
        wire::Error::Incomplete
        | wire::Error::Invalid
        | wire::Error::Correlation
        | wire::Error::Checksum => protocol::ERR_PROTOCOL,
    }
}

fn map_broker(error: i16) -> i64 {
    match error {
        wire::PRODUCER_FENCED
        | wire::INVALID_PRODUCER_EPOCH
        | wire::TRANSACTION_COORDINATOR_FENCED => protocol::ERR_FENCED,
        wire::REQUEST_TIMED_OUT => protocol::ERR_TIMEOUT,
        _ => protocol::ERR_BROKER,
    }
}

struct OwnedRecord {
    timestamp_ms: i64,
    key: Option<Vec<u8>>,
    value: Option<Vec<u8>>,
}

fn decode_record(
    memory: Option<OwnedMemory>,
    exact_len: usize,
    clock: ConnectionRef<'_>,
) -> Result<OwnedRecord, i64> {
    let memory = memory.ok_or(protocol::ERR_INVALID)?;
    let mapping = memory.map_read_only().map_err(|_| protocol::ERR_INVALID)?;
    if exact_len > mapping.len() {
        return Err(protocol::ERR_INVALID);
    }
    let record =
        RecordRequest::decode(&mapping.as_slice()[..exact_len]).ok_or(protocol::ERR_INVALID)?;
    let timestamp_ms = if record.timestamp_ms < 0 {
        unix_millis(clock).ok_or(protocol::ERR_TIMEOUT)?
    } else {
        record.timestamp_ms
    };
    Ok(OwnedRecord {
        timestamp_ms,
        key: record.key.map(<[u8]>::to_vec),
        value: record.value.map(<[u8]>::to_vec),
    })
}

struct ConsumerState {
    owner: ClientIdentity,
    fetch_offset: i64,
    committed_offset: i64,
    outstanding: Option<u32>,
    reserved_transaction: Option<u32>,
}

struct DeliveryState {
    owner: ClientIdentity,
    consumer_id: u32,
    offset: i64,
    next_offset: i64,
}

struct IncludedDelivery {
    consumer_id: u32,
    offset: i64,
    next_offset: i64,
}

struct TransactionState {
    owner: ClientIdentity,
    partition_added: bool,
    touched: bool,
    reset_producer: bool,
    included: Option<IncludedDelivery>,
}

struct Service<'connection> {
    profile: Profile,
    clock: ConnectionRef<'connection>,
    broker: BrokerSession<'connection>,
    non_transactional_producer: ProducerIdentity,
    transactional_producer: ProducerIdentity,
    non_transactional_sequence: i32,
    transactional_sequence: i32,
    consumers: BTreeMap<u32, ConsumerState>,
    deliveries: BTreeMap<u32, DeliveryState>,
    transaction: Option<(u32, TransactionState)>,
    next_id: u32,
    requests: u32,
    produced: u32,
    consumed: u32,
    commits: u32,
    aborts: u32,
    backpressure: u32,
}

impl Service<'_> {
    fn id(&mut self) -> Result<u32, i64> {
        for _ in 0..u32::MAX {
            let id = self.next_id.max(1);
            self.next_id = id.wrapping_add(1).max(1);
            let tx_used = self.transaction.as_ref().is_some_and(|(tx, _)| *tx == id);
            if !self.consumers.contains_key(&id) && !self.deliveries.contains_key(&id) && !tx_used {
                return Ok(id);
            }
        }
        Err(protocol::ERR_BUSY)
    }

    fn produce(&mut self, record: OwnedRecord) -> Result<i64, i64> {
        if !self.profile.has(protocol::RIGHT_PRODUCE) {
            return Err(protocol::ERR_DENIED);
        }
        let offset = self.broker.produce(
            &self.profile,
            self.non_transactional_producer,
            self.non_transactional_sequence,
            false,
            &record,
        )?;
        self.non_transactional_sequence =
            self.non_transactional_sequence.checked_add(1).ok_or(protocol::ERR_FENCED)?;
        self.produced = self.produced.wrapping_add(1);
        Ok(offset)
    }

    fn open_consumer(&mut self, owner: ClientIdentity) -> Result<u32, i64> {
        if !self.profile.has(protocol::RIGHT_CONSUME) {
            return Err(protocol::ERR_DENIED);
        }
        if self.consumers.len() >= MAX_CONSUMERS {
            return Err(protocol::ERR_BUSY);
        }
        let committed = self
            .broker
            .committed_offset(&self.profile)?
            .unwrap_or(self.broker.earliest_offset(&self.profile)?);
        let id = self.id()?;
        self.consumers.insert(
            id,
            ConsumerState {
                owner,
                fetch_offset: committed,
                committed_offset: committed,
                outstanding: None,
                reserved_transaction: None,
            },
        );
        Ok(id)
    }

    fn poll_consumer(
        &mut self,
        owner: ClientIdentity,
        consumer_id: u32,
    ) -> Result<Option<(u32, wire::Record)>, i64> {
        let consumer = self
            .consumers
            .get(&consumer_id)
            .filter(|consumer| consumer.owner == owner)
            .ok_or(protocol::ERR_INVALID)?;
        if consumer.outstanding.is_some() || consumer.reserved_transaction.is_some() {
            self.backpressure = self.backpressure.wrapping_add(1);
            return Err(protocol::ERR_BUSY);
        }
        if self.deliveries.len() >= MAX_DELIVERIES {
            self.backpressure = self.backpressure.wrapping_add(1);
            return Err(protocol::ERR_BUSY);
        }
        let offset = consumer.fetch_offset;
        let record = self
            .broker
            .fetch(&self.profile, offset)?
            .into_iter()
            .find(|record| record.offset >= offset);
        let Some(record) = record else {
            return Ok(None);
        };
        if (DeliveredRecord {
            partition: self.profile.partition,
            offset: record.offset,
            timestamp_ms: record.timestamp,
            key: record.key.as_deref(),
            value: record.value.as_deref(),
        })
        .encoded_len()
            > protocol::MAX_RECORD_BYTES
        {
            return Err(protocol::ERR_TOO_LARGE);
        }
        let delivery_id = self.id()?;
        let next_offset = record.offset.checked_add(1).ok_or(protocol::ERR_PROTOCOL)?;
        self.deliveries.insert(
            delivery_id,
            DeliveryState {
                owner,
                consumer_id,
                offset: record.offset,
                next_offset,
            },
        );
        let consumer = self.consumers.get_mut(&consumer_id).ok_or(protocol::ERR_INVALID)?;
        consumer.fetch_offset = next_offset;
        consumer.outstanding = Some(delivery_id);
        self.consumed = self.consumed.wrapping_add(1);
        Ok(Some((delivery_id, record)))
    }

    fn release_delivery(&mut self, owner: ClientIdentity, delivery_id: u32) -> Result<(), i64> {
        let delivery = self
            .deliveries
            .get(&delivery_id)
            .filter(|delivery| delivery.owner == owner)
            .ok_or(protocol::ERR_INVALID)?;
        let consumer_id = delivery.consumer_id;
        let offset = delivery.offset;
        self.deliveries.remove(&delivery_id);
        let consumer = self.consumers.get_mut(&consumer_id).ok_or(protocol::ERR_INVALID)?;
        consumer.outstanding = None;
        consumer.fetch_offset = offset;
        Ok(())
    }

    fn commit_delivery(&mut self, owner: ClientIdentity, delivery_id: u32) -> Result<(), i64> {
        let delivery = self
            .deliveries
            .get(&delivery_id)
            .filter(|delivery| delivery.owner == owner)
            .ok_or(protocol::ERR_INVALID)?;
        self.broker.commit_offset(&self.profile, delivery.next_offset)?;
        let consumer_id = delivery.consumer_id;
        let next_offset = delivery.next_offset;
        self.deliveries.remove(&delivery_id);
        let consumer = self.consumers.get_mut(&consumer_id).ok_or(protocol::ERR_INVALID)?;
        consumer.outstanding = None;
        consumer.committed_offset = next_offset;
        self.commits = self.commits.wrapping_add(1);
        Ok(())
    }

    fn close_consumer(&mut self, owner: ClientIdentity, consumer_id: u32) -> Result<(), i64> {
        let consumer = self
            .consumers
            .get(&consumer_id)
            .filter(|consumer| consumer.owner == owner)
            .ok_or(protocol::ERR_INVALID)?;
        if consumer.reserved_transaction.is_some() {
            return Err(protocol::ERR_BUSY);
        }
        if let Some(delivery) = consumer.outstanding {
            self.deliveries.remove(&delivery);
        }
        self.consumers.remove(&consumer_id);
        Ok(())
    }

    fn begin_transaction(&mut self, owner: ClientIdentity) -> Result<u32, i64> {
        if !self.profile.has(protocol::RIGHT_TRANSACTION) {
            return Err(protocol::ERR_DENIED);
        }
        if self.transaction.is_some() {
            return Err(protocol::ERR_BUSY);
        }
        let id = self.id()?;
        self.transaction = Some((
            id,
            TransactionState {
                owner,
                partition_added: false,
                touched: false,
                reset_producer: false,
                included: None,
            },
        ));
        Ok(id)
    }

    fn transaction_produce(
        &mut self,
        owner: ClientIdentity,
        transaction_id: u32,
        record: OwnedRecord,
    ) -> Result<i64, i64> {
        if !self.profile.has(protocol::RIGHT_PRODUCE) {
            return Err(protocol::ERR_DENIED);
        }
        let transaction = self
            .transaction
            .as_ref()
            .filter(|(id, transaction)| *id == transaction_id && transaction.owner == owner)
            .ok_or(protocol::ERR_INVALID)?;
        if !transaction.1.partition_added {
            if let Err(error) =
                self.broker.add_partition(&self.profile, self.transactional_producer)
            {
                let transaction = &mut self.transaction.as_mut().ok_or(protocol::ERR_INVALID)?.1;
                // AddPartitionsToTxn may have reached the coordinator even
                // when its reply did not. Force a fencing reinitialization
                // after the Drop-driven abort path.
                transaction.touched = true;
                transaction.reset_producer = true;
                return Err(error);
            }
            self.transaction.as_mut().ok_or(protocol::ERR_INVALID)?.1.partition_added = true;
        }
        let result = self.broker.produce(
            &self.profile,
            self.transactional_producer,
            self.transactional_sequence,
            true,
            &record,
        );
        let offset = match result {
            Ok(offset) => offset,
            Err(error) => {
                let transaction = &mut self.transaction.as_mut().ok_or(protocol::ERR_INVALID)?.1;
                transaction.touched = true;
                transaction.reset_producer = true;
                return Err(error);
            }
        };
        self.transactional_sequence =
            self.transactional_sequence.checked_add(1).ok_or(protocol::ERR_FENCED)?;
        self.transaction.as_mut().ok_or(protocol::ERR_INVALID)?.1.touched = true;
        self.produced = self.produced.wrapping_add(1);
        Ok(offset)
    }

    fn include_delivery(
        &mut self,
        owner: ClientIdentity,
        transaction_id: u32,
        delivery_id: u32,
    ) -> Result<(), i64> {
        let transaction = self
            .transaction
            .as_ref()
            .filter(|(id, transaction)| *id == transaction_id && transaction.owner == owner)
            .ok_or(protocol::ERR_INVALID)?;
        if transaction.1.included.is_some() {
            return Err(protocol::ERR_BUSY);
        }
        let delivery = self
            .deliveries
            .get(&delivery_id)
            .filter(|delivery| delivery.owner == owner)
            .ok_or(protocol::ERR_INVALID)?;
        let included = IncludedDelivery {
            consumer_id: delivery.consumer_id,
            offset: delivery.offset,
            next_offset: delivery.next_offset,
        };
        self.deliveries.remove(&delivery_id);
        let consumer =
            self.consumers.get_mut(&included.consumer_id).ok_or(protocol::ERR_INVALID)?;
        consumer.outstanding = None;
        consumer.reserved_transaction = Some(transaction_id);
        let transaction = &mut self.transaction.as_mut().ok_or(protocol::ERR_INVALID)?.1;
        transaction.included = Some(included);
        transaction.touched = true;
        Ok(())
    }

    fn finish_transaction(
        &mut self,
        owner: ClientIdentity,
        transaction_id: u32,
        commit: bool,
    ) -> Result<(), i64> {
        let owned = self
            .transaction
            .as_ref()
            .is_some_and(|(id, transaction)| *id == transaction_id && transaction.owner == owner);
        if !owned {
            return Err(protocol::ERR_INVALID);
        }
        let (_, transaction) = self.transaction.take().ok_or(protocol::ERR_INVALID)?;
        let mut result = if transaction.touched {
            let offsets = if commit {
                match transaction.included.as_ref() {
                    Some(included) => self.broker.add_transactional_offset(
                        &self.profile,
                        self.transactional_producer,
                        included.next_offset,
                    ),
                    None => Ok(()),
                }
            } else {
                Ok(())
            };
            offsets.and_then(|()| {
                self.broker.end_transaction(&self.profile, self.transactional_producer, commit)
            })
        } else {
            Ok(())
        };
        if result.is_err() && commit && transaction.touched {
            let _ = self.broker.end_transaction(&self.profile, self.transactional_producer, false);
        }
        if transaction.reset_producer || result.is_err() {
            match self.broker.init_producer(
                Some(&self.profile.transactional_id),
                self.profile.transaction_timeout_ms,
            ) {
                Ok(producer) => {
                    self.transactional_producer = producer;
                    self.transactional_sequence = 0;
                }
                Err(error) => result = Err(error),
            }
        }
        if let Some(included) = transaction.included
            && let Some(consumer) = self.consumers.get_mut(&included.consumer_id)
        {
            consumer.reserved_transaction = None;
            if commit && result.is_ok() {
                consumer.committed_offset = included.next_offset;
                consumer.fetch_offset = included.next_offset;
            } else {
                consumer.fetch_offset = included.offset;
            }
        }
        if commit && result.is_ok() {
            self.commits = self.commits.wrapping_add(1);
        } else {
            self.aborts = self.aborts.wrapping_add(1);
        }
        result
    }

    fn account(&mut self, result: i64) {
        self.requests = self.requests.wrapping_add(1);
        config::write::<u32>(status::REQUESTS, self.requests);
        config::write::<u32>(status::PRODUCED, self.produced);
        config::write::<u32>(status::CONSUMED, self.consumed);
        config::write::<u32>(status::COMMITS, self.commits);
        config::write::<u32>(status::ABORTS, self.aborts);
        config::write::<u32>(status::BACKPRESSURE, self.backpressure);
        if result == protocol::ERR_FENCED {
            config::write::<u32>(status::ERROR, 0x4b46_0001);
        }
    }
}

fn reply_record(reply: ReplyToken, id: u32, profile: &Profile, record: &wire::Record) -> bool {
    let memory = match OwnedMemory::allocate(1) {
        Ok(memory) => memory,
        Err(_) => return false,
    };
    let mut mapping = match memory.map_writable() {
        Ok(mapping) => mapping,
        Err(_) => return false,
    };
    let delivered = DeliveredRecord {
        partition: profile.partition,
        offset: record.offset,
        timestamp_ms: record.timestamp,
        key: record.key.as_deref(),
        value: record.value.as_deref(),
    };
    let Some(_len) = delivered.encode(mapping.as_mut_slice()) else {
        return false;
    };
    let memory = match mapping.unmap() {
        Ok(memory) => memory,
        Err(_) => return false,
    };
    reply.reply_move(memory, i64::from(id)).is_ok()
}

fn handle_message(service: &mut Service<'_>, mut message: catten_rt::owned::IncomingMessage) {
    let Some(reply) = message.reply.take() else {
        return;
    };
    let owner = ClientIdentity {
        domain: message.sender,
        generation: message.sender_generation,
    };
    let mut result = 0;
    match message.opcode {
        protocol::OP_PRODUCE => {
            let record = usize::try_from(message.arg0)
                .map_err(|_| protocol::ERR_INVALID)
                .and_then(|len| decode_record(message.memory.take(), len, service.clock));
            result =
                record.and_then(|record| service.produce(record)).unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_CONSUMER_OPEN => {
            result = service.open_consumer(owner).map(i64::from).unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_CONSUMER_POLL => {
            let consumer_id = u32::try_from(message.arg0).map_err(|_| protocol::ERR_INVALID);
            match consumer_id.and_then(|id| service.poll_consumer(owner, id)) {
                Ok(Some((id, record))) => {
                    result = i64::from(id);
                    if !reply_record(reply, id, &service.profile, &record) {
                        let _ = service.release_delivery(owner, id);
                        result = protocol::ERR_TRANSPORT;
                    }
                }
                Ok(None) => {
                    let _ = reply.reply(0);
                }
                Err(error) => {
                    result = error;
                    let _ = reply.reply(error);
                }
            }
        }
        protocol::OP_DELIVERY_COMMIT | protocol::OP_DELIVERY_RELEASE => {
            let id = u32::try_from(message.arg0).map_err(|_| protocol::ERR_INVALID);
            let operation = if message.opcode == protocol::OP_DELIVERY_COMMIT {
                Service::commit_delivery
            } else {
                Service::release_delivery
            };
            result = id
                .and_then(|id| operation(service, owner, id))
                .map(|()| 0)
                .unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_CONSUMER_CLOSE => {
            result = u32::try_from(message.arg0)
                .map_err(|_| protocol::ERR_INVALID)
                .and_then(|id| service.close_consumer(owner, id))
                .map(|()| 0)
                .unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_TX_BEGIN => {
            result = service.begin_transaction(owner).map(i64::from).unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_TX_PRODUCE => {
            let transaction_id = message.arg0 as u32;
            let len = (message.arg0 >> 32) as usize;
            let record = decode_record(message.memory.take(), len, service.clock);
            result = record
                .and_then(|record| service.transaction_produce(owner, transaction_id, record))
                .unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_TX_INCLUDE_DELIVERY => {
            let transaction_id = message.arg0 as u32;
            let delivery_id = (message.arg0 >> 32) as u32;
            result = service
                .include_delivery(owner, transaction_id, delivery_id)
                .map(|()| 0)
                .unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_TX_COMMIT | protocol::OP_TX_ABORT => {
            result = u32::try_from(message.arg0)
                .map_err(|_| protocol::ERR_INVALID)
                .and_then(|id| {
                    service.finish_transaction(owner, id, message.opcode == protocol::OP_TX_COMMIT)
                })
                .map(|()| 0)
                .unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_STATUS => {
            result = (service.profile.rights & 0xffff) as i64
                | ((service.profile.partition as i64 & 0xffff) << 16);
            let _ = reply.reply(result);
        }
        _ => {
            result = protocol::ERR_BAD_OPCODE;
            let _ = reply.reply(result);
        }
    }
    service.account(result);
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let profile = Profile::from_context(&ctx).unwrap_or_else(|| fail(0x4b01));
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| fail(0x4b02));
    let (_, tcp_connection) =
        wait_for_registered_name_owned(ns_connection, socket::NAME).unwrap_or_else(|| fail(0x4b03));
    let (_, time_connection) =
        wait_for_registered_name_owned(ns_connection, time::NAME).unwrap_or_else(|| fail(0x4b04));
    let entropy_connection =
        try_registered_name_owned(ns_connection, entropy::NAME).map(|(_, connection)| connection);
    if !wait_for_local_ready_owned(ns_connection) {
        fail(0x4b05);
    }
    while !tcpip_has_ipv4(tcp_connection.as_ref()) {
        sleep_ms(250);
    }
    if profile.tls {
        let mut synchronized = false;
        for _ in 0..TLS_TIME_ATTEMPTS {
            if tls_unix_seconds(time_connection.as_ref()).is_some() {
                synchronized = true;
                break;
            }
            sleep_ms(TLS_TIME_RETRY_MS);
        }
        if !synchronized {
            catten_rt::logln!("[kafka] TLS unavailable: time service did not synchronize");
            fail(0x4b00 | (-protocol::ERR_TLS_REQUIRED as u32 & 0xff));
        }
    }

    let mut broker = BrokerSession::new(
        tcp_connection.as_ref(),
        entropy_connection.as_ref().map(Connection::as_ref),
        time_connection.as_ref(),
        &profile,
    );
    let (non_transactional_producer, transactional_producer) =
        broker.bootstrap(&profile).unwrap_or_else(|error| fail(0x4b00 | (-error as u32 & 0xff)));

    let endpoint = Endpoint::create(protocol::INTERFACE, protocol::VERSION, 32)
        .unwrap_or_else(|_| fail(0x4b06));
    let registration = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            protocol::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .unwrap_or_else(|_| fail(0x4b07));
    if !registration.wait().is_ok_and(|reply| reply.result >= 1)
        || endpoint.bind_completion_queue(0).is_err()
    {
        fail(0x4b07);
    }
    catten_rt::logln!(
        "[kafka] serving broker={}:{} tls={} topic={} partition={} group={} transactional-id={} \
         rights={:#x}",
        profile.host,
        profile.port,
        profile.tls,
        core::str::from_utf8(&profile.topic).unwrap_or("?"),
        profile.partition,
        core::str::from_utf8(&profile.group).unwrap_or("?"),
        core::str::from_utf8(&profile.transactional_id).unwrap_or("?"),
        profile.rights
    );
    config::write::<u32>(status::STAGE, 2);

    let mut service = Service {
        profile,
        clock: time_connection.as_ref(),
        broker,
        non_transactional_producer,
        transactional_producer,
        non_transactional_sequence: 0,
        transactional_sequence: 0,
        consumers: BTreeMap::new(),
        deliveries: BTreeMap::new(),
        transaction: None,
        next_id: 1,
        requests: 0,
        produced: 0,
        consumed: 0,
        commits: 0,
        aborts: 0,
        backpressure: 0,
    };
    loop {
        match endpoint.receive() {
            Ok(message) => handle_message(&mut service, message),
            Err(catten_rt::owned::ReceiveError::EndpointClosed) => unsafe { thread_exit() },
            Err(_) => config::write::<u32>(status::ERROR, 0x4b08),
        }
    }
}
