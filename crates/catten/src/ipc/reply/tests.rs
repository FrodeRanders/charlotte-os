//! Deterministic interleaving fixtures for reply ownership, before AP boot.
//! Kernel ABI tests intentionally hold scalar handles. Failure/abandonment
//! retain two data pages; abandonment additionally retains both live roots.

use super::*;
mod connection_tests;
mod memory_tests;
use crate::memory::{
    self,
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    AddressSpaceCloseError,
    object,
    retirement::{
        CloseProgress,
        ClosingAddressSpace,
    },
};

struct Fixture {
    caller: AddressSpaceHandle,
    server: AddressSpaceHandle,
    call: CapabilityId,
    reply: CapabilityId,
    owners: Vec<MemoryObjectCap>,
    borrows: Vec<MemoryBorrow>,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let caller = crate::service::loader::create_user_address_space_handle();
        let server = crate::service::loader::create_user_address_space_handle();
        let endpoint = endpoint_create(server.id(), 0x5250_4c59, 1, 4).unwrap();
        let connection =
            connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
        let mut owners = Vec::new();
        let mut descriptor = Vec::new();
        descriptor.extend_from_slice(&(count as u16).to_le_bytes());
        for _ in 0..count {
            let cap = object::allocate(caller.id(), 1).unwrap();
            owners.push(cap);
            descriptor.extend_from_slice(&cap.to_le_bytes());
            descriptor.extend_from_slice(&2u32.to_le_bytes());
            descriptor.extend_from_slice(&0u32.to_le_bytes());
        }
        let vector = object::allocate(caller.id(), 1).unwrap();
        object::write_bytes(caller.id(), vector, &descriptor).unwrap();
        let call = vector_call(caller.id(), connection, 1, 0, vector).unwrap();
        let reply = receive(server.id(), endpoint).unwrap().reply.unwrap();
        let borrows = {
            let ipc = IPC.read();
            let (token, _, _) = validate(&ipc, server.id(), reply).unwrap();
            ipc.reply_tokens[&token].borrows.clone()
        };
        for borrow in &borrows {
            object::map_any(server.id(), borrow.borrower_cap, false).unwrap();
        }
        Self {
            caller,
            server,
            call,
            reply,
            owners,
            borrows,
        }
    }

    fn prepare(&self) -> PreparedReply {
        PreparedReply::prepare(self.server.id(), self.reply).unwrap()
    }

    fn close(self) {
        memory::close_user_address_space_handle(self.caller).unwrap();
        memory::close_user_address_space_handle(self.server).unwrap();
    }
}

fn unlocked() {
    assert!(IPC.try_write().is_some(), "reply physical cleanup held IPC");
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(), "reply cleanup held lifecycle");
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some(), "reply cleanup held table");
}

pub(crate) fn run() {
    success_and_close_wait();
    preparation_rollback();
    staged_close();
    connection_tests::run();
    memory_tests::run();
    completion_failure();
    abandonment();
    crate::logln!(
        "[IPC reply ownership] unlocked loan detach, competing reply/close, preparation rollback, \
         staged close, partial failure and abandonment passed; failed cleanup retains exact \
         closing roots and original backing charges"
    );
}

