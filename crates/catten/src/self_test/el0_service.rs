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

const fn packed_name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

const NAME_ECHO: u64 = packed_name(b"echo");
const NAME_SVCMGR: u64 = packed_name(b"svcmgr");
const OP_LOOKUP: u32 = 2;
const OP_ECHO: u32 = 1;
const OP_SHUTDOWN: u32 = 2;
const ECHO_VALUE: u64 = 0x1234_5678;
const CLIENT_SENTINEL: u32 = 0xc0de;

/// The kernel verifier acts as a second client through the direct kernel
/// API under this pseudo address-space id. It only exists in the IPC
/// capability registry.
const KCLIENT_ASID: usize = 0x7100;
static mut TEST_STATE: Option<TestState> = None;

struct TestState {
    name_service: NameServiceHandle,
    echo: Option<ServiceDomain>,
    client_config: PAddr,
    client_asid: usize,
    client_tid: usize,
    client_generation: u64,
}

pub fn test_el0_service() {
        logln!("Testing EL0 name service, bootstrap delivery, and service restart...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        let ns_tid = name_service.domain.tid;
        logln!("[service] using node name service (asid={}, tid={})", ns_asid, ns_tid);

        let echo = supervisor::spawn_with_name_service(
            crate::service::store::service_elf(b"echo").expect("[el0_service] echo.elf"),
            &name_service,
            ConnectionRights::CALL,
        );
        let echo_asid = echo.asid;
        let echo_tid = echo.tid;
        logln!("[service] echo service spawned (asid={}, tid={})", echo_asid, echo_tid);

        let client = supervisor::spawn_with_name_service(
            crate::service::store::service_elf(b"client").expect("[el0_service] client.elf"),
            &name_service,
            ConnectionRights::CALL,
        );
        let client_asid = client.asid;
        let client_tid = client.tid;
        let client_generation = client.generation;
        logln!("[service] client spawned (asid={}, tid={})", client_asid, client_tid);

        unsafe {
            TEST_STATE = Some(TestState {
                name_service,
                echo: Some(echo),
                client_config: client.status_frame,
                client_asid,
                client_tid,
                client_generation,
            });
        }

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Service,
            verify_el0_service,
        );
        logln!("[service] verifier deferred");
}

pub(crate) fn verify_persistent_upgrade(name_service: &NameServiceHandle) {
    let passed = crate::self_test::results::wait_until_resolved(
        crate::self_test::results::TestId::Service,
        30_000,
    );
    assert!(passed, "EL0 persistent upgrade waiting for service-lifecycle test");
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
    let ns_status: *const u32 = {
        let base: *mut u8 = name_service.domain.status_frame.into();
        base as *const u32
    };
    let ready = ipc::wait_reply_timeout(client_asid, upgrade, 30_000)
        .expect("[service] persistent upgrade reply failed");
    assert!(ready, "persistent upgrade completion deadline expired");
    logln!(
        "[service] persistent upgrade completed: ns_waiters={}, ns_handled={}",
        unsafe { core::ptr::read_volatile(ns_status.add(12)) },
        unsafe { core::ptr::read_volatile(ns_status.add(4)) }
    );
    let upgraded = ipc::poll_reply(client_asid, upgrade)
        .expect("[service] persistent upgrade poll failed")
        .expect("[service] persistent upgrade reply missing");
    ipc::close_cap(client_asid, upgrade).expect("persistent upgrade pending-call close");
    assert!(upgraded.result > 0, "persistent ELF upgrade failed");

    let lookup =
        ipc::scalar_call(client_asid, ns, OP_LOOKUP, NAME_ECHO).expect("persistent replacement");
    let replacement = wait_reply_k2(client_asid, lookup, "persistent replacement lookup");
    assert_eq!(replacement.result, old_generation + 1, "persistent replacement generation");
    let connection = replacement.cap.expect("persistent replacement connection");
    let call = ipc::scalar_call(client_asid, connection, OP_ECHO, 0x51a5)
        .expect("persistent replacement call");
    assert_eq!(wait_reply_k2(client_asid, call, "persistent replacement echo").result, 0x51a5);
    crate::self_test::close_test_address_space(client_asid)
        .expect("[service] persistent-upgrade client cleanup failed");
    logln!("[service] persistent NVMe ELF reload verified.");
}

