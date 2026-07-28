#![allow(unused_assignments)]
//! Self-test: Phase 3 userspace name service and service manager.
//!
//! Uses the node name service and spawns EL0 protection domains from
//! Rust-compiled ELFs:
//!
//! - `echo.elf` — a service that creates its own endpoint and registers it by name, attaching a
//!   re-delegable connection at call time;
//! - `client.elf` — a client that looks up "echo" by name and calls it through the returned
//!   connection.
//! - `servicemgr.elf` — the only domain granted authority to spawn a replacement after completing
//!   the handoff protocol.
//!
//! No domain ever learns another domain's ASID, LP, or kernel object ids;
//! all authority flows through delegated capabilities.
//!
//! A kernel verifier thread then exercises restart semantics through the
//! same EL0 name service: it shuts the echo service down, tears down its
//! domain, observes that stale connections fail with `EndpointClosed`,
//! restarts the service, and observes the instance generation increment.

use crate::logln;
#[cfg(target_arch = "aarch64")]
use crate::{
    ipc::{
        self,
        ConnectionRights,
        IpcError,
    },
    memory::physical::PAddr,
    service::supervisor::{
        self,
        NameServiceHandle,
        ServiceDomain,
    },
};

#[cfg(target_arch = "aarch64")]
const ECHO_ELF: &[u8] = include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/echo.elf"));
#[cfg(target_arch = "aarch64")]
const CLIENT_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/client.elf"));
#[cfg(target_arch = "aarch64")]
const SERVICEMGR_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/servicemgr.elf"));

#[cfg(target_arch = "aarch64")]
const fn packed_name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

#[cfg(target_arch = "aarch64")]
const NAME_ECHO: u64 = packed_name(b"echo");
#[cfg(target_arch = "aarch64")]
const NAME_SVCMGR: u64 = packed_name(b"svcmgr");
#[cfg(target_arch = "aarch64")]
const OP_LOOKUP: u32 = 2;
#[cfg(target_arch = "aarch64")]
const OP_ECHO: u32 = 1;
#[cfg(target_arch = "aarch64")]
const OP_SHUTDOWN: u32 = 2;
#[cfg(target_arch = "aarch64")]
const ECHO_VALUE: u64 = 0x1234_5678;
#[cfg(target_arch = "aarch64")]
const CLIENT_SENTINEL: u32 = 0xc0de;

/// The kernel verifier acts as a second client through the direct kernel
/// API under this pseudo address-space id. It only exists in the IPC
/// capability registry.
#[cfg(target_arch = "aarch64")]
const KCLIENT_ASID: usize = 0x7100;
#[cfg(target_arch = "aarch64")]
#[cfg(target_arch = "aarch64")]
static mut TEST_STATE: Option<TestState> = None;

#[cfg(target_arch = "aarch64")]
struct TestState {
    name_service: NameServiceHandle,
    echo: Option<ServiceDomain>,
    client_config: PAddr,
}

