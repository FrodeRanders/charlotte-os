//! Public-data entropy-client ownership controls for the opt-in signed probe.
//! The local peer returns deterministic fixture bytes, never production entropy.

use catten_rt::owned::{
    Completion,
    Connection,
    ConnectionRef,
    Endpoint,
    IpcError,
    OwnedMemory,
    PendingCall,
};
use catten_services::{
    deadline::Deadline,
    entropy,
    entropy_service,
    tls_client::{
        self,
        EntropyError,
        ServiceEntropyRead,
    },
};
use catten_syscall::IpcRights;

use crate::check;

// Fixture budgets, independent of the installed production entropy policy.
const REQUEST_LEN: usize = 32;
const REQUEST_BUDGET_MS: u64 = 2_000;
const SILENT_BUDGET_MS: u64 = 30;
const DELAYED_OBSERVATION_MS: u64 = 80;

struct Peer {
    connection: Connection,
    endpoint: Endpoint,
}

impl Peer {
    fn new() -> Self {
        let endpoint = Endpoint::create(entropy::INTERFACE, entropy::VERSION, 8)
            .unwrap_or_else(|_| catten_rt::domain_abort());
        let connection =
            endpoint.connect(IpcRights::CALL).unwrap_or_else(|_| catten_rt::domain_abort());
        Self {
            connection,
            endpoint,
        }
    }

    fn reply(&self, mode: u8) {
        let mut message = self
            .endpoint
            .try_receive()
            .unwrap_or_else(|_| catten_rt::domain_abort())
            .unwrap_or_else(|| catten_rt::domain_abort());
        check(
            message.opcode == entropy::OP_FILL
                && message.arg0 == REQUEST_LEN as u64
                && message.memory.as_ref().is_some_and(|m| m.len() == entropy::BUFFER_BYTES)
                && message.connection.is_none(),
            300,
        );
        if mode == 0 || mode == 4 {
            check(
                entropy_service::serve_fill(message, |bytes| {
                    if mode == 4 {
                        return super::rng_device_contract::rejecting_fill(bytes);
                    }
                    bytes.fill(0x3a);
                    true
                }) == (mode == 0),
                320,
            );
            return;
        }
        let reply = message.reply.take().unwrap_or_else(|| catten_rt::domain_abort());
        match mode {
            1 | 5 => {
                let memory = if mode == 1 {
                    message.memory.take().unwrap_or_else(|| catten_rt::domain_abort())
                } else {
                    OwnedMemory::allocate(entropy::BUFFER_PAGES + 1)
                        .unwrap_or_else(|_| catten_rt::domain_abort())
                };
                let mut mapping =
                    memory.map_writable().unwrap_or_else(|_| catten_rt::domain_abort());
                mapping.as_mut_slice()[..REQUEST_LEN].fill(0x3a);
                let memory = mapping.unmap().unwrap_or_else(|_| catten_rt::domain_abort());
                check(
                    reply.reply_move(memory, (REQUEST_LEN - usize::from(mode == 1)) as i64).is_ok(),
                    301,
                );
            }
            2 => check(reply.reply(REQUEST_LEN as i64).is_ok(), 302),
            3 => check(
                reply.reply_connection(&self.endpoint, IpcRights::CALL, REQUEST_LEN as i64).is_ok(),
                303,
            ),
            _ => check(false, 305),
        }
    }
}

fn delayed_observation() {
    // Owned timer readiness, not repeated sleeps/polls, advances the original
    // operation past its deadline while retaining its pending owner/output.
    check(
        Completion::timer(DELAYED_OBSERVATION_MS)
            .unwrap_or_else(|_| catten_rt::domain_abort())
            .wait()
            .is_ok(),
        306,
    );
}

