//! Self-test: cluster administration service (`clusterctl`) and the serial
//! admin console.
//!
//! Observes the normal `clusterctl` EL0 service, which wraps the dns manifest
//! ops and the object store behind admin operations. The verifier drives the
//! programmatic flow through the clusterctl endpoint: upload a signed artifact
//! ("agent"), reject retired deployment operations, verify full-name query handling, and run the
//! key ceremony that commits the cluster's public key to replicated state. A kernel
//! thread then runs the interactive serial console (commands: `help`,
//! `upload`, `status`) on top of the same service.
mod inner {
    use alloc::vec::Vec;

    use crate::{
        ipc::{
            self,
            ConnectionRights,
        },
        logln,
        memory::KERNEL_ASID,
        service::supervisor::{
            self,
            NameServiceHandle,
        },
    };

    // "ctl" packed LE; catten_services::clusterctl.
    const CTL_NAME: u64 = 0x0000_0000_006c_7463;
    const CTL_OP_UPLOAD: u32 = 1;
    const RETIRED_CTL_DEPLOY: u32 = 2;
    const CTL_OP_STATUS: u32 = 3;
    const CTL_OP_KEYCEREMONY: u32 = 4;
    const CTL_OP_KEY: u32 = 5;
    // "agent" packed LE. This artifact is independently signed as `agent`;
    // using greet bytes under an alias would correctly fail CLS2 identity
    // binding and is no longer a valid upload test.
    const TEST_ARTIFACT_NAME: u64 = 0x0000_0074_6e65_6761;
    const CTL_STAGE_SERVING: u32 = 6;
    // ns opcodes.
    const NS_OP_LOOKUP: u32 = 2;

    static mut TEST_STATE: Option<NameServiceHandle> = None;
    /// The kernel-side connection to the clusterctl service.
    static mut CTL_CONN: u64 = 0;

