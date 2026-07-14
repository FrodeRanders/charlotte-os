//! Self-test: Phase 3 userspace name service and service manager.
//!
//! Spawns three isolated EL0 protection domains from Rust-compiled ELFs:
//!
//! - `ns.elf` — the userspace name service (registry endpoint created and
//!   delivered by the supervisor through the bootstrap slot);
//! - `echo.elf` — a service that creates its own endpoint and registers it
//!   by name, attaching a re-delegable connection at call time;
//! - `client.elf` — a client that looks up "echo" by name and calls it
//!   through the returned connection.
//!
//! No domain ever learns another domain's ASID, LP, or kernel object ids;
//! all authority flows through delegated capabilities.
//!
//! A kernel verifier thread then exercises restart semantics through the
//! same EL0 name service: it shuts the echo service down, tears down its
//! domain, observes that stale connections fail with `EndpointClosed`,
//! restarts the service, and observes the instance generation increment.

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
use crate::logln;

#[cfg(target_arch = "aarch64")]
const NS_ELF: &[u8] = include_bytes!("ns.elf");
#[cfg(target_arch = "aarch64")]
const ECHO_ELF: &[u8] = include_bytes!("echo.elf");
#[cfg(target_arch = "aarch64")]
const CLIENT_ELF: &[u8] = include_bytes!("client.elf");

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
const NS_INTERFACE: u64 = packed_name(b"NAME");
#[cfg(target_arch = "aarch64")]
const NAME_ECHO: u64 = packed_name(b"echo");
#[cfg(target_arch = "aarch64")]
const OP_LOOKUP: u32 = 2;
#[cfg(target_arch = "aarch64")]
const OP_ECHO: u32 = 1;
#[cfg(target_arch = "aarch64")]
const OP_SHUTDOWN: u32 = 2;
#[cfg(target_arch = "aarch64")]
const ECHO_VALUE: u64 = 0x1234_5678;
#[cfg(target_arch = "aarch64")]
const CLIENT_SENTINEL: u32 = 0xC0DE;

/// The kernel verifier acts as a second client through the direct kernel
/// API under this pseudo address-space id. It only exists in the IPC
/// capability registry.
#[cfg(target_arch = "aarch64")]
const KCLIENT_ASID: usize = 0x7100;

#[cfg(target_arch = "aarch64")]
const MAX_SPINS: u64 = 80_000_000;

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

        let name_service = supervisor::spawn_name_service(NS_ELF, NS_INTERFACE, 1, 8);
        let ns_asid = name_service.domain.asid;
        let ns_tid = name_service.domain.tid;
        logln!("[service] name service spawned (asid={}, tid={})", ns_asid, ns_tid);

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
                client_config: client.config_frame,
            });
        }

        let _vtid = crate::cpu::scheduler::spawn_thread(
            crate::memory::KERNEL_ASID,
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
fn spin_until<F: FnMut() -> bool>(mut condition: F, what: &str) {
    use crate::cpu::scheduler::yield_lp;

    let mut spins: u64 = 0;
    while !condition() {
        spins += 1;
        assert!(spins < MAX_SPINS, "[service] FAILED waiting for {}", what);
        yield_lp();
    }
}

/// Poll a pending call created through the direct kernel API until the EL0
/// server replies.
#[cfg(target_arch = "aarch64")]
fn wait_reply(call: u64, what: &str) -> ipc::ReplyValue {
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
    use crate::cpu::scheduler::yield_lp;

    let state = unsafe { TEST_STATE.as_mut() }.expect("[service] test state missing");

    // --- Phase A: the EL0 client completes bootstrap → lookup → call. ---
    let config: *const u32 = {
        let base: *mut u8 = state.client_config.into();
        base as *const u32
    };
    let ns_config: *const u32 = {
        let base: *mut u8 = state.name_service.domain.config_frame.into();
        base as *const u32
    };
    let echo_config: *const u32 = {
        let base: *mut u8 =
            state.echo.as_ref().expect("[service] echo domain missing").config_frame.into();
        base as *const u32
    };
    {
        let mut spins: u64 = 0;
        while unsafe { core::ptr::read_volatile(config) } != CLIENT_SENTINEL {
            spins += 1;
            if spins % 1_000_000 == 0 {
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
            if spins >= MAX_SPINS {
                panic!("[service] FAILED waiting for EL0 client");
            }
            yield_lp();
        }
    }
    let echoed = unsafe { core::ptr::read_volatile(config.add(1)) };
    let generation = unsafe { core::ptr::read_volatile(config.add(2)) };
    assert_eq!(
        echoed, ECHO_VALUE as u32,
        "[service] client echoed value mismatch: {:#x}",
        echoed
    );
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
    supervisor::wait_domain_exit(&echo1, MAX_SPINS);
    logln!("[service] echo generation 1 exited");
    // Give the exiting thread time to be switched out and reaped before its
    // address space is torn down.
    for _ in 0..1_000 {
        yield_lp();
    }
    supervisor::teardown_domain(echo1);
    logln!("[service] echo generation 1 shut down and torn down");

    assert_eq!(
        ipc::scalar_call(KCLIENT_ASID, stale_conn, OP_ECHO, 1),
        Err(IpcError::EndpointClosed),
        "[service] stale connection to restarted service must fail EndpointClosed"
    );

    let echo2 = supervisor::spawn_with_name_service(
        ECHO_ELF,
        &state.name_service,
        ConnectionRights::CALL,
    );
    let echo2_asid = echo2.asid;
    logln!("[service] echo service restarted (asid={})", echo2_asid);

    // Re-lookup until the restarted instance registers with generation 2.
    let mut fresh_conn = 0u64;
    spin_until(
        || {
            let lookup = ipc::scalar_call(KCLIENT_ASID, kclient_conn, OP_LOOKUP, NAME_ECHO)
                .expect("[service] verifier re-lookup call failed");
            let reply = wait_reply(lookup, "post-restart lookup reply");
            if reply.result == 2 {
                fresh_conn = reply.cap.expect("[service] re-lookup should return connection");
                true
            } else {
                if let Some(cap) = reply.cap {
                    let _ = ipc::close_cap(KCLIENT_ASID, cap);
                }
                false
            }
        },
        "generation-2 registration",
    );

    let call = ipc::scalar_call(KCLIENT_ASID, fresh_conn, OP_ECHO, 0xfeed)
        .expect("[service] generation-2 echo call failed");
    let reply = wait_reply(call, "generation-2 echo reply");
    assert_eq!(reply.result, 0xfeed, "[service] generation-2 echo mismatch");

    state.echo = Some(echo2);
    ipc::close_address_space(KCLIENT_ASID);
    logln!(
        "[service] SUCCESS: bootstrap delivery, name lookup, stale-connection failure, and \
         restart generation all verified."
    );
    loop {
        yield_lp();
    }
}