/// Block on a pending call created through the direct kernel API until the
/// EL0 server replies (event-driven, with a deadline watchdog).
fn wait_reply_k2(kclient_asid: usize, call: u64, what: &str) -> ipc::ReplyValue {
    let ready = ipc::wait_reply_timeout(kclient_asid, call, 30_000)
        .unwrap_or_else(|e| panic!("[srv] K2 fail {}: {:?}", what, e));
    assert!(ready, "[srv] K2 deadline expired waiting for {}", what);
    let val = ipc::poll_reply(kclient_asid, call)
        .expect("[srv] K2 poll failed")
        .expect("[srv] K2 reply missing");
    ipc::close_cap(kclient_asid, call).expect("K2 close");
    val
}

/// Repeatedly look up `name` through the name service until the registry
/// reports `expected_generation`. A lookup racing a re-registration can
/// resolve the previous generation's stale entry, so the lookup is retried
/// with a parked sleep between attempts. Returns the pending-call cap of the
/// successful lookup (reply not yet drained). Panics on timeout.
fn lookup_until_generation(
    caller_asid: usize,
    ns_conn: u64,
    name: u64,
    expected_generation: i64,
    what: &str,
) -> u64 {
    let deadline = crate::self_test::results::Deadline::after_millis(30_000);
    loop {
        let lookup = ipc::scalar_call(caller_asid, ns_conn, OP_LOOKUP, name)
            .unwrap_or_else(|e| panic!("[service] lookup failed for {}: {:?}", what, e));
        let ready = ipc::wait_reply_timeout(caller_asid, lookup, 30_000)
            .unwrap_or_else(|e| panic!("[service] lookup reply failed for {}: {:?}", what, e));
        assert!(ready, "[service] deadline expired waiting for {}", what);
        let reply = ipc::poll_reply(caller_asid, lookup)
            .expect("[service] lookup poll failed")
            .expect("[service] lookup reply missing");
        if reply.result == expected_generation {
            return lookup;
        }
        // Stale generation still registered; the replacement has not landed
        // yet. Park briefly (blocking sleep) and retry.
        if let Some(connection) = reply.cap {
            let _ = ipc::close_cap(caller_asid, connection);
        }
        ipc::close_cap(caller_asid, lookup).expect("[service] stale lookup close");
        crate::cpu::scheduler::sleep_millis(10);
        deadline.assert_pending(what);
    }
}

fn wait_reply(call: u64, what: &str) -> ipc::ReplyValue {
    let ready = ipc::wait_reply_timeout(KCLIENT_ASID, call, 30_000)
        .unwrap_or_else(|error| panic!("[service] wait_reply failed for {}: {:?}", what, error));
    assert!(ready, "[service] deadline expired waiting for {}", what);
    let value = ipc::poll_reply(KCLIENT_ASID, call)
        .expect("[service] poll_reply failed")
        .expect("[service] reply value missing");
    ipc::close_cap(KCLIENT_ASID, call).expect("[service] pending-call close failed");
    value
}

