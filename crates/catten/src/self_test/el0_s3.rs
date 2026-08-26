//! Opt-in end-to-end S3/TLS verifier backed by the run script's RustFS
//! container. Production network/time services remain part of steady state;
//! this module adds only the provisioned S3 profile and smoke application.

use crate::{
    ipc::ConnectionRights,
    logln,
    service::{
        launch::S3Profile,
        supervisor::{
            self,
            NameServiceHandle,
        },
    },
};

static mut S3_NS: Option<NameServiceHandle> = None;

pub fn test_el0_s3() {
    logln!("Testing EL0 S3 client over verified TLS against RustFS...");
    unsafe { S3_NS = Some(supervisor::node_name_service()) };
    let _ = crate::self_test::results::spawn_verifier(
        crate::self_test::results::TestId::S3,
        verify_el0_s3,
    );
}

extern "C" fn verify_el0_s3() {
    use crate::cpu::scheduler::yield_lp;

    let ns = unsafe { S3_NS.as_ref() }.expect("[s3-test] name service missing");
    let ca_der = include_bytes!(env!("CATTEN_S3_TEST_CA_DER"));
    let service = crate::service::launch::launch_s3_profile(
        ns,
        &S3Profile {
            endpoint_ipv4: [10, 0, 2, 2],
            host: b"rustfs.test",
            port: 19_000,
            tls: true,
            ca_certificate_der: Some(ca_der),
            region: b"us-east-1",
            bucket: b"charlotte-test",
            prefix: b"",
            access_key: b"charlotte-test-access",
            secret_key: b"charlotte-test-secret-2026",
            namespace: None,
            rights: charlotte_protocol_s3::RIGHT_GET
                | charlotte_protocol_s3::RIGHT_PUT
                | charlotte_protocol_s3::RIGHT_DELETE,
        },
    );
    logln!("[s3-test] S3 service spawned (asid={})", service.asid);
    let service_status: *const u8 = {
        let base: *mut u8 = service.status_frame.into();
        base
    };

    let client = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"s3_smoke").expect("[s3-test] s3_smoke.elf"),
        ns,
        ConnectionRights::CALL,
    );
    let status: *const u8 = {
        let base: *mut u8 = client.status_frame.into();
        base
    };
    let deadline = crate::self_test::results::Deadline::after_millis(180_000);
    loop {
        let stage = unsafe {
            crate::self_test::status_u32(status, charlotte_launch::s3_smoke_status::STAGE)
        };
        let error = unsafe {
            crate::self_test::status_u32(status, charlotte_launch::s3_smoke_status::ERROR)
        };
        let service_error = unsafe {
            crate::self_test::status_u32(service_status, charlotte_launch::s3_status::ERROR)
        };
        if stage == charlotte_launch::s3_smoke_status::SUCCESS {
            let bytes = unsafe {
                crate::self_test::status_u32(status, charlotte_launch::s3_smoke_status::BYTES)
            };
            logln!("[s3-test] SUCCESS: verified {}-byte object round trip", bytes);
            crate::self_test::results::pass(crate::self_test::results::TestId::S3);
            return;
        }
        if error != 0 {
            logln!("[s3-test] FAILURE: smoke error={:#x} stage={}", error, stage);
            crate::self_test::results::fail(crate::self_test::results::TestId::S3);
            return;
        }
        if service_error != 0 {
            logln!("[s3-test] FAILURE: S3 service error={:#x}", service_error);
            crate::self_test::results::fail(crate::self_test::results::TestId::S3);
            return;
        }
        deadline.assert_pending("EL0 S3 TLS round trip");
        yield_lp();
    }
}
