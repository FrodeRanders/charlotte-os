//! Real EL0 scoped applications, configured roots and adversarial grant IPC.
use alloc::vec::Vec;

use charlotte_launch::security_probe_status as status;

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        sleep_millis,
    },
    logln,
    service::{
        bootstrap::{
            self,
            ManifestEntry,
            ManifestValue,
        },
        supervisor::{
            self,
            ProfileLaunchError,
            ServiceDomain,
        },
    },
};

const IMAGE: &[u8] = include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/probe.elf"));
const POLICY: &[u8] = include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/admitted.cdep"));
const ALTERNATE: &[u8] =
    include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/alternate.cdep"));
const ARTIFACT_KEY_HEX: &[u8] =
    include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/artifact.pub"));
const DEPLOYMENT_KEY_HEX: &[u8] =
    include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/deployment.pub"));

/// Own a test domain at the kernel/runtime boundary without constructing a userspace
/// capability.
/// Explicit stop waits and reaps. Failure-path Drop only requests an abort;
/// it must not block while another verifier is reporting a panic.
struct ProbeDomain(Option<ServiceDomain>);
impl ProbeDomain {
    fn get(&self) -> &ServiceDomain {
        self.0.as_ref().unwrap()
    }

    fn stop(mut self) {
        bootstrap::with_service_pages(self.get(), |pages| {
            pages.write_request(
                charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
                charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
                monotonic_millis().saturating_add(5_000),
            )
        })
        .expect("security probe drain pages");
        supervisor::wait_domain_exit(self.get(), 5_000);
        supervisor::teardown_domain(self.0.take().unwrap()).expect("security probe reclamation");
    }
}
impl Drop for ProbeDomain {
    fn drop(&mut self) {
        if let Some(domain) = self.0.take() {
            if !crate::memory::address_space_handle_is_current(domain.address_space) {
                return;
            }
            if supervisor::domain_exited(&domain) {
                // Failure-path Drop must not poll/wait under unknown guards.
                // A current, exited fixture root is retained for inspection.
                crate::logln!(
                    "[security] retaining exited probe on exceptional Drop asid={}",
                    domain.asid
                );
            } else {
                if let Err(error) = crate::cpu::scheduler::system_scheduler::abort_domain_threads(
                    domain.address_space,
                ) {
                    crate::logln!("[security] exceptional exact-root abort rejected: {:?}", error);
                }
            }
        }
    }
}

fn public_key(bytes: &[u8]) -> [u8; 32] {
    let text = core::str::from_utf8(bytes).expect("test public key UTF-8").trim();
    assert!(text.is_ascii() && text.len() == 64, "test public key length/encoding");
    let mut key = [0; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("test public-key hex");
    }
    key
}

fn word(domain: &ServiceDomain, offset: usize) -> u32 {
    let base: *const u8 = domain.status_frame.into();
    unsafe { crate::self_test::status_u32(base, offset) }
}

fn wait_for(domain: &ServiceDomain, predicate: impl Fn(&ServiceDomain) -> bool, what: &str) {
    wait_for_budget(domain, predicate, what, 30_000);
}

fn wait_for_budget(
    domain: &ServiceDomain,
    predicate: impl Fn(&ServiceDomain) -> bool,
    what: &str,
    budget_ms: u64,
) {
    let deadline = crate::self_test::results::Deadline::after_millis(budget_ms);
    while !predicate(domain) {
        assert_ne!(
            word(domain, status::STAGE),
            status::FAILED,
            "security probe failed check {}",
            word(domain, status::FAILURE)
        );
        assert!(!supervisor::domain_exited(domain), "security probe exited before {what}");
        deadline.assert_pending(what);
        sleep_millis(10);
    }
}