extern "C" fn verify_el0_service() {
    let state = unsafe { TEST_STATE.as_mut() }.expect("[service] test state missing");

    // --- Phase A: the EL0 client completes bootstrap → lookup → call. ---
    let config: *const u32 = {
        let base: *mut u8 = state.client_config.into();
        base as *const u32
    };
    {
        // The client writes its completion sentinel and then exits; its
        // thread exit is the completion event, so block on that instead of
        // polling the shared status frame.
        let client_exit = crate::completion::observe_thread_exit_with_generation(
            state.client_asid,
            state.client_tid,
            Some(state.client_generation),
        )
        .expect("[service] client exit observer");
        let exited = crate::completion::wait_timeout(state.client_asid, client_exit, 10_000)
            .expect("[service] client exit wait error");
        assert!(exited, "[service] EL0 service client did not exit within deadline");
    }
    assert_eq!(
        unsafe { core::ptr::read_volatile(config) },
        CLIENT_SENTINEL,
        "[service] client did not reach completion sentinel"
    );
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

    let echo2 = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"echo").expect("[el0_service] echo.elf"),
        &state.name_service,
        ConnectionRights::CALL,
    );
    let echo2_asid = echo2.asid;
    logln!("[service] echo service restarted (asid={})", echo2_asid);

    // The name service replaces the old generation's registry entry when the
    // replacement registers, but a lookup that races the replacement can
    // still resolve the stale entry. Retry with a parked sleep between
    // attempts until the registry reports generation 2 — each attempt blocks
    // on the lookup reply (event-driven) rather than busy-polling.
    let lookup =
        lookup_until_generation(KCLIENT_ASID, kclient_conn, NAME_ECHO, 2, "post-restart lookup");
    let reply = ipc::poll_reply(KCLIENT_ASID, lookup)
        .expect("[service] post-restart poll failed")
        .expect("[service] post-restart reply missing");
    ipc::close_cap(KCLIENT_ASID, lookup).expect("[service] post-restart lookup close");
    let fresh_conn = reply.cap.expect("[service] re-lookup should return connection");

    let call = ipc::scalar_call(KCLIENT_ASID, fresh_conn, OP_ECHO, 0xfeed)
        .expect("[service] generation-2 echo call failed");
    let reply = wait_reply(call, "generation-2 echo reply");
    assert_eq!(reply.result, 0xfeed, "[service] generation-2 echo mismatch");

    state.echo = Some(echo2);

    // --- live handoff (Phase D), initiated entirely by the EL0 manager. ---
    let service_manager = supervisor::spawn_service_manager(
        crate::service::store::service_elf(b"servicemgr").expect("[el0_service] servicemgr.elf"),
        &state.name_service,
    );
    logln!(
        "[service] service manager spawned (asid={}, tid={})",
        service_manager.asid,
        service_manager.tid
    );
    // The manager registers with the name service during its bootstrap; the
    // deferred lookup below is the registration event.
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
    let mgr_status: *const u32 = {
        let base: *mut u8 = service_manager.status_frame.into();
        base as *const u32
    };
    let upgrade_ready = ipc::wait_reply_timeout(kclient2_asid, upgrade, 30_000)
        .expect("[service] manager upgrade reply failed");
    assert!(upgrade_ready, "[service] manager upgrade reply deadline expired");
    logln!(
        "[service] manager upgrade replied: stage={}, error={}",
        unsafe { core::ptr::read_volatile(mgr_status) },
        unsafe { core::ptr::read_volatile(mgr_status.add(2)) }
    );
    let upgrade_reply = ipc::poll_reply(kclient2_asid, upgrade)
        .expect("[service] manager upgrade poll failed")
        .expect("[service] manager upgrade reply missing");
    ipc::close_cap(kclient2_asid, upgrade).expect("manager upgrade pending-call close");
    assert!(upgrade_reply.result > 0, "EL0 manager failed to spawn replacement");
    let replacement_asid = upgrade_reply.result as usize;

    let e2 = state.echo.take().unwrap();
    supervisor::wait_domain_exit(&e2, 10_000);
    supervisor::teardown_domain(e2);
    logln!("[service] EL0 manager spawned generation-3 echo (asid={})", replacement_asid);

    let l3 = lookup_until_generation(kclient2_asid, ns2, NAME_ECHO, 3, "gen-3 lookup");
    let lookup3 = ipc::poll_reply(kclient2_asid, l3)
        .expect("[service] gen-3 lookup poll failed")
        .expect("[service] gen-3 lookup reply missing");
    ipc::close_cap(kclient2_asid, l3).expect("[service] gen-3 lookup close");
    let f3 = lookup3.cap.expect("gen-3 connection");
    let c3 = ipc::scalar_call(kclient2_asid, f3, OP_ECHO, 0x99).expect("gen-3 call");
    let r3 = wait_reply_k2(kclient2_asid, c3, "gen-3 echo");
    assert_eq!(r3.result, 0x99, "gen-3 mismatch");
    crate::self_test::close_test_address_space(kclient2_asid)
        .expect("[service] K2 address-space close failed");
    logln!("[service] live handoff verified");

    ipc::close_address_space(KCLIENT_ASID);
    logln!(
        "[service] SUCCESS: bootstrap delivery, name lookup, stale-connection failure, and \
         restart generation all verified."
    );
    crate::self_test::results::pass(crate::self_test::results::TestId::Service);
}
