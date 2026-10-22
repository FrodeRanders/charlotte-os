//! Opt-in adversarial scoped-application probe. No ambient naming authority.
#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;

use catten_rt::{
    Context,
    ManifestValue,
    config,
    owned::{
        CallResult,
        Completion,
        Connection,
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        PendingCall,
        ThreadHandle,
    },
};
use catten_services::{
    clusterctl_client,
    deadline::Deadline,
    grant,
    grant_client,
    publication,
    sleep_ms,
    socket,
};
use catten_syscall::IpcRights;
use charlotte_launch::{
    deployment,
    security_probe_status as status,
};

const AVAILABLE: &[u8] = b"secavail";
const MISSING: &[u8] = b"security.missing";
const SILENT: &[u8] = b"security.silent";
const PING: u32 = 0x5ec;
const PONG: i64 = 0x5eccafe;

/// One owner for the watched endpoint, its connection and completion batch.
struct CloseWatches {
    endpoint: Option<Endpoint>,
    connection: Connection,
    completions: Vec<Completion>,
}

impl Drop for CloseWatches {
    fn drop(&mut self) {
        drop(self.endpoint.take());
    }
}

extern "C" fn thread_probe_entry() {
    unsafe { catten_syscall::thread_exit() }
}

fn check(condition: bool, failure: u32) {
    if !condition {
        config::write::<u32>(status::FAILURE, failure);
        config::write::<u32>(status::STAGE, status::FAILED);
        catten_rt::logln!("[security-probe] failed check {}", failure);
        catten_rt::domain_abort();
    }
}

fn request(
    controller: ConnectionRef<'_>,
    descriptor: &[u8],
    service: &[u8],
    rights: u16,
) -> PendingCall<'static> {
    let memory = OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort());
    let mut mapping = memory.map_writable().unwrap_or_else(|_| catten_rt::domain_abort());
    let len = grant::encode_request(service, rights, descriptor, mapping.as_mut_slice())
        .unwrap_or_else(|| catten_rt::domain_abort());
    let memory = mapping.unmap().unwrap_or_else(|_| catten_rt::domain_abort());
    controller
        .call_move(grant::OP_ACQUIRE, len as u64, memory)
        .unwrap_or_else(|_| catten_rt::domain_abort())
}

fn serve(endpoint: &Endpoint) -> bool {
    if let Some(mut message) = endpoint.try_receive().unwrap_or_else(|_| catten_rt::domain_abort())
    {
        let result = if message.opcode == PING {
            PONG
        } else {
            -1
        };
        if let Some(reply) = message.reply.take() {
            let _ = reply.reply(result);
        }
        true
    } else {
        false
    }
}

fn wait(mut pending: PendingCall<'static>, endpoint: &Endpoint) -> CallResult {
    let deadline = Deadline::after(5_000);
    endpoint.bind_completion_queue(0).unwrap_or_else(|error| {
        catten_rt::logln!("[security-probe] wait endpoint interest rejected: {:?}", error);
        check(false, 102);
        catten_rt::domain_abort()
    });
    pending.bind_cq(0).unwrap_or_else(|error| {
        catten_rt::logln!("[security-probe] wait reply interest rejected: {:?}", error);
        check(false, 103);
        catten_rt::domain_abort()
    });
    loop {
        check(!deadline.expired(), 100);
        let serviced = serve(endpoint);
        if let Some(reply) = pending.poll().unwrap_or_else(|_| catten_rt::domain_abort()) {
            return reply;
        }
        if !serviced {
            let (_, status) = catten_syscall::cq_wait_checked(Some(deadline.remaining_ms()), 0);
            check(
                matches!(
                    status,
                    catten_syscall::completion_status::READY
                        | catten_syscall::completion_status::PENDING_OR_TIMEOUT
                ),
                104,
            );
        }
    }
}

fn denied(reply: CallResult, error: i64, failure: u32) {
    check(reply.result == error && reply.connection.is_none() && reply.memory.is_none(), failure);
}