fn spawn(artifact_key: &[u8; 32], deployment_key: &[u8; 32], mode: u64) -> ProbeDomain {
    let manifest = [
        ManifestEntry {
            key: status::FOREIGN_THREAD_KEY,
            flags: 0,
            value: ManifestValue::Unsigned(supervisor::node_name_service().domain.tid as u64),
        },
        ManifestEntry {
            key: status::MODE_KEY,
            flags: 0,
            value: ManifestValue::Unsigned(mode),
        },
        ManifestEntry {
            key: status::ALTERNATE_KEY,
            flags: 0,
            value: ManifestValue::Bytes(ALTERNATE),
        },
        ManifestEntry {
            key: status::DEPLOYMENT_KEY,
            flags: 0,
            value: ManifestValue::Bytes(deployment_key),
        },
    ];
    ProbeDomain(Some(
        supervisor::try_spawn_security_probe(
            IMAGE,
            POLICY,
            deployment_key,
            artifact_key,
            &manifest,
        )
        .expect("scoped probe launch"),
    ))
}

pub fn test_el0_security() {
    crate::self_test::results::spawn_verifier(crate::self_test::results::TestId::Security, verify);
}

fn verify_publication_and_lookup_boundary(expected: Option<u64>) {
    // Serialized kernel fixture adapter: an ordinary live namespace issues
    // each public wire opcode against the already policy-published application.
    let caller = crate::service::loader::create_user_address_space();
    let ns = supervisor::node_name_service();
    let connection = crate::ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        caller,
        crate::ipc::ConnectionRights::CALL,
    )
    .expect("public lookup fixture connection");
    // Public naming metadata is observable without any invocation capability.
    // This must not turn the policy-published entry into a public grant below.
    // Current metadata grammar, including after logical endpoint death.
    let frame = charlotte_protocol_ns::encode(
        charlotte_protocol_ns::Action::Observe,
        charlotte_protocol_ns::Request {
            name: b"secavail",
            generation: 0,
        },
    )
    .unwrap();
    let memory = crate::memory::object::allocate_with_bytes(caller, &frame).unwrap();
    let call = crate::ipc::scalar_call_with_memory_move(
        caller,
        connection,
        charlotte_protocol_ns::OP_PUBLICATION_GENERATION,
        frame.len() as u64,
        memory,
    )
    .unwrap();
    assert!(crate::ipc::wait_reply_timeout(caller, call, 5_000).unwrap());
    let result = crate::ipc::poll_reply(caller, call).unwrap().unwrap();
    assert_eq!(result.result, expected.map_or(-1, |generation| generation as i64));
    assert!(result.cap.is_none() && result.memory.is_none(), "metadata cannot convey authority");
    crate::ipc::close_cap(caller, call).unwrap();

    // Retired scalar/bare-name observation rejects independently of liveness.
    for named in [false, true] {
        let call = if named {
            let memory = crate::memory::object::allocate_with_bytes(caller, b"secavail").unwrap();
            crate::ipc::scalar_call_with_memory_move(caller, connection, 18, 8, memory)
        } else {
            crate::ipc::scalar_call(caller, connection, 18, u64::from_le_bytes(*b"secavail"))
        }
        .unwrap();
        assert!(crate::ipc::wait_reply_timeout(caller, call, 5_000).unwrap());
        let result = crate::ipc::poll_reply(caller, call).unwrap().unwrap();
        assert_eq!(result.result, -2, "retired metadata shape must reject");
        assert!(result.cap.is_none() && result.memory.is_none());
        crate::ipc::close_cap(caller, call).unwrap();
    }

    // Retired bearer-key publication/lookup rejects with attachments, without
    // returning authority or making a policy-published name public. These
    // copied inputs retain their original owners in this live fixture root.
    let endpoint = crate::ipc::endpoint_create(caller, 0x4b455953, 1, 2).unwrap();
    let source = crate::ipc::connection_delegate(
        caller,
        endpoint,
        caller,
        crate::ipc::ConnectionRights::ALL,
    )
    .unwrap();
    let key_memory =
        crate::memory::object::allocate_with_bytes(caller, &0u64.to_le_bytes()).unwrap();
    for opcode in [5, 6] {
        let call = crate::ipc::scalar_call_with_connection_copy(
            caller,
            connection,
            opcode,
            u64::from_le_bytes(*b"secavail"),
            source,
            crate::ipc::ConnectionRights::ALL,
            key_memory,
        )
        .unwrap();
        assert!(crate::ipc::wait_reply_timeout(caller, call, 5_000).unwrap());
        let result = crate::ipc::poll_reply(caller, call).unwrap().unwrap();
        assert_eq!(result.result, -3, "retired bearer opcode {opcode} must reject");
        assert!(result.cap.is_none() && result.memory.is_none());
        crate::ipc::close_cap(caller, call).unwrap();
        assert!(!crate::memory::object::info(caller, key_memory).unwrap().lent);
    }
    if expected.is_none() {
        crate::self_test::close_test_address_space(caller).unwrap();
        logln!(
            "[security] current metadata observed absence; retired metadata and bearer operations \
             rejected without authority"
        );
        return;
    }
    for opcode in [2, 7, 4, 16] {
        let call = if matches!(opcode, 4 | 16) {
            let memory = crate::memory::object::allocate_with_bytes(caller, b"secavail")
                .expect("public named lookup fixture memory");
            crate::ipc::scalar_call_with_memory_move(caller, connection, opcode, 8, memory)
        } else {
            crate::ipc::scalar_call(caller, connection, opcode, u64::from_le_bytes(*b"secavail"))
        }
        .expect("public lookup fixture call");
        assert!(
            crate::ipc::wait_reply_timeout(caller, call, 5_000)
                .expect("public lookup fixture wait")
        );
        let result = crate::ipc::poll_reply(caller, call).unwrap().unwrap();
        assert_eq!(result.result, -4, "public opcode {opcode} bypassed policy publication");
        assert!(result.cap.is_none() && result.memory.is_none());
        crate::ipc::close_cap(caller, call).expect("public lookup fixture completion");
    }
    crate::self_test::close_test_address_space(caller).expect("public lookup fixture cleanup");
    logln!(
        "[security] current metadata returned no authority; retired bearer/metadata shapes \
         rejected; all four public lookup opcodes denied policy-published secavail"
    );
}

