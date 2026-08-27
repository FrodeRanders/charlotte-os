//! Opt-in end-to-end Kafka verifier backed by the run script's three-broker
//! KRaft fixture.

use crate::{
    ipc::ConnectionRights,
    logln,
    service::{
        launch::{
            KafkaAuthentication,
            KafkaAuthorityEndpoint,
            KafkaBrokerEndpoint,
            KafkaProduceRoute,
            KafkaProfile,
            KafkaStepProfile,
        },
        supervisor::{
            self,
            NameServiceHandle,
        },
    },
};

static mut KAFKA_NS: Option<NameServiceHandle> = None;
const CONNECTOR_NAME: &[u8] = b"kafka/selftest/main";
const PRODUCER_NAME: &[u8] = b"kafka/selftest/main/producer";
const CONSUMER_NAME: &[u8] = b"kafka/selftest/main/consumer";
const TRANSACTION_NAME: &[u8] = b"kafka/selftest/main/transactional";

pub fn test_el0_kafka() {
    logln!("Testing EL0 Kafka idempotent and transactional data plane...");
    unsafe { KAFKA_NS = Some(supervisor::node_name_service()) };
    let _ = crate::self_test::results::spawn_verifier(
        crate::self_test::results::TestId::Kafka,
        verify_el0_kafka,
    );
}