    fn kernel_ns_connection(ns: &NameServiceHandle) -> u64 {
        ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            KERNEL_ASID,
            ConnectionRights::CALL,
        )
        .expect("[clusterctl] kernel name-service connection")
    }

    fn lookup_service(kernel_ns: u64, name: u64) -> Option<u64> {
        let call = ipc::scalar_call(KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name).ok()?;
        if ipc::wait_reply(KERNEL_ASID, call).is_err() {
            let _ = ipc::close_cap(KERNEL_ASID, call);
            return None;
        }
        let connection = ipc::poll_reply(KERNEL_ASID, call)
            .ok()
            .flatten()
            .and_then(|reply| reply.cap)
            .filter(|cap| *cap != 0);
        let _ = ipc::close_cap(KERNEL_ASID, call);
        connection
    }

    /// Scalar call with a moved memory object; returns the reply result and
    /// any returned memory cap (for `OP_STATUS`).
    fn call_with_memory_reply(
        kernel_conn: u64,
        opcode: u32,
        arg0: u64,
        bytes: &[u8],
    ) -> Option<(i64, Option<u64>)> {
        let mem = crate::memory::object::allocate_with_bytes(KERNEL_ASID, bytes).ok()?;
        let call =
            match ipc::scalar_call_with_memory_move(KERNEL_ASID, kernel_conn, opcode, arg0, mem) {
                Ok(call) => call,
                Err(_) => {
                    let _ = crate::memory::object::close_cap(KERNEL_ASID, mem);
                    return None;
                }
            };
        if ipc::wait_reply(KERNEL_ASID, call).is_err() {
            let _ = ipc::close_cap(KERNEL_ASID, call);
            return None;
        }
        let reply = ipc::poll_reply(KERNEL_ASID, call).ok().flatten();
        let result = reply.map(|reply| {
            if let Some(connection) = reply.cap {
                let _ = ipc::close_cap(KERNEL_ASID, connection);
            }
            (reply.result, reply.memory)
        });
        let _ = ipc::close_cap(KERNEL_ASID, call);
        result
    }

    /// Raw boot-fixture adapter: copy exact readable reply bytes through the
    /// existing kernel snapshot boundary, then release the uniquely received
    /// capability. Public application mapping requires a user-root operation
    /// lease and intentionally rejects KERNEL_ASID; no fixture mapping bypass
    /// or fixed low-address scratch reservation is needed here.
    fn read_moved_memory(
        cap: u64,
        len: usize,
    ) -> Result<Vec<u8>, crate::memory::object::MemoryObjectError> {
        let bytes = crate::memory::object::snapshot_bytes(KERNEL_ASID, cap, len);
        let closed = crate::memory::object::close_cap(KERNEL_ASID, cap);
        closed?;
        bytes
    }

    fn status_word(base: *const u8, offset: usize) -> u32 {
        unsafe { core::ptr::read_volatile(base.add(offset).cast::<u32>()) }
    }

    /// Pack a short name into a little-endian u64 (mirrors the userspace
    /// `catten_services::name` helper).
    fn pack_name(name: &[u8]) -> u64 {
        let mut packed = 0u64;
        for (index, byte) in name.iter().copied().enumerate().take(8) {
            packed |= u64::from(byte) << (8 * index);
        }
        packed
    }

    pub fn test_el0_clusterctl() {
        logln!("Testing cluster administration service (clusterctl + serial console)...");
        let name_service = supervisor::node_name_service();
        unsafe { TEST_STATE = Some(name_service) };
        let _verifier = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Clusterctl,
            verify_el0_clusterctl,
        );
        logln!(
            "[clusterctl] verifier deferred (drives upload/rejection/status; console on serial)"
        );
    }

    extern "C" fn verify_el0_clusterctl() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { TEST_STATE.as_ref() }.expect("[clusterctl] test state missing");
        let kernel_ns = kernel_ns_connection(ns);

        let ctl = crate::service::launch::steady_state()
            .deployment
            .expect("[clusterctl] operational deployment plane missing")
            .clusterctl;
        logln!("[clusterctl] observing clusterctl (asid={})", ctl.asid);
        let ctl_cfg: *const u8 = {
            let base: *mut u8 = ctl.status_frame.into();
            base
        };
        let bootstrap_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        while status_word(ctl_cfg, charlotte_launch::clusterctl_status::STAGE) < CTL_STAGE_SERVING {
            bootstrap_deadline.assert_pending("EL0 clusterctl local bootstrap");
            yield_lp();
        }
        logln!("[clusterctl] clusterctl reached serving stage.");

        // Resolve the service connection for both the programmatic flow and
        // the serial console.
        let mut ctl_conn = 0u64;
        let lookup_deadline = crate::self_test::results::Deadline::after_millis(30_000);
        while ctl_conn == 0 {
            ctl_conn = lookup_service(kernel_ns, CTL_NAME).unwrap_or(0);
            if ctl_conn == 0 {
                lookup_deadline.assert_pending("EL0 clusterctl registration");
                yield_lp();
            }
        }
        unsafe { CTL_CONN = ctl_conn };

        // --- Programmatic flow through clusterctl ---
        // 1. A valid signature for the wrong logical name is not sufficient: ingress must reject
        //    substitution before touching the store.
        let wrong_artifact = crate::service::store::service_elf(b"greet")
            .expect("[clusterctl] greet substitution test artifact");
        let mut wrong_request = Vec::with_capacity(8 + wrong_artifact.len());
        wrong_request.extend_from_slice(&(wrong_artifact.len() as u64).to_le_bytes());
        wrong_request.extend_from_slice(wrong_artifact);
        let rejected =
            call_with_memory_reply(ctl_conn, CTL_OP_UPLOAD, TEST_ARTIFACT_NAME, &wrong_request);
        assert_eq!(
            rejected.map(|(result, _)| result),
            Some(-11),
            "[clusterctl] a greet-signed ELF must not be accepted as agent"
        );

        // Upload an artifact whose CLS2 metadata blesses this exact name.
        let test_artifact =
            crate::service::store::service_elf(b"agent").expect("[clusterctl] agent artifact");
        let expected_object_id = charlotte_launch::artifact_object_id(b"agent");
        let mut request = Vec::with_capacity(8 + test_artifact.len());
        request.extend_from_slice(&(test_artifact.len() as u64).to_le_bytes());
        request.extend_from_slice(test_artifact);
        let uploaded =
            call_with_memory_reply(ctl_conn, CTL_OP_UPLOAD, TEST_ARTIFACT_NAME, &request);
        logln!("[clusterctl] upload result = {uploaded:?}");
        assert_eq!(
            uploaded.map(|(result, _)| result),
            Some(expected_object_id as i64),
            "[clusterctl] upload must return the derived artifact object id"
        );

        // Retired deployment opcodes reject before hashing, consensus or
        // placement. A tiny payload suffices; there is no old request writer.
        assert_eq!(
            call_with_memory_reply(ctl_conn, RETIRED_CTL_DEPLOY, 0, b"retired")
                .map(|(result, _)| result),
            Some(-1),
        );
        let dns = lookup_service(kernel_ns, 0x0073_6e64).expect("[clusterctl] DNS connection");
        for retired_opcode in [8, 9] {
            assert_eq!(
                call_with_memory_reply(dns, retired_opcode, 0, b"retired")
                    .map(|(result, _)| result),
                Some(-3),
                "[clusterctl] retired DNS opcode {retired_opcode}",
            );
        }
        let _ = ipc::close_cap(KERNEL_ASID, dns);
        for name in [b"agent".as_slice(), b"not-a-deployment-long-name"] {
            assert_eq!(
                call_with_memory_reply(ctl_conn, CTL_OP_STATUS, name.len() as u64, name)
                    .map(|(result, _)| result),
                Some(-1),
            );
        }
        for (len, bytes) in [(0, b"agent".as_slice()), (5, b"bad x"), (129, b"agent")] {
            assert_eq!(
                call_with_memory_reply(ctl_conn, CTL_OP_STATUS, len, bytes)
                    .map(|(result, _)| result),
                Some(-3),
                "[clusterctl] malformed full-name query",
            );
        }
        logln!(
            "[clusterctl] retired opcodes reject; full-name status accepts long names and rejects \
             malformed names"
        );

        // --- Key ceremony ---
        // Commit the cluster's public key to the replicated state (the
        // manual establishment path) and read it back, exactly as a joining
        // node would: the key is obtained from the cluster, not from a
        // channel out of band. Best-effort like the deploy: the ceremony
        // commits through the leader's dns; the read gate below polls every
        // replica's locally applied state until it sees the key.
        let key_deadline = crate::self_test::results::Deadline::after_millis(120_000);
        let mut ceremony_committed = false;
        loop {
            if !ceremony_committed
                && let Some((generation, _)) = call_with_memory_reply(
                    ctl_conn,
                    CTL_OP_KEYCEREMONY,
                    0,
                    &charlotte_launch::CLUSTER_PUBLIC_KEY,
                )
                && generation >= 1
            {
                ceremony_committed = true;
                logln!("[clusterctl] key ceremony committed (generation {generation})");
            }
            let Some((status, memory)) = call_with_memory_reply(ctl_conn, CTL_OP_KEY, 0, &[])
            else {
                key_deadline.assert_pending("EL0 clusterctl key ceremony");
                yield_lp();
                continue;
            };
            if let Some(memory) = memory.filter(|_| status >= 0) {
                let key = read_moved_memory(memory, 32).expect("[clusterctl] readable cluster key");
                assert_eq!(
                    key.as_slice(),
                    &charlotte_launch::CLUSTER_PUBLIC_KEY,
                    "[clusterctl] the committed cluster key must match the injected key"
                );
                logln!("[clusterctl] cluster key committed and replicated across the cluster");
                break;
            }
            key_deadline.assert_pending("EL0 clusterctl key ceremony replication");
            yield_lp();
        }

        // --- Serial admin console ---
        // The console thread reads commands from the platform serial RX FIFO
        // and calls the same clusterctl service.
        crate::cpu::scheduler::spawn_thread(KERNEL_ASID, console_entry);
        logln!(
            "[clusterctl] SUCCESS: clusterctl verified upload, rejected retired opcodes and \
             malformed queries, and completed key ceremony; serial console listening."
        );
        crate::self_test::results::pass(crate::self_test::results::TestId::Clusterctl);
    }

    // ---- serial admin console ------------------------------------------------

    /// Prompt printed at the start of each console line.
    const CONSOLE_PROMPT: &str = "\r\n[admin] ";

    /// The console thread entry: read serial lines and dispatch commands to
    /// the clusterctl service.
    extern "C" fn console_entry() {
        logln!("[clusterctl] admin console ready (help for commands).");
        crate::log::serial::SERIAL.lock().write_bytes(CONSOLE_PROMPT);

        let mut line: Vec<u8> = Vec::new();
        loop {
            let byte = crate::log::serial::SERIAL.lock().try_get_byte();
            match byte {
                Some(byte) if byte == b'\r' || byte == b'\n' => {
                    if !line.is_empty() {
                        logln!("[admin] {}", core::str::from_utf8(&line).unwrap_or("<binary>"));
                        run_console_command(&line);
                        line.clear();
                        crate::log::serial::SERIAL.lock().write_bytes(CONSOLE_PROMPT);
                    }
                }
                Some(byte) if byte == 0x7f || byte == 0x08 => {
                    line.pop();
                }
                Some(byte) => {
                    line.push(byte);
                }
                // The early PL011 console has no delegated RX interrupt. Do
                // not leave this optional administration thread permanently
                // Ready: that would consume a CPU in an otherwise idle
                // system and contend with the serial log sink. A short
                // timer-backed sleep gives interactive input acceptable
                // latency while allowing the LP to enter its idle path.
                None => crate::cpu::scheduler::sleep_millis(10),
            }
        }
    }

    fn run_console_command(line: &[u8]) {
        let mut parts =
            line.split(|byte| *byte == b' ' || *byte == b'\t').filter(|part| !part.is_empty());
        match parts.next() {
            Some(b"help") => {
                logln!(
                    "[admin] commands: upload <short-name> <payload> | status <full-name>; signed \
                     deployment: cluster-sign deployment-notify"
                );
            }
            Some(b"upload") => {
                let name = parts.next();
                let payload = parts.next();
                let Some(name) = name else {
                    logln!("[admin] usage: upload <name> [<payload>]");
                    return;
                };
                if name.is_empty() || name.len() > 8 {
                    logln!("[admin] upload requires a name of 1 to 8 bytes");
                    return;
                }
                // `upload greet` (no payload) re-stages the pre-signed greet
                // artifact; anything else stores the given bytes as-is, so
                // only a payload signed off-cluster will validate at pickup.
                let artifact = match (name, payload) {
                    (b"greet", None) => crate::service::store::service_elf(b"greet")
                        .expect("[admin] greet artifact in object store"),
                    (_, None) => {
                        logln!(
                            "[admin] usage: upload <name> [<payload>] (no signed artifact for {})",
                            core::str::from_utf8(name).unwrap_or("?")
                        );
                        return;
                    }
                    (_, Some(payload)) => payload,
                };
                let mut request = Vec::with_capacity(8 + artifact.len());
                request.extend_from_slice(&(artifact.len() as u64).to_le_bytes());
                request.extend_from_slice(artifact);
                let conn = unsafe { CTL_CONN };
                let result = call_with_memory_reply(conn, CTL_OP_UPLOAD, pack_name(name), &request);
                match result.map(|(result, _)| result) {
                    Some(object_id) if object_id >= 0 => {
                        logln!(
                            "[admin] uploaded {} at object {object_id:#x}",
                            core::str::from_utf8(name).unwrap_or("?")
                        )
                    }
                    Some(error) => logln!("[admin] upload failed ({error})"),
                    None => logln!("[admin] upload call failed"),
                }
            }
            Some(b"status") => {
                let name = parts.next();
                let Some(name) = name else {
                    logln!("[admin] usage: status <name>");
                    return;
                };
                let conn = unsafe { CTL_CONN };
                let Some((status, memory)) =
                    call_with_memory_reply(conn, CTL_OP_STATUS, name.len() as u64, name)
                else {
                    logln!("[admin] status call failed");
                    return;
                };
                let Some(memory) = memory else {
                    logln!("[admin] status: {status}");
                    return;
                };
                let bytes = match read_moved_memory(memory, 24) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        logln!("[admin] status read failed: {error:?}");
                        return;
                    }
                };
                let generation = u64::from_le_bytes(bytes[0..8].try_into().expect("generation"));
                let object_id = u64::from_le_bytes(bytes[8..16].try_into().expect("object id"));
                let node_key = u64::from_le_bytes(bytes[16..24].try_into().expect("node key"));
                logln!(
                    "[admin] {}: generation={generation} object={object_id:#x} node={node_key:#x}",
                    core::str::from_utf8(name).unwrap_or("?")
                );
            }
            _ => {
                logln!("[admin] unknown command (try 'help')");
            }
        }
    }
}

pub use inner::test_el0_clusterctl;