fn success_and_close_wait() {
    for close_reply in [false, true] {
        let fixture = Fixture::new(2);
        let operation = fixture.prepare();
        assert_eq!(reply(fixture.server.id(), fixture.reply, 99), Err(IpcError::ReplyAlreadyUsed));
        assert_eq!(
            reply_with_memory_move(fixture.server.id(), fixture.reply, fixture.owners[0], 99),
            Err(IpcError::ReplyAlreadyUsed)
        );
        assert_eq!(
            memory::close_user_address_space_handle(fixture.caller),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
        assert_eq!(
            memory::close_user_address_space_handle(fixture.server),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
        let mut operation = Some(operation);
        let mut waits = 0;
        let (asid, cap) = if close_reply {
            (fixture.server.id(), fixture.reply)
        } else {
            (fixture.caller.id(), fixture.call)
        };
        let closed = close_cap_with_wait(asid, cap, || {
            unlocked();
            assert!(object::info(fixture.caller.id(), fixture.owners[0]).unwrap().lent);
            operation
                .take()
                .unwrap()
                .finish_with(42, |loan| loan.finish_observed(unlocked))
                .unwrap();
            waits += 1;
        });
        assert_eq!(waits, 1);
        assert_eq!(
            closed,
            if close_reply {
                Err(IpcError::UnknownCapability)
            } else {
                Ok(())
            }
        );
        if close_reply {
            assert_eq!(poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().result, 42);
        }
        for borrow in &fixture.borrows {
            assert_eq!(
                object::info(fixture.server.id(), borrow.borrower_cap),
                Err(MemoryObjectError::UnknownCapability)
            );
        }
        for &cap in &fixture.owners {
            assert!(!object::info(fixture.caller.id(), cap).unwrap().lent);
        }
        fixture.close();
    }
}

fn preparation_rollback() {
    let fixture = Fixture::new(2);
    let borrow = fixture.borrows[1];
    let blocker = LoanRevocation::prepare(
        borrow.owner,
        borrow.owner_cap,
        borrow.borrower,
        borrow.borrower_cap,
    )
    .unwrap();
    assert!(matches!(
        PreparedReply::prepare(fixture.server.id(), fixture.reply),
        Err(IpcError::MemoryTransferFailed)
    ));
    // The first prepared loan was restored without invalidating or releasing
    // its mapping. Only admission receipts, never started cleanup, can do this.
    let first = fixture.borrows[0];
    assert!(object::info(fixture.server.id(), first.borrower_cap).unwrap().mapped);
    let first_again =
        LoanRevocation::prepare(first.owner, first.owner_cap, first.borrower, first.borrower_cap)
            .unwrap();
    first_again.cancel_prepared();
    blocker.cancel_prepared();
    fixture.prepare().finish(1).unwrap();
    fixture.close();

    let fixture = Fixture::new(1);
    let closing = ClosingAddressSpace::begin(fixture.server).unwrap();
    assert!(matches!(
        PreparedReply::prepare(fixture.server.id(), fixture.reply),
        Err(IpcError::ResourceLimit)
    ));
    // Failed second lease admission must release the first root's lease.
    memory::close_user_address_space_handle(fixture.caller).unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
}

fn staged_close() {
    let fixture = Fixture::new(1);
    let source = endpoint_create(fixture.server.id(), 8, 1, 4).unwrap();
    let operation = PreparedReply::prepare_with_connection(
        fixture.server.id(),
        fixture.reply,
        Some((source, ConnectionRights::ALL)),
    )
    .unwrap();
    let closing = ClosingAddressSpace::begin(fixture.caller).unwrap();
    let CloseProgress::Pending(closing) = closing.poll().unwrap() else {
        panic!("reply lease did not hold close");
    };
    operation.finish_with(1, |loan| loan.finish_observed(unlocked)).unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    memory::close_user_address_space_handle(fixture.server).unwrap();
}

fn completion_failure() {
    let fixture = Fixture::new(3);
    let source = endpoint_create(fixture.server.id(), 2, 1, 4).unwrap();
    let before = crate::capability::admission_tests::test_namespace_used(fixture.caller.id());
    let memory = object::allocate(fixture.server.id(), 1).unwrap();
    let operation = PreparedReply::prepare_with_outputs(
        fixture.server.id(),
        fixture.reply,
        Some((source, ConnectionRights::ALL)),
        Some(memory),
    )
    .unwrap();
    let destination = operation.connection.as_ref().unwrap().grant.authority.identity();
    let mut finished = 0;
    assert_eq!(
        operation.finish_with(1, |loan| {
            unlocked();
            finished += 1;
            if finished == 2 {
                drop(loan); // Fault before detach: fence/pin survive uncertain cleanup.
                Err(MemoryObjectError::UnmapFailed)
            } else {
                loan.finish_observed(unlocked)
            }
        }),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    assert_eq!(
        crate::capability::admission_tests::test_namespace_used(fixture.caller.id()),
        before
    );
    assert!(!crate::capability::contains(
        fixture.caller.id(),
        destination,
        crate::capability::ObjectKind::Ipc
    ));
    close_cap(fixture.server.id(), source).unwrap();
    object::close_cap(fixture.server.id(), memory).unwrap();
    let ipc = IPC.read();
    let (token, _, _) = validate(&ipc, fixture.server.id(), fixture.reply).unwrap();
    assert_eq!(ipc.reply_tokens[&token].borrows, fixture.borrows[..2]);
    drop(ipc);
    assert_eq!(
        object::info(fixture.server.id(), fixture.borrows[2].borrower_cap),
        Err(MemoryObjectError::UnknownCapability)
    );
    let first = fixture.borrows[0];
    LoanRevocation::prepare(first.owner, first.owner_cap, first.borrower, first.borrower_cap)
        .unwrap()
        .cancel_prepared();
    assert_eq!(
        object::close_cap(fixture.caller.id(), fixture.owners[1]),
        Err(MemoryObjectError::LendingActive)
    );
    let used = memory::budget::used(fixture.caller);
    for handle in [fixture.caller, fixture.server] {
        assert_eq!(
            memory::close_user_address_space_handle(handle),
            Err(AddressSpaceCloseError::IpcCleanupFailed)
        );
        assert_eq!(memory::current_address_space_handle(handle.id()), Some(handle));
    }
    // Returned leases do not prove uncertain loan cleanup. Whole-root close
    // retains the token, roots and every original charge on that rejection.
    assert_eq!(memory::budget::used(fixture.caller), used);
}

fn abandonment() {
    let fixture = Fixture::new(1);
    let source = endpoint_create(fixture.server.id(), 3, 1, 4).unwrap();
    let before = crate::capability::admission_tests::test_namespace_used(fixture.caller.id());
    let memory = object::allocate(fixture.server.id(), 1).unwrap();
    drop(
        PreparedReply::prepare_with_outputs(
            fixture.server.id(),
            fixture.reply,
            Some((source, ConnectionRights::ALL)),
            Some(memory),
        )
        .unwrap(),
    );
    assert_eq!(
        crate::capability::admission_tests::test_namespace_used(fixture.caller.id()),
        before
    );
    let ipc = IPC.read();
    let token = match ipc.cap(fixture.server.id(), fixture.reply).unwrap() {
        Capability::ReplyToken {
            token,
        } => token,
        _ => unreachable!(),
    };
    assert_eq!(ipc.reply_tokens[&token].connection_source, Some(source));
    drop(ipc);
    // Returned memory was never detached or published: preparation Drop safely
    // restores its original source, even though reply/loan/root claims remain.
    object::close_cap(fixture.server.id(), memory).unwrap();
    assert_eq!(reply(fixture.server.id(), fixture.reply, 1), Err(IpcError::ReplyAlreadyUsed));
    assert_eq!(
        memory::close_user_address_space_handle(fixture.caller),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(
        memory::close_user_address_space_handle(fixture.server),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(
        object::close_cap(fixture.caller.id(), fixture.owners[0]),
        Err(MemoryObjectError::LendingActive)
    );
    assert_eq!(
        memory::budget::used(fixture.caller),
        memory::budget::Amount {
            pages: 1,
            objects: 1
        }
    );
    // No recovery bypass: both root leases, IPC records and backing stay fenced.
}