fn controller_reactor_probe(ctx: &Context, tcpip: ConnectionRef<'_>) {
    use catten_services::{
        clusterctl,
        deployment_control::Controls,
        deployment_http::Request,
    };
    let endpoint = Endpoint::create(clusterctl::INTERFACE, clusterctl::VERSION, 8)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    let connection =
        endpoint.connect(IpcRights::CALL).unwrap_or_else(|_| catten_rt::domain_abort());
    let mut controls = Controls::new(connection.as_ref(), 0);
    let healthy_budget = Deadline::after(2_000);
    for _ in 0..2 {
        let socket = socket::OwnedSocket::open(tcpip, socket::DOMAIN_UDP)
            .unwrap_or_else(|_| catten_rt::domain_abort());
        check(
            controls
                .start(
                    socket,
                    Request::Status(b"security_missing".to_vec()),
                    catten_services::deployment_input::MAX_BUFFERED_BYTES,
                )
                .is_none(),
            200,
        );
    }
    let held = endpoint
        .try_receive()
        .unwrap_or_else(|_| catten_rt::domain_abort())
        .unwrap_or_else(|| catten_rt::domain_abort());
    let mut healthy = endpoint
        .try_receive()
        .unwrap_or_else(|_| catten_rt::domain_abort())
        .unwrap_or_else(|| catten_rt::domain_abort());
    check(
        held.opcode == clusterctl::OP_ROLLOUT
            && healthy.opcode == clusterctl::OP_ROLLOUT
            && held.memory.is_some()
            && healthy.memory.is_some(),
        201,
    );
    let _ = healthy
        .reply
        .take()
        .unwrap_or_else(|| catten_rt::domain_abort())
        .reply(clusterctl::ERR_NOT_FOUND);
    drop(healthy);
    let cq = ctx.completion_queue_layout();
    let mut completed = 0;
    let outer = Deadline::after(7_000);
    loop {
        for _ in 0..cq.entries {
            if unsafe { catten_syscall::cq_read(cq.base, cq.entries) }.is_none() {
                break;
            }
        }
        check(!outer.expired(), 202);
        if let Some((socket, reply)) = controls.advance() {
            if completed == 0 {
                check(
                    reply.starts_with("HTTP/1.1 404 ")
                        && !healthy_budget.expired()
                        && controls.occupied_slots() == 1
                        && controls.reserved_bytes() == 4096,
                    203,
                );
            } else {
                check(
                    reply.starts_with("HTTP/1.1 503 ")
                        && reply.contains("-22")
                        && controls.occupied_slots() == 0
                        && controls.reserved_bytes() == 0,
                    204,
                );
                // Local timeout does not release the receiver's moved page.
                let memory = held.memory.as_ref().unwrap_or_else(|| catten_rt::domain_abort());
                check(memory.len() == 4096, 205);
                let _ = socket.close();
                break;
            }
            let _ = socket.close();
            completed += 1;
            continue;
        }
        catten_services::reactor::wait(controls.remaining_ms(), 0);
    }
    check(endpoint.try_receive().unwrap_or_else(|_| catten_rt::domain_abort()).is_none(), 206);
    drop(held);
    catten_rt::logln!(concat!(
        "[security] deployment control CQ: healthy moved reply beside ",
        "held moved request; original timeout, no replay and receiver-owned backing retained"
    ));
}

fn output_reactor_probe(ctx: &Context, tcpip: ConnectionRef<'_>) {
    use catten_services::deployment_output::{
        Error,
        Outputs,
    };
    let mut outputs = Outputs::new(0);
    let budget = Deadline::after(2_000);
    for domain in [socket::DOMAIN_TCP, socket::DOMAIN_UDP] {
        let socket =
            socket::OwnedSocket::open(tcpip, domain).unwrap_or_else(|_| catten_rt::domain_abort());
        check(outputs.start(socket, alloc::borrow::Cow::Borrowed("probe")).is_none(), 210);
    }
    let cq = ctx.completion_queue_layout();
    let mut completed = 0;
    while completed < 2 {
        for _ in 0..cq.entries {
            if unsafe { catten_syscall::cq_read(cq.base, cq.entries) }.is_none() {
                break;
            }
        }
        check(!budget.expired(), 211);
        if let Some((socket, result)) = outputs.advance().1 {
            check(matches!(result, Err(Error::Service(socket::ERR_NOT_CONNECTED))), 212);
            let _ = socket.close();
            completed += 1;
            continue;
        }
        catten_services::reactor::wait(
            Some(outputs.remaining_ms().unwrap_or(0).min(budget.remaining_ms())),
            0,
        );
    }
    check(outputs.occupied_slots() == 0, 213);

    // A current TCP socket still rejects an untagged moved page before enqueue.
    let socket = socket::OwnedSocket::open(tcpip, socket::DOMAIN_TCP)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    let memory = OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort());
    let mut mapping = memory.map_writable().unwrap_or_else(|_| catten_rt::domain_abort());
    mapping.as_mut_slice().fill(0);
    let memory = mapping.unmap().unwrap_or_else(|_| catten_rt::domain_abort());
    let mut pending = tcpip
        .call_move(socket::OP_SEND, socket.id(), memory)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    let reply = pending
        .wait_timeout(2_000)
        .unwrap_or_else(|_| catten_rt::domain_abort())
        .unwrap_or_else(|| catten_rt::domain_abort());
    denied(reply, socket::ERR_BAD_OPCODE, 214);
    drop(pending);
    let _ = socket.close();
    catten_rt::logln!(concat!(
        "[security] deployment output CQ: TCP terminal and UDP rejection; ",
        "moved frame cleanup and retired frame rejection"
    ));
}

