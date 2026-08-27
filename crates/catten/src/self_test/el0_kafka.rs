//! Opt-in end-to-end Kafka verifier backed by the run script's three-broker
//! KRaft fixture.

use crate::{
    ipc::ConnectionRights,
    logln,
    service::{
        launch::{
            KafkaAuthentication,
            KafkaBrokerEndpoint,
            KafkaProduceRoute,
            KafkaProfile,
        },
        supervisor::{
            self,
            NameServiceHandle,
        },
    },
};

static mut KAFKA_NS: Option<NameServiceHandle> = None;

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
    loop {
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
                "[kafka-test] SUCCESS: committed transaction and group offset at output offset {}",
                offset
            );
            crate::self_test::results::pass(crate::self_test::results::TestId::Kafka);
            return;
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
    }
}
