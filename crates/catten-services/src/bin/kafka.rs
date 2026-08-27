//! Capability-oriented Kafka producer, fixed-partition consumer, and
//! transactional consume-transform-produce service.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{
    boxed::Box,
    collections::{
        BTreeMap,
        BTreeSet,
    },
    string::String,
    vec::Vec,
};

use catten_rt::{
    Context,
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
const ROUTING_ATTEMPTS: usize = 3;
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
    bootstrap: BrokerDestination,
    broker_endpoints: Vec<BrokerDestination>,
    tls: bool,
    ca_der: Vec<u8>,
    topic: Vec<u8>,
    partition: i32,
    produce_routes: Vec<TopicPartition>,
    group: Vec<u8>,
    transactional_id: Vec<u8>,
    rights: u64,
    transaction_timeout_ms: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BrokerDestination {
    ip: [u8; 4],
    host: String,
    port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TopicPartition {
    topic: Vec<u8>,
    partition: i32,
}

impl Profile {
    fn from_context(ctx: &Context) -> Option<Self> {
        let memory = ctx.profile_memory()?;
        let mapping = memory.map_read_only().ok()?;
        let profile = protocol::Profile::decode(mapping.as_slice()).ok()?;
        Some(Self {
            bootstrap: BrokerDestination {
                ip: profile.endpoint_ipv4,
                host: String::from_utf8(profile.host.to_vec()).ok()?,
                port: profile.port,
            },
            broker_endpoints: profile
                .broker_endpoints
                .iter()
                .map(|broker| {
                    Some(BrokerDestination {
                        ip: broker.endpoint_ipv4,
                        host: String::from_utf8(broker.host.to_vec()).ok()?,
                        port: broker.port,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            tls: profile.tls,
            ca_der: profile.ca_certificate_der.to_vec(),
            topic: profile.topic.to_vec(),
            partition: profile.partition,
            produce_routes: profile
                .produce_routes
                .iter()
                .map(|route| TopicPartition {
                    topic: route.topic.to_vec(),
                    partition: route.partition,
                })
                .collect(),
            group: profile.group.to_vec(),
            transactional_id: profile.transactional_id.to_vec(),
            rights: profile.rights,
            transaction_timeout_ms: profile.transaction_timeout_ms.try_into().ok()?,
        })
    }

    fn has(&self, right: u64) -> bool {
        self.rights & right != 0
    }

    fn route(&self, index: u16) -> Option<(&[u8], i32)> {
        if index == protocol::DEFAULT_ROUTE {
            return Some((&self.topic, self.partition));
        }
        self.produce_routes
            .get(usize::from(index) - 1)
            .map(|route| (route.topic.as_slice(), route.partition))
    }

    fn broker_destination(&self, host: &str, port: i32) -> Option<&BrokerDestination> {
        let port = u16::try_from(port).ok()?;
        core::iter::once(&self.bootstrap)
            .chain(&self.broker_endpoints)
            .find(|broker| broker.host == host && broker.port == port)
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
        destination: &BrokerDestination,
        tls: bool,
        ca_der: &[u8],
    ) -> Self {
        Self {
            tcp,
            entropy,
            clock,
            ip: destination.ip,
            port: destination.port,
            tls,
            host: destination.host.clone(),
            ca_der: ca_der.to_vec(),
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
    tcp: ConnectionRef<'connection>,
    entropy: Option<ConnectionRef<'connection>>,
    clock: ConnectionRef<'connection>,
    bootstrap: BrokerTransport<'connection>,
    seeds: Vec<BrokerTransport<'connection>>,
    brokers: BTreeMap<i32, BrokerTransport<'connection>>,
    route_nodes: BTreeMap<u16, i32>,
    group_coordinator: Option<i32>,
    transaction_coordinator: Option<i32>,
    correlation: i32,
}

impl<'connection> BrokerSession<'connection> {
    fn new(
        tcp: ConnectionRef<'connection>,
        entropy: Option<ConnectionRef<'connection>>,
        clock: ConnectionRef<'connection>,
        profile: &Profile,
    ) -> Self {
        let seeds = profile
            .broker_endpoints
            .iter()
            .map(|destination| {
                BrokerTransport::new(tcp, entropy, clock, destination, profile.tls, &profile.ca_der)
            })
            .collect();
        Self {
            tcp,
            entropy,
            clock,
            bootstrap: BrokerTransport::new(
                tcp,
                entropy,
                clock,
                &profile.bootstrap,
                profile.tls,
                &profile.ca_der,
            ),
            seeds,
            brokers: BTreeMap::new(),
            route_nodes: BTreeMap::new(),
            group_coordinator: None,
            transaction_coordinator: None,
            correlation: 1,
        }
    }

    fn next(&mut self) -> i32 {
        let value = self.correlation;
        self.correlation = self.correlation.wrapping_add(1).max(1);
        value
    }

    fn exchange_any(&mut self, request: &[u8]) -> Result<Vec<u8>, i64> {
        let mut last_error = match self.bootstrap.request(request) {
            Ok(response) => return Ok(response),
            Err(error) => Some(error),
        };
        for seed in &mut self.seeds {
            match seed.request(request) {
                Ok(response) => return Ok(response),
                Err(error) => last_error = Some(error),
            }
        }
        let nodes: Vec<i32> = self.brokers.keys().copied().collect();
        for node in nodes {
            match self.brokers.get_mut(&node).ok_or(protocol::ERR_TRANSPORT)?.request(request) {
                Ok(response) => return Ok(response),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or(protocol::ERR_TRANSPORT))
    }

    fn exchange_node(&mut self, node: i32, request: Vec<u8>) -> Result<Vec<u8>, i64> {
        self.brokers.get_mut(&node).ok_or(protocol::ERR_DENIED)?.request(&request)
    }

    fn bootstrap(
        &mut self,
        profile: &Profile,
    ) -> Result<(ProducerIdentity, ProducerIdentity), i64> {
        let correlation = self.next();
        let request = wire::api_versions_request(correlation, CLIENT_ID).map_err(map_wire)?;
        let response = self.exchange_any(&request)?;
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

        self.refresh_routes(profile)?;

        self.group_coordinator = Some(self.find_coordinator(profile, &profile.group, false)?);
        self.transaction_coordinator =
            Some(self.find_coordinator(profile, &profile.transactional_id, true)?);

        let leader = self.route_node(protocol::DEFAULT_ROUTE)?;
        let non_transactional = self.init_producer(leader, None, profile.transaction_timeout_ms)?;
        let transactional = self.init_producer(
            self.transaction_coordinator.ok_or(protocol::ERR_BROKER)?,
            Some(&profile.transactional_id),
            profile.transaction_timeout_ms,
        )?;
        Ok((non_transactional, transactional))
    }

    fn refresh_routes(&mut self, profile: &Profile) -> Result<(), i64> {
        let mut topics: Vec<&[u8]> = Vec::with_capacity(profile.produce_routes.len() + 1);
        topics.push(&profile.topic);
        for route in &profile.produce_routes {
            if !topics.contains(&route.topic.as_slice()) {
                topics.push(&route.topic);
            }
        }
        let correlation = self.next();
        let request =
            wire::metadata_request_many(correlation, CLIENT_ID, &topics).map_err(map_wire)?;
        let response = self.exchange_any(&request)?;
        let metadata = wire::parse_metadata_many(&response, correlation).map_err(map_wire)?;
        let mut brokers = BTreeMap::new();
        for broker in &metadata.brokers {
            if let Some(destination) = profile.broker_destination(&broker.host, broker.port) {
                brokers.insert(
                    broker.node_id,
                    BrokerTransport::new(
                        self.tcp,
                        self.entropy,
                        self.clock,
                        destination,
                        profile.tls,
                        &profile.ca_der,
                    ),
                );
            }
        }
        let mut route_nodes = BTreeMap::new();
        for (route, (topic, partition)) in core::iter::once((
            protocol::DEFAULT_ROUTE,
            (profile.topic.as_slice(), profile.partition),
        ))
        .chain(profile.produce_routes.iter().enumerate().map(|(index, route)| {
            (
                u16::try_from(index + 1).expect("profile route index exceeds u16"),
                (route.topic.as_slice(), route.partition),
            )
        })) {
            let topic_metadata = metadata
                .topics
                .iter()
                .find(|candidate| candidate.topic == topic)
                .ok_or(protocol::ERR_BROKER)?;
            if topic_metadata.error != wire::NO_ERROR {
                return Err(protocol::ERR_BROKER);
            }
            let leader = topic_metadata
                .partitions
                .iter()
                .find(|candidate| candidate.partition == partition)
                .ok_or(protocol::ERR_UNSUPPORTED)?;
            if leader.error != wire::NO_ERROR {
                return Err(map_broker(leader.error));
            }
            if !brokers.contains_key(&leader.leader) {
                // Metadata is discovery, not authority: an application profile
                // must authorize every selected network destination.
                return Err(protocol::ERR_DENIED);
            }
            route_nodes.insert(route, leader.leader);
        }
        self.brokers = brokers;
        self.route_nodes = route_nodes;
        Ok(())
    }

    fn find_coordinator(
        &mut self,
        profile: &Profile,
        key: &[u8],
        transaction: bool,
    ) -> Result<i32, i64> {
        for _ in 0..COORDINATOR_ATTEMPTS {
            let correlation = self.next();
            let request = wire::find_coordinator_request(correlation, CLIENT_ID, key, transaction)
                .map_err(map_wire)?;
            let response = self.exchange_any(&request)?;
            let coordinator =
                wire::parse_find_coordinator(&response, correlation).map_err(map_wire)?;
            if coordinator.error == wire::NO_ERROR {
                let destination = profile
                    .broker_destination(&coordinator.host, coordinator.port)
                    .ok_or(protocol::ERR_DENIED)?;
                self.brokers.entry(coordinator.node_id).or_insert_with(|| {
                    BrokerTransport::new(
                        self.tcp,
                        self.entropy,
                        self.clock,
                        destination,
                        profile.tls,
                        &profile.ca_der,
                    )
                });
                return Ok(coordinator.node_id);
            }
            if !wire::is_retriable_broker_error(coordinator.error) {
                return Err(map_broker(coordinator.error));
            }
            sleep_ms(COORDINATOR_RETRY_MS);
        }
        Err(protocol::ERR_TIMEOUT)
    }

    fn route_node(&self, route: u16) -> Result<i32, i64> {
        self.route_nodes.get(&route).copied().ok_or(protocol::ERR_DENIED)
    }

    fn init_producer(
        &mut self,
        node: i32,
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
            let response = self.exchange_node(node, request)?;
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
        route: u16,
        producer: ProducerIdentity,
        sequence: i32,
        transactional: bool,
        record: &OwnedRecord,
    ) -> Result<i64, i64> {
        let (topic, partition) = profile.route(route).ok_or(protocol::ERR_DENIED)?;
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
        for attempt in 0..ROUTING_ATTEMPTS {
            let correlation = self.next();
            let request = wire::produce_request(
                correlation,
                CLIENT_ID,
                transactional.then_some(profile.transactional_id.as_slice()),
                topic,
                partition,
                &batch,
                PRODUCE_TIMEOUT_MS,
            )
            .map_err(map_wire)?;
            let node = self.route_node(route)?;
            match self.exchange_node(node, request).and_then(|response| {
                wire::parse_produce(&response, correlation, topic, partition).map_err(map_wire)
            }) {
                Ok(result) if result.error == wire::NO_ERROR => return Ok(result.base_offset),
                Ok(result)
                    if attempt + 1 < ROUTING_ATTEMPTS
                        && wire::is_retriable_broker_error(result.error) =>
                {
                    self.refresh_routes(profile)?;
                }
                Ok(result) => return Err(map_broker(result.error)),
                Err(error)
                    if attempt + 1 < ROUTING_ATTEMPTS
                        && (error == protocol::ERR_TRANSPORT || error == protocol::ERR_TIMEOUT) =>
                {
                    self.refresh_routes(profile)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(protocol::ERR_TIMEOUT)
    }

    fn add_partition(
        &mut self,
        profile: &Profile,
        route: u16,
        producer: ProducerIdentity,
    ) -> Result<(), i64> {
        let (topic, partition) = profile.route(route).ok_or(protocol::ERR_DENIED)?;
        let node = self.transaction_coordinator.ok_or(protocol::ERR_BROKER)?;
        for _ in 0..COORDINATOR_ATTEMPTS {
            let correlation = self.next();
            let request = wire::add_partitions_to_txn_request(
                correlation,
                CLIENT_ID,
                &profile.transactional_id,
                producer,
                topic,
                partition,
            )
            .map_err(map_wire)?;
            let response = self.exchange_node(node, request)?;
            match wire::parse_partition_error(&response, correlation, topic, partition) {
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
        let node = self.group_coordinator.ok_or(protocol::ERR_BROKER)?;
        let correlation = self.next();
        let request = wire::offset_fetch_request(
            correlation,
            CLIENT_ID,
            &profile.group,
            &profile.topic,
            profile.partition,
        )
        .map_err(map_wire)?;
        let response = self.exchange_node(node, request)?;
        wire::parse_offset_fetch(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)
    }

    fn earliest_offset(&mut self, profile: &Profile) -> Result<i64, i64> {
        for attempt in 0..ROUTING_ATTEMPTS {
            let node = self.route_node(protocol::DEFAULT_ROUTE)?;
            let correlation = self.next();
            let request = wire::list_offsets_request(
                correlation,
                CLIENT_ID,
                &profile.topic,
                profile.partition,
                true,
            )
            .map_err(map_wire)?;
            match self.exchange_node(node, request).and_then(|response| {
                wire::parse_list_offsets(&response, correlation, &profile.topic, profile.partition)
                    .map_err(map_wire)
            }) {
                Ok(offset) => return Ok(offset),
                Err(error)
                    if attempt + 1 < ROUTING_ATTEMPTS
                        && (error == protocol::ERR_BROKER
                            || error == protocol::ERR_TRANSPORT
                            || error == protocol::ERR_TIMEOUT) =>
                {
                    self.refresh_routes(profile)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(protocol::ERR_TIMEOUT)
    }

    fn fetch(&mut self, profile: &Profile, offset: i64) -> Result<Vec<wire::Record>, i64> {
        for attempt in 0..ROUTING_ATTEMPTS {
            let node = self.route_node(protocol::DEFAULT_ROUTE)?;
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
            match self.exchange_node(node, request).and_then(|response| {
                wire::parse_fetch(&response, correlation, &profile.topic, profile.partition)
                    .map_err(map_wire)
            }) {
                Ok(result) if result.error == wire::NO_ERROR => return Ok(result.records),
                Ok(result)
                    if attempt + 1 < ROUTING_ATTEMPTS
                        && wire::is_retriable_broker_error(result.error) =>
                {
                    self.refresh_routes(profile)?;
                }
                Ok(result) => return Err(map_broker(result.error)),
                Err(error)
                    if attempt + 1 < ROUTING_ATTEMPTS
                        && (error == protocol::ERR_TRANSPORT || error == protocol::ERR_TIMEOUT) =>
                {
                    self.refresh_routes(profile)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(protocol::ERR_TIMEOUT)
    }

    fn commit_offset(&mut self, profile: &Profile, next_offset: i64) -> Result<(), i64> {
        let node = self.group_coordinator.ok_or(protocol::ERR_BROKER)?;
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
        let response = self.exchange_node(node, request)?;
        wire::parse_offset_commit(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)
    }

    fn add_transactional_offset(
        &mut self,
        profile: &Profile,
        producer: ProducerIdentity,
        next_offset: i64,
    ) -> Result<(), i64> {
        let transaction_node = self.transaction_coordinator.ok_or(protocol::ERR_BROKER)?;
        let correlation = self.next();
        let request = wire::add_offsets_to_txn_request(
            correlation,
            CLIENT_ID,
            &profile.transactional_id,
            producer,
            &profile.group,
        )
        .map_err(map_wire)?;
        let response = self.exchange_node(transaction_node, request)?;
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
        let group_node = self.group_coordinator.ok_or(protocol::ERR_BROKER)?;
        let response = self.exchange_node(group_node, request)?;
        wire::parse_partition_error(&response, correlation, &profile.topic, profile.partition)
            .map_err(map_wire)
    }

    fn end_transaction(
        &mut self,
        profile: &Profile,
        producer: ProducerIdentity,
        commit: bool,
    ) -> Result<(), i64> {
        let node = self.transaction_coordinator.ok_or(protocol::ERR_BROKER)?;
        let correlation = self.next();
        let request = wire::end_txn_request(
            correlation,
            CLIENT_ID,
            &profile.transactional_id,
            producer,
            commit,
        )
        .map_err(map_wire)?;
        let response = self.exchange_node(node, request)?;
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
    partitions_added: BTreeSet<u16>,
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
    non_transactional_sequences: BTreeMap<u16, i32>,
    transactional_sequences: BTreeMap<u16, i32>,
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

    fn produce(&mut self, route: u16, record: OwnedRecord) -> Result<i64, i64> {
        if !self.profile.has(protocol::RIGHT_PRODUCE) {
            return Err(protocol::ERR_DENIED);
        }
        let sequence = self.non_transactional_sequences.get(&route).copied().unwrap_or(0);
        let offset = self.broker.produce(
            &self.profile,
            route,
            self.non_transactional_producer,
            sequence,
            false,
            &record,
        )?;
        self.non_transactional_sequences
            .insert(route, sequence.checked_add(1).ok_or(protocol::ERR_FENCED)?);
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
                partitions_added: BTreeSet::new(),
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
        route: u16,
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
        if !transaction.1.partitions_added.contains(&route) {
            if let Err(error) =
                self.broker.add_partition(&self.profile, route, self.transactional_producer)
            {
                let transaction = &mut self.transaction.as_mut().ok_or(protocol::ERR_INVALID)?.1;
                // AddPartitionsToTxn may have reached the coordinator even
                // when its reply did not. Force a fencing reinitialization
                // after the Drop-driven abort path.
                transaction.touched = true;
                transaction.reset_producer = true;
                return Err(error);
            }
            self.transaction
                .as_mut()
                .ok_or(protocol::ERR_INVALID)?
                .1
                .partitions_added
                .insert(route);
        }
        let sequence = self.transactional_sequences.get(&route).copied().unwrap_or(0);
        let result = self.broker.produce(
            &self.profile,
            route,
            self.transactional_producer,
            sequence,
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
        self.transactional_sequences
            .insert(route, sequence.checked_add(1).ok_or(protocol::ERR_FENCED)?);
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
            let coordinator = self.broker.transaction_coordinator.ok_or(protocol::ERR_BROKER);
            match coordinator.and_then(|node| {
                self.broker.init_producer(
                    node,
                    Some(&self.profile.transactional_id),
                    self.profile.transaction_timeout_ms,
                )
            }) {
                Ok(producer) => {
                    self.transactional_producer = producer;
                    self.transactional_sequences.clear();
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
            result = record
                .and_then(|record| service.produce(protocol::DEFAULT_ROUTE, record))
                .unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_PRODUCE_TO => {
            let (resource_id, route, len) = protocol::unpack_routed_record_arg(message.arg0);
            let record = if resource_id == 0 {
                decode_record(message.memory.take(), len, service.clock)
            } else {
                Err(protocol::ERR_INVALID)
            };
            result = record
                .and_then(|record| service.produce(route, record))
                .unwrap_or_else(|error| error);
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
                .and_then(|record| {
                    service.transaction_produce(
                        owner,
                        transaction_id,
                        protocol::DEFAULT_ROUTE,
                        record,
                    )
                })
                .unwrap_or_else(|error| error);
            let _ = reply.reply(result);
        }
        protocol::OP_TX_PRODUCE_TO => {
            let (transaction_id, route, len) = protocol::unpack_routed_record_arg(message.arg0);
            let record = decode_record(message.memory.take(), len, service.clock);
            result = record
                .and_then(|record| {
                    service.transaction_produce(owner, transaction_id, route, record)
                })
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
        "[kafka] serving broker={}:{} tls={} consume-topic={} partition={} produce-routes={} \
         group={} transactional-id={} rights={:#x}",
        profile.bootstrap.host,
        profile.bootstrap.port,
        profile.tls,
        core::str::from_utf8(&profile.topic).unwrap_or("?"),
        profile.partition,
        profile.produce_routes.len(),
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
        non_transactional_sequences: BTreeMap::new(),
        transactional_sequences: BTreeMap::new(),
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
