use std::{
    env,
    io::{
        Read,
        Write,
    },
    net::TcpStream,
    thread,
    time::Duration,
};

use charlotte_kafka::{
    self as kafka,
    ProducerIdentity,
    RecordInput,
};

const CLIENT: &[u8] = b"charlotte-host-smoke";
const TOPIC: &[u8] = b"charlotte-events";
const GROUP: &[u8] = b"charlotte-smoke-group";
const TRANSACTIONAL_ID: &[u8] = b"charlotte-host-smoke-tx";

struct Broker {
    stream: TcpStream,
    correlation: i32,
}

impl Broker {
    fn connect(address: &str) -> Self {
        let stream = TcpStream::connect(address).expect("connect Kafka fixture");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
        Self {
            stream,
            correlation: 1,
        }
    }

    fn correlation(&mut self) -> i32 {
        let correlation = self.correlation;
        self.correlation += 1;
        correlation
    }

    fn exchange(&mut self, request: Vec<u8>) -> Vec<u8> {
        self.stream.write_all(&request).expect("write request");
        let mut length = [0; 4];
        self.stream.read_exact(&mut length).expect("read response length");
        let payload = i32::from_be_bytes(length);
        assert!((4..=kafka::MAX_FRAME_LEN as i32).contains(&payload));
        let mut response = vec![0; payload as usize + 4];
        response[..4].copy_from_slice(&length);
        self.stream.read_exact(&mut response[4..]).expect("read response");
        response
    }

    fn init_producer(&mut self, transactional_id: Option<&[u8]>) -> ProducerIdentity {
        for _ in 0..120 {
            let correlation = self.correlation();
            let request =
                kafka::init_producer_id_request(correlation, CLIENT, transactional_id, 60_000)
                    .unwrap();
            let response = self.exchange(request);
            match kafka::parse_init_producer_id(&response, correlation) {
                Ok(identity) => return identity,
                Err(kafka::Error::Broker(error)) if kafka::is_retriable_broker_error(error) => {
                    thread::sleep(Duration::from_millis(250));
                }
                Err(error) => panic!("InitProducerId failed: {error:?}"),
            }
        }
        panic!("InitProducerId coordinator did not become ready")
    }

    fn produce(
        &mut self,
        producer: ProducerIdentity,
        sequence: i32,
        transactional: bool,
        value: &[u8],
    ) -> i64 {
        let batch = kafka::encode_record_batch(
            &[RecordInput {
                timestamp_ms: 1_777_000_000_000 + i64::from(sequence),
                key: None,
                value: Some(value),
            }],
            producer,
            sequence,
            transactional,
        )
        .unwrap();
        let correlation = self.correlation();
        let request = kafka::produce_request(
            correlation,
            CLIENT,
            transactional.then_some(TRANSACTIONAL_ID),
            TOPIC,
            0,
            &batch,
            30_000,
        )
        .unwrap();
        let response = self.exchange(request);
        let result = kafka::parse_produce(&response, correlation, TOPIC, 0).unwrap();
        assert_eq!(result.error, kafka::NO_ERROR);
        result.base_offset
    }

    fn add_partition(&mut self, producer: ProducerIdentity) {
        for _ in 0..120 {
            let correlation = self.correlation();
            let request = kafka::add_partitions_to_txn_request(
                correlation,
                CLIENT,
                TRANSACTIONAL_ID,
                producer,
                TOPIC,
                0,
            )
            .unwrap();
            let response = self.exchange(request);
            match kafka::parse_partition_error(&response, correlation, TOPIC, 0) {
                Ok(()) => return,
                Err(kafka::Error::Broker(error)) if kafka::is_retriable_broker_error(error) => {
                    thread::sleep(Duration::from_millis(250));
                }
                Err(error) => panic!("AddPartitionsToTxn failed: {error:?}"),
            }
        }
        panic!("AddPartitionsToTxn did not become ready")
    }

    fn end_transaction(&mut self, producer: ProducerIdentity, commit: bool) {
        let correlation = self.correlation();
        let request =
            kafka::end_txn_request(correlation, CLIENT, TRANSACTIONAL_ID, producer, commit)
                .unwrap();
        let response = self.exchange(request);
        kafka::parse_top_level_error(&response, correlation).unwrap();
    }
}