fn verify_caller_buffers(
    ns: &supervisor::NameServiceHandle,
    root: crate::memory::AddressSpaceHandle,
) -> u64 {
    use crate::ipc;
    const NS_OP_LOOKUP: u32 = 2;
    const TCPIP_NAME: u64 = u64::from_le_bytes(*b"tcpip\0\0\0");
    const SOCKET_OP_STATUS: u32 = 10;
    const STATUS_WORDS: usize = 5;
    // Kernel fixture adapter, ordinary exact namespace with a deliberately
    // small memory-object ceiling. Returned pages remain owned here.
    let caller = root.id();
    let baseline = crate::memory::budget::used(root);
    crate::memory::budget::set_limit(
        root,
        crate::memory::budget::Amount {
            pages: baseline.pages + 32,
            objects: baseline.objects + 32,
        },
    )
    .unwrap();
    let connection = ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        caller,
        ipc::ConnectionRights::CALL,
    )
    .unwrap();
    let lookup = ipc::scalar_call(caller, connection, NS_OP_LOOKUP, TCPIP_NAME).unwrap();
    let wait = |call| {
        assert!(ipc::wait_reply_timeout(caller, call, 5_000).unwrap());
        let result = ipc::poll_reply(caller, call).unwrap().unwrap();
        ipc::close_cap(caller, call).unwrap();
        result
    };
    let tcpip = wait(lookup).cap.unwrap();
    let socket_result = wait(ipc::scalar_call(caller, tcpip, 1, 2).unwrap()).result;
    assert!(socket_result > 0, "authenticated ordinary fixture socket rejected: {socket_result}");
    let socket = socket_result as u64;
    for opcode in [7, SOCKET_OP_STATUS] {
        let result = wait(ipc::scalar_call(caller, tcpip, opcode, socket).unwrap());
        assert_eq!(result.result, -7, "unbuffered reply must reject");
        assert!(result.memory.is_none());
        let memory = crate::memory::object::allocate(caller, 2).unwrap();
        let result =
            wait(ipc::scalar_call_with_memory_move(caller, tcpip, opcode, socket, memory).unwrap());
        assert_eq!(result.result, -7, "oversized reply buffer must reject");
        assert!(result.memory.is_none());
        assert_eq!(crate::memory::budget::used(root).pages, baseline.pages);
    }
    let buffer = crate::memory::object::allocate(caller, 1).unwrap();
    let receive = ipc::scalar_call_with_memory_move(caller, tcpip, 7, socket, buffer).unwrap();
    assert_eq!(wait(ipc::scalar_call(caller, tcpip, 11, socket).unwrap()).result, 0);
    let cancelled = wait(receive);
    assert_eq!(cancelled.result, -5);
    assert!(cancelled.memory.is_none());
    assert_eq!(crate::memory::budget::used(root).pages, baseline.pages);
    let readonly = crate::memory::object::allocate(caller, 1).unwrap();
    let rejected = wait(
        ipc::scalar_call_with_memory_borrow_read(caller, tcpip, SOCKET_OP_STATUS, 0, readonly)
            .unwrap(),
    );
    assert_eq!(rejected.result, -5, "read-only buffer mapping must reject");
    assert!(rejected.memory.is_none());
    crate::memory::object::write_bytes(caller, readonly, &[1]).unwrap();
    crate::memory::object::close_cap(caller, readonly).unwrap();
    assert_eq!(crate::memory::budget::used(root).pages, baseline.pages);
    assert_eq!(wait(ipc::scalar_call(caller, tcpip, 8, socket).unwrap()).result, 0);
    let mut retained = Vec::new();
    for count in 1..=32 {
        let buffer = crate::memory::object::allocate(caller, 1).unwrap();
        let result = wait(
            ipc::scalar_call_with_memory_move(caller, tcpip, SOCKET_OP_STATUS, 0, buffer).unwrap(),
        );
        assert!(result.result >= (STATUS_WORDS * 4) as i64);
        retained.push(result.memory.expect("caller buffer must return"));
        assert_eq!(crate::memory::budget::used(root).pages, baseline.pages + count);
    }
    assert!(crate::memory::object::allocate(caller, 1).is_err());
    assert_eq!(crate::memory::budget::used(root).pages, baseline.pages + 32);
    // A saturated ordinary owner cannot prevent a different sponsor's
    // status operation from making progress while all 32 pages stay live.
    let kernel_ns = ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        crate::memory::KERNEL_ASID,
        ipc::ConnectionRights::CALL,
    )
    .unwrap();
    let lookup =
        ipc::scalar_call(crate::memory::KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, TCPIP_NAME).unwrap();
    assert!(ipc::wait_reply_timeout(crate::memory::KERNEL_ASID, lookup, 5_000).unwrap());
    let kernel_tcpip =
        ipc::poll_reply(crate::memory::KERNEL_ASID, lookup).unwrap().unwrap().cap.unwrap();
    ipc::close_cap(crate::memory::KERNEL_ASID, lookup).unwrap();
    let memory = crate::memory::object::allocate(crate::memory::KERNEL_ASID, 1).unwrap();
    let status = ipc::scalar_call_with_memory_move(
        crate::memory::KERNEL_ASID,
        kernel_tcpip,
        SOCKET_OP_STATUS,
        0,
        memory,
    )
    .unwrap();
    assert!(ipc::wait_reply_timeout(crate::memory::KERNEL_ASID, status, 5_000).unwrap());
    let result = ipc::poll_reply(crate::memory::KERNEL_ASID, status).unwrap().unwrap();
    assert!(result.result >= (STATUS_WORDS * 4) as i64);
    crate::memory::object::close_cap(crate::memory::KERNEL_ASID, result.memory.unwrap()).unwrap();
    ipc::close_cap(crate::memory::KERNEL_ASID, status).unwrap();
    ipc::close_cap(crate::memory::KERNEL_ASID, kernel_tcpip).unwrap();
    ipc::close_cap(crate::memory::KERNEL_ASID, kernel_ns).unwrap();
    let external =
        crate::memory::object::move_to(caller, retained.pop().unwrap(), crate::memory::KERNEL_ASID)
            .unwrap();
    for memory in retained {
        crate::memory::object::close_cap(caller, memory).unwrap();
    }
    assert_eq!(crate::memory::budget::used(root).pages, baseline.pages + 1);
    ipc::close_cap(caller, tcpip).unwrap();
    ipc::close_cap(caller, connection).unwrap();
    logln!(
        "[tcpip] caller reply backing: missing/oversized/read-only rejected, receive cancellation \
         refunded, 32 retained pages charged to caller, independent status progressed"
    );
    external
}