extern "C" fn verify_el0_kafka() {
    use crate::cpu::scheduler::yield_lp;

    let ns = unsafe { KAFKA_NS.as_ref() }.expect("[kafka-test] name service missing");
    let ca_der = include_bytes!(env!("CATTEN_KAFKA_TEST_CA_DER"));
    let client_certificate_der = include_bytes!(env!("CATTEN_KAFKA_TEST_CLIENT_CERT_DER"));
    let client_private_key_der = include_bytes!(env!("CATTEN_KAFKA_TEST_CLIENT_KEY_DER"));
    let service = crate::service::launch::launch_kafka_profile(
        ns,
        &KafkaProfile {
            instance_name: CONNECTOR_NAME,
            authority_endpoints: &[
                KafkaAuthorityEndpoint {
                    service_name: PRODUCER_NAME,
                    rights: charlotte_protocol_kafka::RIGHT_PRODUCE,
                },
                KafkaAuthorityEndpoint {
                    service_name: CONSUMER_NAME,
                    rights: charlotte_protocol_kafka::RIGHT_CONSUME,
                },
                KafkaAuthorityEndpoint {
                    service_name: TRANSACTION_NAME,
                    rights: charlotte_protocol_kafka::ALL_RIGHTS,
                },
            ],
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"kafka-1.test",
            port: 19_092,
            broker_endpoints: &[
                KafkaBrokerEndpoint {
                    endpoint_ipv4: [10, 0, 2, 2],
                    host: b"kafka-2.test",
                    port: 19_094,
                },
                KafkaBrokerEndpoint {
                    endpoint_ipv4: [10, 0, 2, 2],
                    host: b"kafka-3.test",
                    port: 19_096,
                },
            ],
            tls: true,
            ca_certificate_der: Some(ca_der),
            topic: b"charlotte-events",
            partition: 0,
            produce_routes: &[KafkaProduceRoute {
                topic: b"charlotte-results",
                partition: 0,
            }],
            max_produce_routes: 64,
            group: b"charlotte-qemu-smoke-group",
            transactional_id: b"charlotte-qemu-smoke-transaction",
            authentication: KafkaAuthentication::ScramSha256AndMtlsP256 {
                username: b"charlotte",
                password: b"charlotte-kafka-test",
                certificate_der: client_certificate_der,
                private_key_der: client_private_key_der,
            },
            rights: charlotte_protocol_kafka::ALL_RIGHTS,
            transaction_timeout_ms: 60_000,
        },
    );
    logln!("[kafka-test] Kafka service spawned (asid={})", service.asid);
    let service_status: *const u8 = {
        let base: *mut u8 = service.status_frame.into();
        base
    };

    let client = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"kafka_smoke").expect("[kafka-test] kafka_smoke.elf"),
        ns,
        ConnectionRights::CALL,
    );
    let status: *const u8 = {
        let base: *mut u8 = client.status_frame.into();
        base
    };
    let deadline = crate::self_test::results::Deadline::after_millis(240_000);
    let output_offset = loop {
        let stage = unsafe {
            crate::self_test::status_u32(status, charlotte_launch::kafka_smoke_status::STAGE)
        };
        let error = unsafe {
            crate::self_test::status_u32(status, charlotte_launch::kafka_smoke_status::ERROR)
        };
        let service_error = unsafe {
            crate::self_test::status_u32(service_status, charlotte_launch::kafka_status::ERROR)
        };
        if stage == charlotte_launch::kafka_smoke_status::SUCCESS {
            let offset = unsafe {
                crate::self_test::status_u32(status, charlotte_launch::kafka_smoke_status::OFFSET)
            };
            logln!(
                "[kafka-test] low-level transaction committed at output offset {}; starting \
                 generic step",
                offset
            );
            break offset;
        }
        if error != 0 {
            logln!("[kafka-test] FAILURE: smoke error={:#x} stage={}", error, stage);
            crate::self_test::results::fail(crate::self_test::results::TestId::Kafka);
            return;
        }
        if service_error != 0 {
            logln!("[kafka-test] FAILURE: Kafka service error={:#x}", service_error);
            crate::self_test::results::fail(crate::self_test::results::TestId::Kafka);
            return;
        }
        deadline.assert_pending("EL0 Kafka transactional round trip");
        yield_lp();
    };

    let procedure = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"kafka_step_proc")
            .expect("[kafka-test] kafka_step_proc.elf"),
        ns,
        ConnectionRights::CALL,
    );
    let procedure_status: *const u8 = {
        let base: *mut u8 = procedure.status_frame.into();
        base
    };
    let step = crate::service::launch::launch_kafka_step(
        ns,
        &KafkaStepProfile {
            procedure_name: b"kproc",
            kafka_connector_name: TRANSACTION_NAME,
            allowed_routes: &[1],
            dlq_route: 1,
            max_outputs: 4,
            max_attempts: 2,
            procedure_timeout_ms: 50,
            retry_backoff_ms: 20,
            idle_poll_ms: 20,
        },
    );
    let step_status: *const u8 = {
        let base: *mut u8 = step.status_frame.into();
        base
    };
    loop {
        let stage = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::STAGE)
        };
        let error = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::ERROR)
        };
        let procedure_error = unsafe {
            crate::self_test::status_u32(
                procedure_status,
                charlotte_launch::kafka_step_procedure_status::ERROR,
            )
        };
        if error != 0 || procedure_error != 0 {
            logln!(
                "[kafka-test] FAILURE: step startup error={:#x} procedure_error={:#x}",
                error,
                procedure_error
            );
            crate::self_test::results::fail(crate::self_test::results::TestId::Kafka);
            return;
        }
        if stage == 2 {
            break;
        }
        deadline.assert_pending("Kafka-step startup");
        yield_lp();
    }

    let input = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"kafka_step_input")
            .expect("[kafka-test] kafka_step_input.elf"),
        ns,
        ConnectionRights::CALL,
    );
    let input_status: *const u8 = {
        let base: *mut u8 = input.status_frame.into();
        base
    };
    loop {
        let input_stage = unsafe {
            crate::self_test::status_u32(
                input_status,
                charlotte_launch::kafka_step_input_status::STAGE,
            )
        };
        let input_error = unsafe {
            crate::self_test::status_u32(
                input_status,
                charlotte_launch::kafka_step_input_status::ERROR,
            )
        };
        let step_error = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::ERROR)
        };
        let commits = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::COMMITS)
        };
        let produced = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::PRODUCED)
        };
        let retries = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::RETRIES)
        };
        let dlq = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::DLQ)
        };
        let timeouts = unsafe {
            crate::self_test::status_u32(step_status, charlotte_launch::kafka_step_status::TIMEOUTS)
        };
        let group_generation = unsafe {
            crate::self_test::status_u32(
                service_status,
                charlotte_launch::kafka_status::GROUP_GENERATION,
            )
        };
        let group_heartbeats = unsafe {
            crate::self_test::status_u32(
                service_status,
                charlotte_launch::kafka_status::GROUP_HEARTBEATS,
            )
        };
        let metadata_refreshes = unsafe {
            crate::self_test::status_u32(
                service_status,
                charlotte_launch::kafka_status::METADATA_REFRESHES,
            )
        };
        let terminal_errors = unsafe {
            crate::self_test::status_u32(
                service_status,
                charlotte_launch::kafka_status::TERMINAL_ERRORS,
            )
        };
        let route_count = unsafe {
            crate::self_test::status_u32(
                service_status,
                charlotte_launch::kafka_status::ROUTE_COUNT,
            )
        };
        let output_route_produced = unsafe {
            crate::self_test::status_i64(
                service_status,
                charlotte_launch::kafka_status::ROUTE_PRODUCED_BASE
                    + charlotte_launch::kafka_status::ROUTE_PRODUCED_STRIDE,
            )
        };
        let consumer_lag = unsafe {
            crate::self_test::status_i64(
                service_status,
                charlotte_launch::kafka_status::CONSUMER_LAG,
            )
        };
        if input_error != 0 || step_error != 0 {
            logln!(
                "[kafka-test] FAILURE: step input error={:#x} step_error={:#x}",
                input_error,
                step_error
            );
            crate::self_test::results::fail(crate::self_test::results::TestId::Kafka);
            return;
        }
        if input_stage == charlotte_launch::kafka_step_input_status::SUCCESS
            && commits >= 4
            && produced >= 4
            && retries >= 2
            && dlq >= 1
            && timeouts >= 1
            && group_generation >= 2
            && group_heartbeats >= 1
            && metadata_refreshes >= 1
            && terminal_errors >= 1
            && route_count == 2
            && output_route_produced >= 5
            && consumer_lag >= 0
        {
            logln!(
                "[kafka-test] SUCCESS: low-level offset {}, generic step commits={} produced={} \
                 retries={} dlq={} timeouts={} group_generation={} heartbeats={} \
                 metadata_refreshes={} terminal_errors={} output_route_produced={} lag={}",
                output_offset,
                commits,
                produced,
                retries,
                dlq,
                timeouts,
                group_generation,
                group_heartbeats,
                metadata_refreshes,
                terminal_errors,
                output_route_produced,
                consumer_lag
            );
            crate::self_test::results::pass(crate::self_test::results::TestId::Kafka);
            return;
        }
        deadline.assert_pending("Kafka-step retry, timeout, DLQ, and commit sequence");
        yield_lp();
    }
}