fn main() {
    let address = env::var("CATTEN_KAFKA_ADDRESS").unwrap_or_else(|_| "127.0.0.1:19092".into());
    let mut broker = Broker::connect(&address);

    let correlation = broker.correlation();
    let request = kafka::api_versions_request(correlation, CLIENT).unwrap();
    let response = broker.exchange(request);
    let versions = kafka::parse_api_versions(&response, correlation).unwrap();
    for (api, version) in [
        (kafka::api::PRODUCE, kafka::version::PRODUCE),
        (kafka::api::FETCH, kafka::version::FETCH),
        (kafka::api::INIT_PRODUCER_ID, kafka::version::INIT_PRODUCER_ID),
        (kafka::api::END_TXN, kafka::version::END_TXN),
    ] {
        assert!(versions.supports(api, version), "broker lacks API {api} v{version}");
    }

    let correlation = broker.correlation();
    let request = kafka::metadata_request(correlation, CLIENT, TOPIC).unwrap();
    let response = broker.exchange(request);
    let metadata = kafka::parse_metadata(&response, correlation, TOPIC).unwrap();
    assert_eq!(metadata.topic_error, kafka::NO_ERROR);
    assert_eq!(metadata.partitions[0].leader, metadata.brokers[0].node_id);

    for (key, transaction) in [(GROUP, false), (TRANSACTIONAL_ID, true)] {
        let mut ready = false;
        for _ in 0..120 {
            let correlation = broker.correlation();
            let request =
                kafka::find_coordinator_request(correlation, CLIENT, key, transaction).unwrap();
            let response = broker.exchange(request);
            let coordinator = kafka::parse_find_coordinator(&response, correlation).unwrap();
            if coordinator.error == kafka::NO_ERROR {
                ready = true;
                break;
            }
            assert!(kafka::is_retriable_broker_error(coordinator.error));
            thread::sleep(Duration::from_millis(250));
        }
        assert!(ready, "coordinator did not become ready");
    }

    let plain = broker.init_producer(None);
    let transactional = broker.init_producer(Some(TRANSACTIONAL_ID));
    let outside_offset = broker.produce(plain, 0, false, b"outside");

    broker.add_partition(transactional);
    let committed_offset = broker.produce(transactional, 0, true, b"committed");
    broker.end_transaction(transactional, true);

    broker.add_partition(transactional);
    let _aborted_offset = broker.produce(transactional, 1, true, b"aborted");
    broker.end_transaction(transactional, false);

    let correlation = broker.correlation();
    let request = kafka::list_offsets_request(correlation, CLIENT, TOPIC, 0, true).unwrap();
    let response = broker.exchange(request);
    let earliest = kafka::parse_list_offsets(&response, correlation, TOPIC, 0).unwrap();

    let correlation = broker.correlation();
    let request = kafka::fetch_request(
        correlation,
        CLIENT,
        kafka::Fetch {
            topic: TOPIC,
            partition: 0,
            offset: earliest,
            max_wait_ms: 1_000,
            max_bytes: 256 * 1024,
            read_committed: true,
        },
    )
    .unwrap();
    let response = broker.exchange(request);
    let fetched = kafka::parse_fetch(&response, correlation, TOPIC, 0).unwrap();
    let values =
        fetched.records.iter().filter_map(|record| record.value.as_deref()).collect::<Vec<_>>();
    assert!(values.contains(&b"outside".as_slice()));
    assert!(values.contains(&b"committed".as_slice()));
    assert!(!values.contains(&b"aborted".as_slice()));

    let next_offset = committed_offset + 1;
    let correlation = broker.correlation();
    let request = kafka::add_offsets_to_txn_request(
        correlation,
        CLIENT,
        TRANSACTIONAL_ID,
        transactional,
        GROUP,
    )
    .unwrap();
    let response = broker.exchange(request);
    kafka::parse_top_level_error(&response, correlation).unwrap();

    let correlation = broker.correlation();
    let request = kafka::txn_offset_commit_request(
        correlation,
        CLIENT,
        kafka::TxnOffsetCommit {
            transactional_id: TRANSACTIONAL_ID,
            group_id: GROUP,
            producer: transactional,
            generation: -1,
            member_id: b"",
            topic: TOPIC,
            partition: 0,
            next_offset,
        },
    )
    .unwrap();
    let response = broker.exchange(request);
    kafka::parse_txn_offset_commit(&response, correlation, TOPIC, 0).unwrap();
    broker.end_transaction(transactional, true);

    let correlation = broker.correlation();
    let request = kafka::offset_fetch_request(correlation, CLIENT, GROUP, TOPIC, 0).unwrap();
    let response = broker.exchange(request);
    assert_eq!(
        kafka::parse_offset_fetch(&response, correlation, TOPIC, 0).unwrap(),
        Some(next_offset)
    );

    println!(
        "Kafka protocol smoke passed: outside={outside_offset}, committed={committed_offset}, \
         group-next={next_offset}"
    );
}
