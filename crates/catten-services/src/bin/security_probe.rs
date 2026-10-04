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
        ConnectionRef,
        Endpoint,
        OwnedMemory,
        PendingCall,
    },
};
use catten_services::{
    deadline::Deadline,
    grant,
    grant_client,
    sleep_ms,
    socket,
};
use catten_syscall::IpcRights;
use charlotte_launch::{
    deployment,
    security_probe_status as status,
};

const AVAILABLE: &[u8] = b"security.available";
const MISSING: &[u8] = b"security.missing";
const SILENT: &[u8] = b"security.silent";
const PING: u32 = 0x5ec;
const PONG: i64 = 0x5eccafe;

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

fn serve(endpoint: &Endpoint) {
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
    }
}

fn wait(mut pending: PendingCall<'static>, endpoint: &Endpoint) -> CallResult {
    let deadline = Deadline::after(5_000);
    loop {
        check(!deadline.expired(), 100);
        serve(endpoint);
        if let Some(reply) = pending.poll().unwrap_or_else(|_| catten_rt::domain_abort()) {
            return reply;
        }
        sleep_ms(5);
    }
}

fn denied(reply: CallResult, error: i64, failure: u32) {
    check(reply.result == error && reply.connection.is_none() && reply.memory.is_none(), failure);
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
    config::write::<u32>(status::STAGE, status::STARTED);

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
            && replacement.artifact_digest == admitted.artifact_digest
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
        wait(connection.call(PING, 0).unwrap_or_else(|_| catten_rt::domain_abort()), &endpoint)
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
    // mintable source. The invalid opcode is rejected remotely, not by IPC.
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
            // This submission ABI reports a zero handle, not a status code.
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
    drop(silent_connection);
    checks |= 512;

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
    check(
        wait(connection.call(PING, 0).unwrap_or_else(|_| catten_rt::domain_abort()), &endpoint)
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
        serve(&endpoint);
        sleep_ms(5);
    }
}

catten_rt::entry!(main);