fn verify_rng_caller_buffers(
    ns: &supervisor::NameServiceHandle,
    root: crate::memory::AddressSpaceHandle,
    producer: crate::memory::AddressSpaceHandle,
) -> u64 {
    use crate::{
        ipc,
        memory,
    };
    // Kernel fixture ABI adapter for RNG v2: opcode 2 returns the supplied
    // page. This intentionally tests the signed service across the IPC boundary.
    const RNG_NAME: u64 = u64::from_le_bytes(*b"rng\0\0\0\0\0");
    const FILL: u32 = 2;
    const REQUEST: u64 = 32;
    const RETAINED: u64 = 32;
    let caller = root.id();
    let baseline = memory::budget::used(root);
    let producer_baseline = memory::budget::used(producer);
    memory::budget::set_limit(
        root,
        memory::budget::Amount {
            pages: baseline.pages + RETAINED,
            objects: baseline.objects + RETAINED,
        },
    )
    .unwrap();
    let ns_connection = ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        caller,
        ipc::ConnectionRights::CALL,
    )
    .unwrap();
    let wait = |call| {
        assert!(ipc::wait_reply_timeout(caller, call, 5_000).unwrap());
        let result = ipc::poll_reply(caller, call).unwrap().unwrap();
        ipc::close_cap(caller, call).unwrap();
        result
    };
    let rng = wait(ipc::scalar_call(caller, ns_connection, 2, RNG_NAME).unwrap()).cap.unwrap();
    for (opcode, expected) in [(1, -4), (FILL, -1)] {
        let result = wait(ipc::scalar_call(caller, rng, opcode, REQUEST).unwrap());
        assert_eq!(result.result, expected, "retired or missing-buffer RNG request must reject");
        assert!(result.cap.is_none() && result.memory.is_none());
    }
    let oversized = memory::object::allocate(caller, 2).unwrap();
    let result =
        wait(ipc::scalar_call_with_memory_move(caller, rng, FILL, REQUEST, oversized).unwrap());
    assert_eq!(result.result, -1);
    assert!(result.memory.is_none());
    assert_eq!(memory::budget::used(root), baseline);
    let readonly = memory::object::allocate(caller, 1).unwrap();
    let result = wait(
        ipc::scalar_call_with_memory_borrow_read(caller, rng, FILL, REQUEST, readonly).unwrap(),
    );
    assert_eq!(result.result, -3, "RNG mapping failure must reject and revoke read loan");
    assert!(result.memory.is_none());
    memory::object::write_bytes(caller, readonly, &[0xa5]).unwrap();
    memory::object::close_cap(caller, readonly).unwrap();
    assert_eq!(memory::budget::used(root), baseline);
    // Ready is not observation: closing the unobserved result releases exactly
    // the caller's original page, without taking ownership of a returned scalar.
    let buffer = memory::object::allocate(caller, 1).unwrap();
    let unobserved = ipc::scalar_call_with_memory_move(caller, rng, FILL, REQUEST, buffer).unwrap();
    assert!(ipc::wait_reply_timeout(caller, unobserved, 5_000).unwrap());
    assert_eq!(memory::budget::used(root).pages, baseline.pages + 1);
    assert_eq!(memory::budget::used(producer), producer_baseline);
    ipc::close_cap(caller, unobserved).unwrap();
    assert_eq!(memory::budget::used(root), baseline);
    let mut retained = Vec::new();
    for count in 1..=RETAINED {
        let buffer = memory::object::allocate(caller, 1).unwrap();
        let result =
            wait(ipc::scalar_call_with_memory_move(caller, rng, FILL, REQUEST, buffer).unwrap());
        assert_eq!(result.result, REQUEST as i64);
        assert!(result.cap.is_none());
        retained.push(result.memory.expect("RNG returns caller backing"));
        assert_eq!(memory::budget::used(root).pages, baseline.pages + count);
        assert_eq!(memory::budget::used(root).objects, baseline.objects + count);
        assert_eq!(memory::budget::used(producer), producer_baseline);
    }
    assert!(memory::object::allocate(caller, 1).is_err());
    // A separately funded operation still progresses with the ordinary caller
    // at its real ceiling. This is one independent sponsor, not flood fairness.
    let independent = memory::KERNEL_ASID;
    let kernel_ns = ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        independent,
        ipc::ConnectionRights::CALL,
    )
    .unwrap();
    let lookup = ipc::scalar_call(independent, kernel_ns, 2, RNG_NAME).unwrap();
    assert!(ipc::wait_reply_timeout(independent, lookup, 5_000).unwrap());
    let kernel_rng = ipc::poll_reply(independent, lookup).unwrap().unwrap().cap.unwrap();
    ipc::close_cap(independent, lookup).unwrap();
    let buffer = memory::object::allocate(independent, 1).unwrap();
    let call =
        ipc::scalar_call_with_memory_move(independent, kernel_rng, FILL, REQUEST, buffer).unwrap();
    assert!(ipc::wait_reply_timeout(independent, call, 5_000).unwrap());
    let result = ipc::poll_reply(independent, call).unwrap().unwrap();
    assert_eq!(result.result, REQUEST as i64);
    memory::object::close_cap(independent, result.memory.unwrap()).unwrap();
    ipc::close_cap(independent, call).unwrap();
    ipc::close_cap(independent, kernel_rng).unwrap();
    ipc::close_cap(independent, kernel_ns).unwrap();
    assert_eq!(memory::budget::used(producer), producer_baseline);
    let external = memory::object::move_to(caller, retained.pop().unwrap(), independent).unwrap();
    for memory in retained {
        memory::object::close_cap(caller, memory).unwrap();
    }
    assert_eq!(memory::budget::used(root).pages, baseline.pages + 1);
    ipc::close_cap(caller, rng).unwrap();
    ipc::close_cap(caller, ns_connection).unwrap();
    logln!(
        "[rng] caller reply backing: retired/missing/oversized/read-only rejected; unobserved \
         cancellation refunded; 32 retained pages charged to caller, producer unchanged, \
         independent fill progressed"
    );
    external
}