pub(crate) fn run(silent: ConnectionRef<'_>) {
    let peer = Peer::new();
    let mut bytes = [0xa5; REQUEST_LEN];
    server_contract(&peer);
    for mode in 0..6 {
        bytes.fill(0xa5);
        let read = ServiceEntropyRead::start(
            peer.connection.as_ref(),
            &mut bytes,
            Deadline::after(REQUEST_BUDGET_MS),
        )
        .unwrap_or_else(|_| catten_rt::domain_abort());
        peer.reply(mode);
        let result = read.finish();
        match mode {
            0 => check(result == Ok(()) && bytes == [0x3a; REQUEST_LEN], 307),
            4 => check(
                result == Err(EntropyError::Rejected(entropy::ERR_DEVICE))
                    && bytes == [0xa5; REQUEST_LEN],
                308,
            ),
            _ => check(
                result == Err(EntropyError::InvalidReply) && bytes == [0xa5; REQUEST_LEN],
                309,
            ),
        }
        check(peer.endpoint.status().is_ok_and(|(_, depth, _)| depth == 0), 310);
    }

    // Completed-but-unobserved memory is cleaned by both explicit cancel and
    // the pending owner's Drop; neither grants its bytes to the destination.
    for explicit in [true, false] {
        bytes.fill(0xa5);
        let read = ServiceEntropyRead::start(
            peer.connection.as_ref(),
            &mut bytes,
            Deadline::after(REQUEST_BUDGET_MS),
        )
        .unwrap_or_else(|_| catten_rt::domain_abort());
        peer.reply(0);
        if explicit {
            check(read.cancel().is_ok(), 311);
        } else {
            drop(read);
        }
        check(bytes == [0xa5; REQUEST_LEN], 312);
    }

    // Time spent by the caller cannot restart the request budget. Even a
    // ready moved-memory result stays unobserved and is disposed on timeout.
    bytes.fill(0xa5);
    let read = ServiceEntropyRead::start(
        peer.connection.as_ref(),
        &mut bytes,
        Deadline::after(SILENT_BUDGET_MS),
    )
    .unwrap_or_else(|_| catten_rt::domain_abort());
    peer.reply(0);
    delayed_observation();
    check(
        read.finish() == Err(EntropyError::ObservationTimeout) && bytes == [0xa5; REQUEST_LEN],
        313,
    );

    check(
        matches!(
            ServiceEntropyRead::start(peer.connection.as_ref(), &mut bytes, Deadline::after(0)),
            Err(EntropyError::ExpiredBeforeAdmission)
        ),
        314,
    );
    check(
        matches!(
            ServiceEntropyRead::start(
                peer.connection.as_ref(),
                &mut [],
                Deadline::after(REQUEST_BUDGET_MS)
            ),
            Err(EntropyError::InvalidLength)
        ),
        315,
    );
    let mut oversized = [0xa5; entropy::MAX_REQUEST + 1];
    check(
        matches!(
            ServiceEntropyRead::start(
                peer.connection.as_ref(),
                &mut oversized,
                Deadline::after(REQUEST_BUDGET_MS)
            ),
            Err(EntropyError::InvalidLength)
        ),
        316,
    );
    check(peer.endpoint.status().is_ok_and(|(_, depth, _)| depth == 0), 317);

    // The distinct silent domain never receives. A full queue rejects before
    // acceptance; a drained queue accepts exactly one request and times out.
    let held: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent
            .call(entropy::OP_FILL, REQUEST_LEN as u64)
            .unwrap_or_else(|_| catten_rt::domain_abort())
    });
    check(
        tls_client::fill_service_entropy(silent, &mut bytes, Deadline::after(REQUEST_BUDGET_MS))
            == Err(EntropyError::Submission(IpcError::Status(
                catten_syscall::ipc_status::QUEUE_FULL,
            ))),
        318,
    );
    drop(held);
    let outer = Deadline::after(REQUEST_BUDGET_MS);
    check(
        tls_client::fill_service_entropy(silent, &mut bytes, Deadline::after(SILENT_BUDGET_MS))
            == Err(EntropyError::ObservationTimeout)
            && !outer.expired(),
        319,
    );
    let recovered: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent
            .call(entropy::OP_FILL, REQUEST_LEN as u64)
            .unwrap_or_else(|_| catten_rt::domain_abort())
    });
    drop(recovered);
    catten_rt::logln!(concat!(
        "[security] entropy read: caller buffer, malformed count/missing memory/extra \
         connection/size, ",
        "device rejection, cancel/Drop and late-result cleanup; original deadline, ",
        "queue-full/silent timeout distinct and all eight slots recovered"
    ));
}