fn close_reactor_probe(ctx: &Context, tcpip: ConnectionRef<'_>, expired: socket::OwnedSocket<'_>) {
    // The existing held-controller fixture consumed five seconds while this
    // unactivated owner stayed live. TCP/IP may reap its backend independently.
    let mut retired = expired.into_closing();
    retired.submit().unwrap_or_else(|_| catten_rt::domain_abort());
    retired.bind_cq(0).unwrap_or_else(|_| catten_rt::domain_abort());
    let budget = Deadline::after(2_000);
    let cq = ctx.completion_queue_layout();
    loop {
        for _ in 0..cq.entries {
            if unsafe { catten_syscall::cq_read(cq.base, cq.entries) }.is_none() {
                break;
            }
        }
        check(!budget.expired(), 221);
        if let Some(receipt) = retired.poll().unwrap_or_else(|_| catten_rt::domain_abort()) {
            check(receipt == socket::CloseReceipt::Unavailable, 222);
            break;
        }
        catten_services::reactor::wait(Some(budget.remaining_ms()), 0);
    }
    drop(retired);
    let mut closes = catten_services::deployment_close::Closes::new(0);
    for domain in [socket::DOMAIN_TCP, socket::DOMAIN_UDP] {
        let socket =
            socket::OwnedSocket::open(tcpip, domain).unwrap_or_else(|_| catten_rt::domain_abort());
        closes.start(socket);
    }
    let cq = ctx.completion_queue_layout();
    while closes.occupied_slots() != 0 {
        for _ in 0..cq.entries {
            if unsafe { catten_syscall::cq_read(cq.base, cq.entries) }.is_none() {
                break;
            }
        }
        check(!budget.expired(), 220);
        if !closes.advance() {
            catten_services::reactor::wait(
                Some(closes.remaining_ms().unwrap_or(0).min(budget.remaining_ms())),
                0,
            );
        }
    }
    catten_rt::logln!(concat!(
        "[security] deployment close CQ: TCP and UDP logical acknowledgements; ",
        "reaped owner returns Unavailable; pending/socket owners released after logical receipts"
    ));
}