extern "C" fn verify() {
    assert!(
        crate::service::launch::steady_state().appliance.is_some(),
        "security test needs tcpip"
    );
    let artifact_key = public_key(ARTIFACT_KEY_HEX);
    let deployment_key = public_key(DEPLOYMENT_KEY_HEX);
    assert_ne!(artifact_key, deployment_key);
    assert_ne!(artifact_key, charlotte_launch::CLUSTER_PUBLIC_KEY);
    assert_ne!(deployment_key, charlotte_launch::CLUSTER_PUBLIC_KEY);
    assert!(matches!(
        supervisor::try_spawn_with_deployment_descriptor(
            IMAGE,
            POLICY,
            &artifact_key,
            &artifact_key
        ),
        Err(ProfileLaunchError::InvalidDeploymentDescriptor)
    ));
    assert!(matches!(
        supervisor::try_spawn_with_deployment_descriptor(
            IMAGE,
            POLICY,
            &deployment_key,
            &deployment_key
        ),
        Err(ProfileLaunchError::DescriptorArtifactMismatch)
    ));
    logln!("[security] independent roots and wrong-role root rejection verified");

    let flood = spawn(&artifact_key, &deployment_key, 1);
    wait_for(
        flood.get(),
        |domain| word(domain, status::REQUESTS) >= 64,
        "cancellation traffic start",
    );
    let socket_floods =
        [spawn(&artifact_key, &deployment_key, 2), spawn(&artifact_key, &deployment_key, 2)];
    for socket_flood in &socket_floods {
        wait_for(
            socket_flood.get(),
            |domain| word(domain, status::REQUESTS) >= 64,
            "socket pressure start",
        );
    }
    let tcpip = crate::service::launch::steady_state().appliance.unwrap().tcpip;
    let tcpip_base: *const u8 = tcpip.status_frame.into();
    let tcpip_word =
        |offset: usize| unsafe { core::ptr::read_volatile(tcpip_base.add(offset).cast::<u64>()) };
    let start_ms = tcpip_word(charlotte_launch::tcpip_status::MONOTONIC_MS);
    let start_cycles = tcpip_word(charlotte_launch::tcpip_status::REACTOR_CYCLES);
    let pressure_deadline = crate::self_test::results::Deadline::after_millis(10_000);
    while tcpip_word(charlotte_launch::tcpip_status::MONOTONIC_MS) < start_ms.saturating_add(1_000)
    {
        pressure_deadline.assert_pending("TCP/IP clock progress under admitted client pressure");
        sleep_millis(10);
    }
    assert!(tcpip_word(charlotte_launch::tcpip_status::REACTOR_CYCLES) > start_cycles);
    for socket_flood in socket_floods {
        let requests = word(socket_flood.get(), status::REQUESTS);
        socket_flood.stop();
        logln!(
            "[security] TCP/IP cycle/time progress under socket pressure; retired {requests} \
             requests"
        );
    }
    let mut previous_generation = 0;
    let mut previous_domain: Option<crate::memory::AddressSpaceHandle> = None;
    for _ in 0..2 {
        let probe = spawn(&artifact_key, &deployment_key, 0);
        if let Some(retired) = previous_domain {
            assert!(!crate::memory::launch_descriptor_matches(
                retired.id(),
                retired.generation() as u64,
                &charlotte_launch::sha256::digest(POLICY)
            ));
        }
        // This aggregate includes serial five-second cancellation fixtures,
        // including the new controller CQ/moved-request case. Keep each inner
        // production deadline unchanged; allow the complete fixture to finish.
        wait_for_budget(
            probe.get(),
            |domain| word(domain, status::STAGE) == status::PASSED,
            "adversarial application grant checks",
            45_000,
        );
        assert_eq!(word(probe.get(), status::CHECKS), status::EXPECTED_CHECKS);
        let mut retained_buffers = alloc::vec![verify_caller_buffers(
            &supervisor::node_name_service(),
            probe.get().address_space
        )];
        if let Some(entropy) = crate::service::launch::steady_state().entropy {
            retained_buffers.push(verify_rng_caller_buffers(
                &supervisor::node_name_service(),
                probe.get().address_space,
                entropy.address_space,
            ));
        }
        let base: *const u8 = probe.get().status_frame.into();
        let generation = unsafe {
            core::ptr::read_volatile(base.add(status::PUBLICATION_GENERATION).cast::<u64>())
        };
        assert!(generation > previous_generation, "republication must advance service generation");
        verify_publication_and_lookup_boundary(Some(generation));
        previous_generation = generation;
        let retired = probe.get().address_space;
        let digest = charlotte_launch::sha256::digest(POLICY);
        assert!(crate::memory::launch_descriptor_matches(
            retired.id(),
            retired.generation() as u64,
            &digest
        ));
        probe.stop();
        verify_publication_and_lookup_boundary(None);
        assert_eq!(
            crate::memory::budget::used(retired).pages,
            retained_buffers.len() as u64,
            "closing original sponsor cannot refund retained backing"
        );
        let successor = crate::service::loader::create_user_address_space();
        let successor_root = crate::memory::current_address_space_handle(successor).unwrap();
        assert_eq!(crate::memory::budget::used(successor_root).pages, 0);
        for retained_buffer in retained_buffers {
            crate::memory::object::close_cap(crate::memory::KERNEL_ASID, retained_buffer).unwrap();
        }
        assert_eq!(crate::memory::budget::used(retired).pages, 0);
        assert_eq!(crate::memory::budget::used(successor_root).pages, 0);
        crate::self_test::close_test_address_space(successor).unwrap();
        logln!(
            "[caller backing] original sponsor close retained TCP/IP/RNG reply charges; physical \
             release refunded original account and left successor unaffected"
        );
        previous_domain = Some(retired);
        assert!(!crate::memory::launch_descriptor_matches(
            retired.id(),
            retired.generation() as u64,
            &digest
        ));
        logln!(
            "[security] probe passed; retired policy fenced, publication generation={generation}"
        );
    }
    wait_for(
        flood.get(),
        |domain| word(domain, status::REQUESTS) >= 512,
        "bounded cancellation stress",
    );
    let submitted = word(flood.get(), status::REQUESTS);
    flood.stop();
    logln!("[security] concurrent cancellation traffic retired after {submitted} requests");
    crate::self_test::results::pass(crate::self_test::results::TestId::Security);
}