fn serve_invalid(peer: &Peer, pending: PendingCall<'_>, error: i64) {
    let message = peer
        .endpoint
        .try_receive()
        .unwrap_or_else(|_| catten_rt::domain_abort())
        .unwrap_or_else(|| catten_rt::domain_abort());
    check(
        !entropy_service::serve_fill(message, |_| {
            check(false, 321);
            false
        }),
        322,
    );
    let reply = pending.wait().unwrap_or_else(|_| catten_rt::domain_abort());
    check(reply.result == error && reply.memory.is_none() && reply.connection.is_none(), 323);
}

fn server_contract(peer: &Peer) {
    // Retired scalar opcode and current missing/oversized/invalid buffer shapes
    // must reject before the hardware callback, with the whole request owned.
    serve_invalid(
        peer,
        peer.connection
            .as_ref()
            .call(1, REQUEST_LEN as u64)
            .unwrap_or_else(|_| catten_rt::domain_abort()),
        entropy::ERR_BAD_OPCODE,
    );
    serve_invalid(
        peer,
        peer.connection
            .as_ref()
            .call(entropy::OP_FILL, REQUEST_LEN as u64)
            .unwrap_or_else(|_| catten_rt::domain_abort()),
        entropy::ERR_INVALID,
    );
    for length in [0, entropy::MAX_REQUEST as u64 + 1, u64::MAX] {
        let buffer = OwnedMemory::allocate(entropy::BUFFER_PAGES)
            .unwrap_or_else(|_| catten_rt::domain_abort());
        let pending = peer
            .connection
            .as_ref()
            .call_move(entropy::OP_FILL, length, buffer)
            .unwrap_or_else(|_| catten_rt::domain_abort());
        serve_invalid(peer, pending, entropy::ERR_INVALID);
    }
    let buffer = OwnedMemory::allocate(entropy::BUFFER_PAGES + 1)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    let pending = peer
        .connection
        .as_ref()
        .call_move(entropy::OP_FILL, REQUEST_LEN as u64, buffer)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    serve_invalid(peer, pending, entropy::ERR_INVALID);
    let buffer =
        OwnedMemory::allocate(entropy::BUFFER_PAGES).unwrap_or_else(|_| catten_rt::domain_abort());
    // The kernel rejects a loan to the same address space before submission.
    // Actual read-only mapping/revocation is tested against the distinct signed
    // RNG domain by the kernel caller-accounting fixture.
    check(
        peer.connection
            .as_ref()
            .call_borrow_read(entropy::OP_FILL, REQUEST_LEN as u64, &buffer)
            .is_err(),
        328,
    );
    drop(buffer.map_writable().unwrap_or_else(|_| catten_rt::domain_abort()));
    for length in [1, entropy::MAX_REQUEST] {
        let mut mapping = OwnedMemory::allocate(entropy::BUFFER_PAGES)
            .unwrap_or_else(|_| catten_rt::domain_abort())
            .map_writable()
            .unwrap_or_else(|_| catten_rt::domain_abort());
        mapping.as_mut_slice().fill(0xa5);
        let buffer = mapping.unmap().unwrap_or_else(|_| catten_rt::domain_abort());
        let pending = peer
            .connection
            .as_ref()
            .call_move(entropy::OP_FILL, length as u64, buffer)
            .unwrap_or_else(|_| catten_rt::domain_abort());
        let message = peer
            .endpoint
            .try_receive()
            .unwrap_or_else(|_| catten_rt::domain_abort())
            .unwrap_or_else(|| catten_rt::domain_abort());
        check(
            entropy_service::serve_fill(message, |bytes| {
                check(bytes.len() == length, 324);
                bytes.fill(0x3a);
                true
            }),
            325,
        );
        let reply = pending.wait().unwrap_or_else(|_| catten_rt::domain_abort());
        check(reply.result == length as i64 && reply.connection.is_none(), 326);
        let mapping = reply
            .memory
            .unwrap_or_else(|| catten_rt::domain_abort())
            .map_read_only()
            .unwrap_or_else(|_| catten_rt::domain_abort());
        check(
            mapping.len() == entropy::BUFFER_BYTES
                && mapping.as_slice()[..length].iter().all(|b| *b == 0x3a)
                && mapping.as_slice()[length..].iter().all(|b| *b == 0xa5),
            327,
        );
    }
    catten_rt::logln!(
        "[security] entropy server: retired/missing/oversized/invalid rejected before fill; \
         same-domain loan rejected before admission; minimum/maximum caller buffers and untouched \
         suffix verified"
    );
}