pub fn test_el0_service() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 name service, bootstrap delivery, and service restart...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        let ns_tid = name_service.domain.tid;
        logln!("[service] using node name service (asid={}, tid={})", ns_asid, ns_tid);

        let echo =
            supervisor::spawn_with_name_service(ECHO_ELF, &name_service, ConnectionRights::CALL);
        let echo_asid = echo.asid;
        let echo_tid = echo.tid;
        logln!("[service] echo service spawned (asid={}, tid={})", echo_asid, echo_tid);

        let client =
            supervisor::spawn_with_name_service(CLIENT_ELF, &name_service, ConnectionRights::CALL);
        let client_asid = client.asid;
        let client_tid = client.tid;
        logln!("[service] client spawned (asid={}, tid={})", client_asid, client_tid);

        unsafe {
            TEST_STATE = Some(TestState {
                name_service,
                echo: Some(echo),
                client_config: client.status_frame,
            });
        }

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Service,
            verify_el0_service,
        );
        logln!("[service] verifier deferred");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 service test (AArch64 only).");
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn verify_persistent_upgrade(name_service: &NameServiceHandle) {
    while !crate::self_test::results::has_passed(crate::self_test::results::TestId::Service) {
        crate::cpu::scheduler::sleep_millis(1);
    }
    let client_asid = crate::service::loader::create_user_address_space();
    let ns = ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        client_asid,
        ConnectionRights::CALL,
    )
    .expect("[service] persistent-upgrade bootstrap failed");
    let echo_lookup =
        ipc::scalar_call(client_asid, ns, OP_LOOKUP, NAME_ECHO).expect("persistent echo lookup");
    let echo_reply = wait_reply_k2(client_asid, echo_lookup, "persistent echo generation");
    let old_generation = echo_reply.result;
    let old_connection = echo_reply.cap.expect("persistent echo connection");
    ipc::close_cap(client_asid, old_connection).expect("persistent echo connection close");

    let manager_lookup = ipc::scalar_call(client_asid, ns, OP_LOOKUP, NAME_SVCMGR)
        .expect("persistent manager lookup");
    let manager_reply = wait_reply_k2(client_asid, manager_lookup, "persistent manager reply");
    let manager = manager_reply.cap.expect("persistent manager connection");
    let upgrade =
        ipc::scalar_call(client_asid, manager, 1, NAME_ECHO).expect("persistent manager upgrade");
    let upgraded = wait_reply_k2(client_asid, upgrade, "persistent upgrade completion");
    assert!(upgraded.result > 0, "persistent ELF upgrade failed");

    let lookup =
        ipc::scalar_call(client_asid, ns, OP_LOOKUP, NAME_ECHO).expect("persistent replacement");
    let replacement = wait_reply_k2(client_asid, lookup, "persistent replacement lookup");
    assert_eq!(replacement.result, old_generation + 1, "persistent replacement generation");
    let connection = replacement.cap.expect("persistent replacement connection");
    let call = ipc::scalar_call(client_asid, connection, OP_ECHO, 0x51a5)
        .expect("persistent replacement call");
    assert_eq!(wait_reply_k2(client_asid, call, "persistent replacement echo").result, 0x51a5);
    crate::memory::close_user_address_space(client_asid)
        .expect("[service] persistent-upgrade client cleanup failed");
    logln!("[service] persistent NVMe ELF reload verified.");
}

#[cfg(target_arch = "aarch64")]
fn spin_until<F: FnMut() -> bool>(mut condition: F, what: &str) {
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    while !condition() {
        deadline.assert_pending(what);
        crate::cpu::scheduler::sleep_millis(1);
    }
}

/// Poll a pending call created through the direct kernel API until the EL0
/// server replies.
#[cfg(target_arch = "aarch64")]
fn wait_reply_k2(kclient_asid: usize, call: u64, what: &str) -> ipc::ReplyValue {
    let mut val = None;
    spin_until(
        || match ipc::poll_reply(kclient_asid, call) {
            Ok(Some(reply)) => {
                val = Some(reply);
                true
            }
            Ok(None) => false,
            Err(e) => panic!("[srv] K2 fail {}: {:?}", what, e),
        },
        what,
    );
    ipc::close_cap(kclient_asid, call).expect("K2 close");
    val.expect("K2 reply")
}

