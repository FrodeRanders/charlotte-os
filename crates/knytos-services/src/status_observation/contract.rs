//! Native controls using the same snapshot adapter as httpd, at a full caller
//! memory budget and against an eight-slot endpoint that never receives.

use alloc::vec::Vec;

use catten_rt::owned::{
    ConnectionRef,
    IpcError,
    MemoryError,
    OwnedMemory,
    PendingCall,
};
use catten_services::{
    deadline::Deadline,
    socket,
    status_observation::{
        Error,
        Snapshot,
    },
};

pub fn run(
    silent: ConnectionRef<'_>,
    tcpip: ConnectionRef<'_>,
    allocations: &mut Vec<OwnedMemory>,
) {
    fn check(condition: bool) {
        if !condition {
            catten_rt::logln!("[security] status snapshot observation contract failed");
            catten_rt::domain_abort();
        }
    }
    fn restored(allocations: &mut Vec<OwnedMemory>) {
        allocations.push(OwnedMemory::allocate(1).unwrap_or_else(|_| catten_rt::domain_abort()));
        check(OwnedMemory::allocate(1).is_err());
    }

    let outer = Deadline::after(2_000);
    let snapshot = Snapshot::new(Deadline::after(100));
    check(matches!(snapshot.call(silent, 0x5ec, 0), Err(Error::Timeout)) && !outer.expired());

    let full: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent.call(0x5ec, 0).unwrap_or_else(|_| catten_rt::domain_abort())
    });
    // An expired snapshot must reject before IPC admission or allocation, even
    // when both resources are exhausted. It cannot restart for another source.
    check(matches!(snapshot.call(silent, 0x5ec, 0), Err(Error::Timeout)));
    check(matches!(snapshot.buffered_call(silent, 0x5ec, 0), Err(Error::Timeout)));
    let available = Snapshot::new(Deadline::after(2_000));
    check(matches!(
        available.call(silent, 0x5ec, 0),
        Err(Error::Admission(socket::SocketError::Ipc(IpcError::Status(
            catten_syscall::ipc_status::QUEUE_FULL
        ))))
    ));
    check(matches!(
        available.buffered_call(silent, 0x5ec, 0),
        Err(Error::Admission(socket::SocketError::Memory(MemoryError::AllocationFailed)))
    ));
    drop(allocations.pop().unwrap_or_else(|| catten_rt::domain_abort()));
    check(matches!(
        available.buffered_call(silent, 0x5ec, 0),
        Err(Error::Admission(socket::SocketError::Ipc(IpcError::Status(
            catten_syscall::ipc_status::QUEUE_FULL
        ))))
    ));
    restored(allocations);
    drop(full);

    drop(allocations.pop().unwrap_or_else(|| catten_rt::domain_abort()));
    let outer = Deadline::after(2_000);
    let snapshot = Snapshot::new(Deadline::after(100));
    check(
        matches!(snapshot.buffered_call(silent, 0x5ec, 0), Err(Error::Timeout)) && !outer.expired(),
    );
    restored(allocations);
    // All eight slots and exactly the original moved page are available again.
    let recovered: [PendingCall<'_>; 8] = core::array::from_fn(|_| {
        silent.call(0x5ec, 0).unwrap_or_else(|_| catten_rt::domain_abort())
    });
    drop(recovered);

    drop(allocations.pop().unwrap_or_else(|| catten_rt::domain_abort()));
    let snapshot = Snapshot::new(Deadline::after(2_000));
    let reply = snapshot
        .buffered_call(tcpip, socket::OP_STATUS, 0)
        .unwrap_or_else(|_| catten_rt::domain_abort());
    check(reply.result == (socket::STATUS_WORDS * 4) as i64 && reply.connection.is_none());
    let memory = reply.memory.unwrap_or_else(|| catten_rt::domain_abort());
    let mapping = memory.map_read_only().unwrap_or_else(|_| catten_rt::domain_abort());
    check(mapping.as_slice().len() >= socket::STATUS_WORDS * 4);
    drop(mapping);
    restored(allocations);
    catten_rt::logln!(concat!(
        "[security] status snapshot: shared deadline, expired admission, scalar/moved timeout, ",
        "eight slots, rejected/cancelled/returned page charges and healthy status passed"
    ));
}
