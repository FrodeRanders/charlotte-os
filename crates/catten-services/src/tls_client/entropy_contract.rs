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
                && message.memory.is_none()
                && message.connection.is_none(),
            300,
        );
        let reply = message.reply.take().unwrap_or_else(|| catten_rt::domain_abort());
        match mode {
            0 | 1 => {
                let memory = OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort());
                let mut mapping =
                    memory.map_writable().unwrap_or_else(|_| catten_rt::domain_abort());
                mapping.as_mut_slice()[..REQUEST_LEN].fill(0x3a);
                let memory = mapping.unmap().unwrap_or_else(|_| catten_rt::domain_abort());
                check(
                    reply.reply_move(memory, (REQUEST_LEN - usize::from(mode)) as i64).is_ok(),
                    301,
                );
            }
            2 => check(reply.reply(REQUEST_LEN as i64).is_ok(), 302),
            3 => check(
                reply.reply_connection(&self.endpoint, IpcRights::CALL, REQUEST_LEN as i64).is_ok(),
                303,
            ),
            4 => check(reply.reply(entropy::ERR_DEVICE).is_ok(), 304),
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
    for mode in 0..5 {
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
        "[security] entropy read: moved reply, malformed count/missing memory/extra connection, ",
        "device rejection, cancel/Drop and late-result cleanup; original deadline, ",
        "queue-full/silent timeout distinct and all eight slots recovered"
    ));
}