#[cfg(target_arch = "aarch64")]
fn wait_reply(call: u64, what: &str) -> ipc::ReplyValue {
    #[allow(unused_assignments)]
    #[allow(unused_assignments)]
    let mut value = None;
    spin_until(
        || match ipc::poll_reply(KCLIENT_ASID, call) {
            Ok(Some(reply)) => {
                value = Some(reply);
                true
            }
            Ok(None) => false,
            Err(error) => panic!("[service] poll_reply failed for {}: {:?}", what, error),
        },
        what,
    );
    ipc::close_cap(KCLIENT_ASID, call).expect("[service] pending-call close failed");
    value.expect("[service] reply value missing")
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_service() {
    let state = unsafe { TEST_STATE.as_mut() }.expect("[service] test state missing");

    // --- Phase A: the EL0 client completes bootstrap → lookup → call. ---
    let config: *const u32 = {
        let base: *mut u8 = state.client_config.into();
        base as *const u32
    };
    let ns_config: *const u32 = {
        let base: *mut u8 = state.name_service.domain.status_frame.into();
        base as *const u32
    };
    let echo_config: *const u32 = {
        let base: *mut u8 =
            state.echo.as_ref().expect("[service] echo domain missing").status_frame.into();
        base as *const u32
    };
    {
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(10_000);
        while unsafe { core::ptr::read_volatile(config) } != CLIENT_SENTINEL {
            spins += 1;
            if spins.is_multiple_of(1_000_000) {
                let ns_stage = unsafe { core::ptr::read_volatile(ns_config) };
                let ns_handled = unsafe { core::ptr::read_volatile(ns_config.add(1)) };
                let ns_opcode = unsafe { core::ptr::read_volatile(ns_config.add(2)) };
                let echo_stage = unsafe { core::ptr::read_volatile(echo_config) };
                let client_stage = unsafe { core::ptr::read_volatile(config.add(3)) };
                logln!(
                    "[service] waiting: ns stage {} handled {} opcode {}, echo stage {}, client \
                     stage {}",
                    ns_stage,
                    ns_handled,
                    ns_opcode,
                    echo_stage,
                    client_stage
                );
            }
            deadline.assert_pending("EL0 service client");
            crate::cpu::scheduler::sleep_millis(1);
        }
    }
    let echoed = unsafe { core::ptr::read_volatile(config.add(1)) };
    let generation = unsafe { core::ptr::read_volatile(config.add(2)) };
    assert_eq!(echoed, ECHO_VALUE as u32, "[service] client echoed value mismatch: {:#x}", echoed);
    assert_eq!(generation, 1, "[service] first echo instance must be generation 1");
    logln!("[service] EL0 client completed name lookup and echo call (generation 1)");

    // --- Phase B: restart semantics through the same EL0 name service. ---
    let ns_asid = state.name_service.domain.asid;
    let ns_endpoint = state.name_service.endpoint_cap;
    let kclient_conn =
        ipc::connection_delegate(ns_asid, ns_endpoint, KCLIENT_ASID, ConnectionRights::CALL)
            .expect("[service] verifier bootstrap connection failed");

    let lookup = ipc::scalar_call(KCLIENT_ASID, kclient_conn, OP_LOOKUP, NAME_ECHO)
        .expect("[service] verifier lookup call failed");
    let reply = wait_reply(lookup, "generation-1 lookup reply");
    assert_eq!(reply.result, 1, "[service] lookup should report generation 1");
    let stale_conn = reply.cap.expect("[service] lookup should return echo connection");
    logln!("[service] verifier got generation-1 connection");

    let shutdown = ipc::scalar_call(KCLIENT_ASID, stale_conn, OP_SHUTDOWN, 0)
        .expect("[service] echo shutdown call failed");
    let reply = wait_reply(shutdown, "echo shutdown reply");
    assert_eq!(reply.result, 0, "[service] echo shutdown should reply 0");
    logln!("[service] echo acknowledged shutdown");

    let echo1 = state.echo.take().expect("[service] echo domain handle missing");
    supervisor::wait_domain_exit(&echo1, 10_000);
    logln!("[service] echo generation 1 exited");
    // `wait_domain_exit` observes removal from the master thread table, which
    // occurs only after the thread has switched away and is safe to tear down.
    supervisor::teardown_domain(echo1);
    logln!("[service] echo generation 1 shut down and torn down");

    assert_eq!(
        ipc::scalar_call(KCLIENT_ASID, stale_conn, OP_ECHO, 1),
        Err(IpcError::EndpointClosed),
        "[service] stale connection to restarted service must fail EndpointClosed"
    );

    let echo2 =
        supervisor::spawn_with_name_service(ECHO_ELF, &state.name_service, ConnectionRights::CALL);
    let echo2_asid = echo2.asid;
    logln!("[service] echo service restarted (asid={})", echo2_asid);

    // Wait on the replacement's launch state instead of flooding the name
    // service with synchronous lookups while registration is still pending.
    let echo2_config: *const u32 = {
        let base: *mut u8 = echo2.status_frame.into();
        base as *const u32
    };
    spin_until(
        || unsafe {
            core::ptr::read_volatile(echo2_config) == 6
                && core::ptr::read_volatile(echo2_config.add(1)) == 2
        },
        "generation-2 registration",
    );
    let lookup = ipc::scalar_call(KCLIENT_ASID, kclient_conn, OP_LOOKUP, NAME_ECHO)
        .expect("[service] verifier re-lookup call failed");
    let reply = wait_reply(lookup, "post-restart lookup reply");
    assert_eq!(reply.result, 2, "[service] re-lookup should report generation 2");
    let fresh_conn = reply.cap.expect("[service] re-lookup should return connection");

    let call = ipc::scalar_call(KCLIENT_ASID, fresh_conn, OP_ECHO, 0xfeed)
        .expect("[service] generation-2 echo call failed");
    let reply = wait_reply(call, "generation-2 echo reply");
    assert_eq!(reply.result, 0xfeed, "[service] generation-2 echo mismatch");

    state.echo = Some(echo2);

    // --- live handoff (Phase D), initiated entirely by the EL0 manager. ---
    let service_manager = supervisor::spawn_service_manager(SERVICEMGR_ELF, &state.name_service);
    logln!(
        "[service] service manager spawned (asid={}, tid={})",
        service_manager.asid,
        service_manager.tid
    );
    let manager_status: *const u32 = {
        let base: *mut u8 = service_manager.status_frame.into();
        base as *const u32
    };
    spin_until(
        || unsafe { core::ptr::read_volatile(manager_status) == 3 },
        "service-manager registration",
    );
    let kclient2_asid = crate::service::loader::create_user_address_space();
    let ns2 = ipc::connection_delegate(
        state.name_service.domain.asid,
        state.name_service.endpoint_cap,
        kclient2_asid,
        ConnectionRights::CALL,
    )
    .expect("[service] K2 bootstrap failed");
    let lookup_mgr = ipc::scalar_call(kclient2_asid, ns2, OP_LOOKUP, NAME_SVCMGR)
        .expect("[service] service-manager lookup failed");
    let mgr_reply = wait_reply_k2(kclient2_asid, lookup_mgr, "service-manager lookup");
    let mgr_conn = mgr_reply.cap.expect("service-manager connection");

    // Opcode 1 asks the manager to upgrade the named service. The manager
    // performs OP_HANDOFF, receives the moved state object, and invokes the
    // authorized SpawnUpgrade syscall itself.
    let upgrade = ipc::scalar_call(kclient2_asid, mgr_conn, 1, NAME_ECHO)
        .expect("[service] manager upgrade call failed");
    let mut upgrade_value = None;
    let mut upgrade_polls = 0u64;
    spin_until(
        || {
            upgrade_polls += 1;
            if upgrade_polls.is_multiple_of(1_000) {
                logln!(
                    "[service] waiting for manager: stage={}, error={}",
                    unsafe { core::ptr::read_volatile(manager_status) },
                    unsafe { core::ptr::read_volatile(manager_status.add(2)) }
                );
            }
            match ipc::poll_reply(kclient2_asid, upgrade) {
                Ok(Some(reply)) => {
                    upgrade_value = Some(reply);
                    true
                }
                Ok(None) => false,
                Err(error) => panic!("[service] manager reply failed: {:?}", error),
            }
        },
        "manager upgrade reply",
    );
    ipc::close_cap(kclient2_asid, upgrade).expect("manager upgrade pending-call close");
    let upgrade_reply = upgrade_value.expect("manager upgrade reply missing");
    assert!(upgrade_reply.result > 0, "EL0 manager failed to spawn replacement");
    let replacement_asid = upgrade_reply.result as usize;

    let e2 = state.echo.take().unwrap();
    supervisor::wait_domain_exit(&e2, 10_000);
    supervisor::teardown_domain(e2);
    logln!("[service] EL0 manager spawned generation-3 echo (asid={})", replacement_asid);

    let l3 = ipc::scalar_call(kclient2_asid, ns2, OP_LOOKUP, NAME_ECHO).expect("gen-3 lookup");
    let lookup3 = wait_reply_k2(kclient2_asid, l3, "gen-3 lookup reply");
    assert_eq!(lookup3.result, 3, "gen-3 lookup generation");
    let f3 = lookup3.cap.expect("gen-3 connection");
    let c3 = ipc::scalar_call(kclient2_asid, f3, OP_ECHO, 0x99).expect("gen-3 call");
    let r3 = wait_reply_k2(kclient2_asid, c3, "gen-3 echo");
    assert_eq!(r3.result, 0x99, "gen-3 mismatch");
    crate::memory::close_user_address_space(kclient2_asid)
        .expect("[service] K2 address-space close failed");
    logln!("[service] live handoff verified");

    ipc::close_address_space(KCLIENT_ASID);
    logln!(
        "[service] SUCCESS: bootstrap delivery, name lookup, stale-connection failure, and \
         restart generation all verified."
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::Service);
}