fn main(ctx: Context) -> ! {
    check(ctx.name_service_connection().is_none(), 1);
    let controller = ctx.bootstrap_connection().unwrap_or_else(|| catten_rt::domain_abort());
    let descriptor = ctx.profile_memory().unwrap_or_else(|| catten_rt::domain_abort());
    let own = descriptor
        .map_read_only()
        .unwrap_or_else(|_| catten_rt::domain_abort())
        .as_slice()
        .to_vec();
    let endpoint = Endpoint::create(0x5ec, 1, 8).unwrap_or_else(|_| catten_rt::domain_abort());
    endpoint.bind_completion_queue(0).unwrap_or_else(|_| catten_rt::domain_abort());
    config::write::<u32>(status::STAGE, status::STARTED);

    if matches!(ctx.manifest_value(status::MODE_KEY), Some(ManifestValue::Unsigned(2))) {
        let tcpip =
            grant_client::acquire(controller, &descriptor, b"tcpip", deployment::RIGHT_CALL)
                .unwrap_or_else(|_| catten_rt::domain_abort());
        let mut submitted = 0u32;
        loop {
            if let Some(shutdown) = ctx.lifecycle().shutdown_requested() {
                drop(tcpip);
                drop(endpoint);
                shutdown.complete();
            }
            // Real admitted clients continuously replenish the shared queue.
            // Every pending call remains in the owning batch through response
            // or cancellation; no integer cleanup or borrowed backing escapes.
            let batch: Vec<_> = (0..8)
                .filter_map(|_| tcpip.call(socket::OP_CONNECTION_STATE, u64::MAX).ok())
                .collect();
            submitted = submitted.saturating_add(batch.len() as u32);
            for call in batch {
                let result = call.wait().unwrap_or_else(|_| catten_rt::domain_abort());
                check(result.result == socket::ERR_BAD_SOCKET, 47);
            }
            config::write::<u32>(status::REQUESTS, submitted);
        }
    }

    if matches!(ctx.manifest_value(status::MODE_KEY), Some(ManifestValue::Unsigned(1))) {
        // Publish from a distinct domain: copied IPC attachments deliberately
        // reject same-domain destinations. Never receive on this endpoint.
        let silent = Endpoint::create(0x5ed, 1, 8).unwrap_or_else(|_| catten_rt::domain_abort());
        grant_client::publish(controller, &descriptor, SILENT, &silent)
            .unwrap_or_else(|_| catten_rt::domain_abort());
        let mut submitted = 0u32;
        loop {
            if let Some(shutdown) = ctx.lifecycle().shutdown_requested() {
                drop(silent);
                drop(endpoint);
                shutdown.complete();
            }
            // The owner contains all transient calls. Dropping the batch is
            // the cancellation path, including any replies that raced it.
            let batch: Vec<_> = (0..4)
                .map(|_| request(controller, &own, MISSING, deployment::RIGHT_CALL))
                .collect();
            submitted = submitted.saturating_add(batch.len() as u32);
            config::write::<u32>(status::REQUESTS, submitted);
            sleep_ms(5);
            drop(batch);
            sleep_ms(5);
        }
    }

    let Some(ManifestValue::Bytes(alternate)) = ctx.manifest_value(status::ALTERNATE_KEY) else {
        catten_rt::domain_abort()
    };
    let Some(ManifestValue::Bytes(key)) = ctx.manifest_value(status::DEPLOYMENT_KEY) else {
        catten_rt::domain_abort()
    };
    let key: &[u8; 32] = key.try_into().unwrap_or_else(|_| catten_rt::domain_abort());
    check(
        deployment::verify(&own, key) == deployment::VerifyOutcome::Valid
            && deployment::verify(alternate, key) == deployment::VerifyOutcome::Valid,
        2,
    );
    let admitted = deployment::decode(&own).unwrap_or_else(|| catten_rt::domain_abort());
    let replacement = deployment::decode(alternate).unwrap_or_else(|| catten_rt::domain_abort());
    check(
        replacement.artifact_name == admitted.artifact_name
            && replacement.variants().eq(admitted.variants())
            && replacement.sequence > admitted.sequence,
        3,
    );
    let mut checks = 1;
    denied(
        wait(request(controller, alternate, b"tcpip", deployment::CLIENT_RIGHTS), &endpoint),
        grant::ERR_UNAUTHORIZED,
        4,
    );
    checks |= 2;
    denied(
        wait(request(controller, &own, b"tcpip", deployment::CLIENT_RIGHTS), &endpoint),
        grant::ERR_UNAUTHORIZED,
        5,
    );
    checks |= 4;
    denied(
        wait(request(controller, &own, b"security.undeclared", deployment::RIGHT_CALL), &endpoint),
        grant::ERR_UNAUTHORIZED,
        6,
    );
    checks |= 8;

    // Submit more than one controller intake batch. Every accepted request
    // remains owned until its denial; no polling timer or replay rescues it.
    let batch: Vec<_> = (0..32)
        .map(|_| request(controller, &own, b"security.undeclared", deployment::RIGHT_CALL))
        .collect();
    for call in batch {
        denied(wait(call, &endpoint), grant::ERR_UNAUTHORIZED, 101);
    }
    catten_rt::logln!(
        "[security] reply CQ: 32 controller denials drained through owned multi-source waits"
    );

    let generation = grant_client::publish(controller, &descriptor, AVAILABLE, &endpoint)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    check(generation > 0, 7);
    config::write::<u64>(status::PUBLICATION_GENERATION, generation as u64);
    checks |= 16;

    // Two live calls coexist, so absence cannot serialize all grant work.
    let unavailable = request(controller, &own, MISSING, deployment::RIGHT_CALL);
    let found = wait(request(controller, &own, AVAILABLE, deployment::CLIENT_RIGHTS), &endpoint);
    check(found.result >= 1 && found.memory.is_none(), 8);
    let connection = found.connection.unwrap_or_else(|| catten_rt::domain_abort());
    denied(wait(unavailable, &endpoint), grant::ERR_UNAVAILABLE, 9);
    checks |= 64;
    check(
        wait(
            connection.call(PING, 0).unwrap_or_else(|error| {
                catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
                check(false, 105);
                catten_rt::domain_abort()
            }),
            &endpoint,
        )
        .result
            == PONG,
        10,
    );
    let memory = OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort());
    // The returned application connection must not be re-delegable. Failure
    // occurs in the kernel before the bogus controller request is submitted.
    let delegatable = endpoint
        .connect(IpcRights::CALL | IpcRights::MINT_CONNECTION)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    // Positive control: the same submission succeeds for an explicitly
    // mintable source. IPC accepts it, then the remote service rejects the invalid opcode.
    denied(
        wait(
            controller
                .call_delegated_connection_copy(
                    0,
                    0,
                    delegatable.as_ref(),
                    IpcRights::CALL,
                    &memory,
                )
                .unwrap_or_else(|_| catten_rt::domain_abort()),
            &endpoint,
        ),
        grant::ERR_INVALID,
        16,
    );
    drop(delegatable);
    check(
        matches!(
            controller.call_delegated_connection_copy(
                grant::OP_PUBLISH,
                0,
                connection.as_ref(),
                IpcRights::CALL,
                &memory,
            ),
            // This submission ABI signals rejection by returning a zero handle.
            Err(catten_rt::owned::IpcError::CreationFailed)
        ),
        11,
    );
    drop(memory);
    checks |= 32;

    // Exceed the historical waitlist size over time without leaving any
    // cancelled lookup parked in the shared name service.
    for _ in 0..96 {
        let batch: Vec<_> =
            (0..4).map(|_| request(controller, &own, MISSING, deployment::RIGHT_CALL)).collect();
        drop(batch);
        sleep_ms(5);
    }
    let recovered =
        grant_client::acquire(controller, &descriptor, AVAILABLE, deployment::CLIENT_RIGHTS)
            .unwrap_or_else(|_| catten_rt::domain_abort());
    check(
        wait(recovered.call(PING, 0).unwrap_or_else(|_| catten_rt::domain_abort()), &endpoint)
            .result
            == PONG,
        12,
    );
    checks |= 128;

    let tcpip = grant_client::acquire(controller, &descriptor, b"tcpip", deployment::RIGHT_CALL)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    check(
        matches!(
            tcpip.send(socket::OP_FRAME, 0),
            Err(catten_rt::owned::IpcError::Status(catten_syscall::ipc_status::PERMISSION_DENIED))
        ),
        13,
    ); // CALL does not imply SEND.
    let frame = OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort());
    denied(
        wait(
            tcpip
                .call_move(socket::OP_FRAME, 14, frame)
                .unwrap_or_else(|_| catten_rt::domain_abort()),
            &endpoint,
        ),
        socket::ERR_BAD_OPCODE,
        14,
    );
    checks |= 256;
    // Even if a controller accepts a call and never answers, publication
    // must cancel locally. The fake endpoint carries no naming authority.
    let silent_connection =
        grant_client::acquire(controller, &descriptor, SILENT, deployment::RIGHT_CALL)
            .unwrap_or_else(|_| catten_rt::domain_abort());
    let outer_budget = Deadline::after(7_000);
    let silent_result =
        grant_client::publish(silent_connection.as_ref(), &descriptor, AVAILABLE, &endpoint);
    check(
        matches!(silent_result, Err(grant_client::Error::Service(grant::ERR_UNAVAILABLE)))
            && !outer_budget.expired(),
        15,
    );
    // The platform publication helper must also bound *admission* to a full
    // remote queue. Keep all eight original calls owned until rejection; the
    // silent fixture never receives, so only cancellation can drain them.
    let queued: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent_connection.call(PING, 0).unwrap_or_else(|error| {
            catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
            check(false, 105);
            catten_rt::domain_abort()
        })
    });
    let admission_budget = Deadline::after(2_000);
    check(
        matches!(
            publication::register(silent_connection.as_ref(), 0, &endpoint),
            Err(publication::Error::Admission(catten_rt::owned::IpcError::Status(
                catten_syscall::ipc_status::QUEUE_FULL
            )))
        ) && !admission_budget.expired(),
        49,
    );
    drop(queued);
    // With that queue drained, one delegation is accepted but never answered.
    // Timeout must cancel that owner without replay or consuming the source.
    let observation_budget = Deadline::after(7_000);
    check(
        matches!(
            publication::register(silent_connection.as_ref(), 0, &endpoint),
            Err(publication::Error::ObservationTimeout)
        ) && !observation_budget.expired()
            && endpoint.status().is_ok(),
        50,
    );
    // All eight slots must be available again: a leaked hidden delegation or
    // replay would strand at least one slot in this never-receiving endpoint.
    let recovered_queue: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent_connection.call(PING, 0).unwrap_or_else(|error| {
            catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
            check(false, 105);
            catten_rt::domain_abort()
        })
    });
    drop(recovered_queue);
    catten_rt::logln!(concat!(
        "[security] platform publication: full-queue deadline, ",
        "silent-call cancellation and all eight slots recovered"
    ));
    let scalar_budget = Deadline::after(7_000);
    check(
        matches!(
            clusterctl_client::call(silent_connection.as_ref(), PING, 0),
            Err(clusterctl_client::Error::Timeout)
        ) && !scalar_budget.expired(),
        51,
    );
    checks |= 512;
    let expired_socket = socket::OwnedSocket::open(tcpip.as_ref(), socket::DOMAIN_UDP)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    controller_reactor_probe(&ctx, tcpip.as_ref());
    output_reactor_probe(&ctx, tcpip.as_ref());
    close_reactor_probe(&ctx, tcpip.as_ref(), expired_socket);

    // Many small, individually legal allocations must hit an aggregate cap
    // before exhausting node RAM. Retain every owner until rejection, then
    // release the whole batch and prove ordinary grant work recovers.
    let mut allocations = Vec::new();
    for _ in 0..2_048 {
        match OwnedMemory::allocate(1) {
            Ok(memory) => allocations.push(memory),
            Err(catten_rt::owned::MemoryError::AllocationFailed) => break,
            Err(_) => check(false, 17),
        }
    }
    check((16..=1_024).contains(&allocations.len()) && OwnedMemory::allocate(1).is_err(), 18);
    // At the original sponsor ceiling, rejection must return exactly one
    // moved request's charge, and accepted timeout must do the same for an
    // unreceived request. Retain every other page throughout both checks.
    let full: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent_connection.call(PING, 0).unwrap_or_else(|error| {
            catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
            check(false, 105);
            catten_rt::domain_abort()
        })
    });
    let request = allocations.pop().unwrap_or_else(|| catten_rt::domain_abort());
    check(
        matches!(
            clusterctl_client::call_move(silent_connection.as_ref(), PING, 0, request),
            Err(clusterctl_client::Error::Admission(catten_rt::owned::IpcError::Status(
                catten_syscall::ipc_status::QUEUE_FULL
            )))
        ),
        52,
    );
    allocations.push(OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort()));
    check(OwnedMemory::allocate(1).is_err(), 53);
    drop(full);
    let request = allocations.pop().unwrap_or_else(|| catten_rt::domain_abort());
    let moved_budget = Deadline::after(7_000);
    check(
        matches!(
            clusterctl_client::call_move(silent_connection.as_ref(), PING, 0, request),
            Err(clusterctl_client::Error::Timeout)
        ) && !moved_budget.expired(),
        54,
    );
    allocations.push(OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort()));
    check(OwnedMemory::allocate(1).is_err(), 55);
    let recovered_rpc_queue: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent_connection.call(PING, 0).unwrap_or_else(|error| {
            catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
            check(false, 105);
            catten_rt::domain_abort()
        })
    });
    drop(recovered_rpc_queue);
    drop(silent_connection);
    // Healthy status returns the same caller-sponsored page. Temporary
    // admission rejection may retry this read-only control, within its own
    // outer budget; a timeout or accepted result is never replayed here.
    let healthy_budget = Deadline::after(2_000);
    loop {
        check(!healthy_budget.expired(), 56);
        let request = allocations.pop().unwrap_or_else(|| catten_rt::domain_abort());
        let result = clusterctl_client::call_move(tcpip.as_ref(), socket::OP_STATUS, 0, request);
        let accepted = match result {
            Ok(reply) => {
                check(reply.result > 0 && reply.memory.is_some() && reply.connection.is_none(), 57);
                drop(reply);
                true
            }
            Err(clusterctl_client::Error::Admission(catten_rt::owned::IpcError::Status(
                catten_syscall::ipc_status::QUEUE_FULL,
            ))) => false,
            Err(_) => {
                check(false, 58);
                false
            }
        };
        allocations.push(OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort()));
        check(OwnedMemory::allocate(1).is_err(), 59);
        if accepted {
            break;
        }
        sleep_ms(5);
    }
    catten_rt::logln!(concat!(
        "[security] controller RPC: scalar/move deadlines, eight queue slots, ",
        "rejected/cancelled/returned page charges and healthy status passed"
    ));
    check(
        wait(
            connection.call(PING, 0).unwrap_or_else(|error| {
                catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
                check(false, 105);
                catten_rt::domain_abort()
            }),
            &endpoint,
        )
        .result
            == PONG,
        19,
    );
    drop(allocations);
    let after_pressure =
        grant_client::acquire(controller, &descriptor, AVAILABLE, deployment::CLIENT_RIGHTS)
            .unwrap_or_else(|_| catten_rt::domain_abort());
    check(
        wait(after_pressure.call(PING, 0).unwrap_or_else(|_| catten_rt::domain_abort()), &endpoint)
            .result
            == PONG,
        20,
    );
    drop(after_pressure);
    checks |= 1_024;

    let mut timers = Vec::new();
    for _ in 0..2_048 {
        match Completion::timer(3_600_000) {
            Ok(timer) => timers.push(timer),
            Err(catten_rt::owned::CompletionError::SubmissionFailed) => break,
            Err(_) => check(false, 21),
        }
    }
    check((4..=1_024).contains(&timers.len()) && Completion::timer(1).is_err(), 22);
    let cleanup_budget = Deadline::after(5_000);
    drop(timers);
    // Drop must cancel the hour-long timer without waiting for its deadline.
    check(!cleanup_budget.expired(), 23);
    for _ in 0..64 {
        drop(Completion::timer(3_600_000).unwrap_or_else(|_| catten_rt::domain_abort()));
    }
    check(Completion::timer(1).and_then(Completion::wait).is_ok(), 24);
    checks |= 2_048;
    let mut endpoints = Vec::new();
    for _ in 0..128 {
        match Endpoint::create(0x5ef, 1, 1) {
            Ok(endpoint) => endpoints.push(endpoint),
            Err(_) => break,
        }
    }
    check((4..64).contains(&endpoints.len()) && Endpoint::create(0x5ef, 1, 1).is_err(), 25);
    check(
        wait(
            connection.call(PING, 0).unwrap_or_else(|error| {
                catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
                check(false, 105);
                catten_rt::domain_abort()
            }),
            &endpoint,
        )
        .result
            == PONG,
        26,
    );
    drop(endpoints);
    for _ in 0..128 {
        drop(Endpoint::create(0x5ef, 1, 1).unwrap_or_else(|_| catten_rt::domain_abort()));
    }
    // Owning batches exercise record admission through the real EL0 ABI.
    // Keep the queue larger than the call-record limit so queue fullness does
    // not masquerade as record rejection. Drop cancels every outstanding call.
    let record_endpoint =
        Endpoint::create(0x5f2, 1, 1_024).unwrap_or_else(|_| catten_rt::domain_abort());
    let mut connections = Vec::new();
    for _ in 0..1_024 {
        match record_endpoint.connect(IpcRights::CALL) {
            Ok(connection) => connections.push(connection),
            Err(_) => break,
        }
    }
    check(
        (4..=512).contains(&connections.len()) && record_endpoint.connect(IpcRights::CALL).is_err(),
        39,
    );
    let mut calls = Vec::new();
    for _ in 0..1_024 {
        match connections[0].call(PING, 0) {
            Ok(call) => calls.push(call),
            Err(_) => break,
        }
    }
    check(calls.len() == 512 && connections[0].call(PING, 0).is_err(), 40);
    drop(calls);
    check(
        wait(
            connections[0].call(PING, 0).unwrap_or_else(|_| catten_rt::domain_abort()),
            &record_endpoint,
        )
        .result
            == PONG,
        41,
    );
    drop(connections);
    for _ in 0..128 {
        drop(
            record_endpoint.connect(IpcRights::CALL).unwrap_or_else(|_| catten_rt::domain_abort()),
        );
    }
    drop(record_endpoint);
    checks |= 4_096;
    let watched = Endpoint::create(0x5f0, 1, 1).unwrap_or_else(|_| catten_rt::domain_abort());
    let watched_connection =
        watched.connect(IpcRights::CALL).unwrap_or_else(|_| catten_rt::domain_abort());
    let mut watches = CloseWatches {
        endpoint: Some(watched),
        connection: watched_connection,
        completions: Vec::new(),
    };
    for _ in 0..2_048 {
        match watches.connection.watch_closed() {
            Ok(completion) => watches.completions.push(completion),
            Err(catten_rt::owned::CompletionError::SubmissionFailed) => break,
            Err(_) => check(false, 27),
        }
    }
    check(
        (4..=1_024).contains(&watches.completions.len())
            && watches.connection.watch_closed().is_err()
            && Completion::timer(1).is_err(),
        28,
    );
    check(
        wait(
            connection.call(PING, 0).unwrap_or_else(|error| {
                catten_rt::logln!("[security-probe] ping admission rejected: {:?}", error);
                check(false, 105);
                catten_rt::domain_abort()
            }),
            &endpoint,
        )
        .result
            == PONG,
        29,
    );
    drop(watches.endpoint.take());
    for completion in watches.completions.drain(..) {
        check(completion.wait() == Ok(catten_syscall::IPC_REPLY_ENDPOINT_CLOSED), 30);
    }
    drop(watches);
    check(Completion::timer(1).and_then(Completion::wait).is_ok(), 31);
    checks |= 8_192;
    let cancel_endpoint =
        Endpoint::create(0x5f1, 1, 1).unwrap_or_else(|_| catten_rt::domain_abort());
    let cancel_connection =
        cancel_endpoint.connect(IpcRights::CALL).unwrap_or_else(|_| catten_rt::domain_abort());
    let mut cancel_watches = Vec::new();
    for _ in 0..2_048 {
        match cancel_connection.watch_closed() {
            Ok(watch) => cancel_watches.push(watch),
            Err(catten_rt::owned::CompletionError::SubmissionFailed) => break,
            Err(_) => check(false, 32),
        }
    }
    check((4..=128).contains(&cancel_watches.len()), 33);
    let cleanup_budget = Deadline::after(5_000);
    // The endpoint stays live while Drop cancels these subscriptions.
    drop(cancel_watches);
    check(!cleanup_budget.expired(), 34);
    for _ in 0..128 {
        drop(cancel_connection.watch_closed().unwrap_or_else(|_| catten_rt::domain_abort()));
    }
    check(!cleanup_budget.expired(), 35);
    let mut surviving =
        cancel_connection.watch_closed().unwrap_or_else(|_| catten_rt::domain_abort());
    check(surviving.poll() == Ok(None), 36);
    drop(cancel_endpoint);
    check(surviving.wait() == Ok(catten_syscall::IPC_REPLY_ENDPOINT_CLOSED), 37);
    drop(cancel_connection);
    check(Completion::timer(1).and_then(Completion::wait).is_ok(), 38);
    checks |= 16_384;
    // This descriptor permits only the already-running main thread. Failed
    // construction must return an error and leave the domain able to proceed.
    check(unsafe { ThreadHandle::spawn(thread_probe_entry as *const () as usize, 0) }.is_err(), 42);
    let foreign_tid = match ctx.manifest_value(status::FOREIGN_THREAD_KEY) {
        Some(ManifestValue::Unsigned(tid)) => tid,
        _ => {
            check(false, 43);
            0
        }
    };
    check(unsafe { catten_syscall::observe_thread_exit(foreign_tid) } == u64::MAX, 44);
    check(catten_syscall::observe_thread_exit_generation(foreign_tid, u64::MAX) == u64::MAX, 45);
    check(catten_syscall::socket_owner_status(0, 0, 0) == u64::MAX, 46);
    checks |= 32_768;
    config::write::<u32>(status::CHECKS, checks);
    config::write::<u32>(status::STAGE, status::PASSED);
    catten_rt::logln!("[security-probe] passed checks={:#x}", checks);
    drop(tcpip);
    drop(recovered);
    drop(connection);
    loop {
        if let Some(shutdown) = ctx.lifecycle().shutdown_requested() {
            drop(endpoint);
            shutdown.complete();
        }
        if !serve(&endpoint) {
            let (_, status) = catten_syscall::cq_wait_checked(None, 0);
            check(status == catten_syscall::completion_status::READY, 106);
        }
    }
}

catten_rt::entry!(main);
