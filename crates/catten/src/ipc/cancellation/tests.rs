//! Deterministic cancellation interleavings before secondary LP startup.
//! Scalar capabilities here are kernel ABI fixtures, not application owners.
use super::*;
use crate::memory::{
    self,
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    AddressSpaceCloseError,
    object::{
        self,
        MemoryObjectCap,
    },
    retirement::{
        CloseProgress,
        ClosingAddressSpace,
    },
};

struct Fixture {
    caller: AddressSpaceHandle,
    server: AddressSpaceHandle,
    endpoint: CapabilityId,
    call: CapabilityId,
    reply: Option<CapabilityId>,
    borrows: Vec<MemoryBorrow>,
    attachments: Vec<MemoryObjectCap>,
}

impl Fixture {
    fn new(count: usize, delivered: bool) -> Self {
        Self::create(count, delivered, true)
    }

    fn create(count: usize, delivered: bool, extras: bool) -> Self {
        let caller = crate::service::loader::create_user_address_space_handle();
        let server = crate::service::loader::create_user_address_space_handle();
        let endpoint = endpoint_create(server.id(), 0x4341_4e43, 1, 4).unwrap();
        let connection =
            connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
        let mut descriptor = Vec::new();
        descriptor.extend_from_slice(&((count + usize::from(extras) * 2) as u16).to_le_bytes());
        // Include move/copy authority so queued cancellation must reclaim it,
        // while delivered cancellation leaves it with the server.
        for mode in (0..count)
            .map(|index| {
                if index % 2 == 0 {
                    2u32
                } else {
                    3
                }
            })
            .chain([1, 0].into_iter().take(usize::from(extras) * 2))
        {
            let cap = object::allocate(caller.id(), 1).unwrap();
            descriptor.extend_from_slice(&cap.to_le_bytes());
            descriptor.extend_from_slice(&mode.to_le_bytes());
            descriptor.extend_from_slice(&0u32.to_le_bytes());
        }
        let vector = object::allocate(caller.id(), 1).unwrap();
        object::write_bytes(caller.id(), vector, &descriptor).unwrap();
        let call = vector_call(caller.id(), connection, 1, 0, vector).unwrap();
        // The vector-call ABI consumes its descriptor on successful submission.
        let (borrows, attachments) = {
            let ipc = IPC.read();
            let identity = resolve(&ipc, caller.id(), call).unwrap().unwrap();
            let Capability::Endpoint {
                endpoint: id,
                ..
            } = ipc.cap(server.id(), endpoint).unwrap()
            else {
                unreachable!()
            };
            (
                ipc.reply_tokens[&identity.token].borrows.clone(),
                ipc.endpoints[&id].queue.front().unwrap().memory.clone(),
            )
        };
        let reply = delivered.then(|| receive(server.id(), endpoint).unwrap().reply.unwrap());
        // Queued caps are guessed only by this raw kernel fixture. This lets
        // queued close exercise a real mapped-loan shootdown, not just metadata.
        for (index, borrow) in borrows.iter().enumerate() {
            object::map_any(server.id(), borrow.borrower_cap, index % 2 == 1).unwrap();
        }
        Self {
            caller,
            server,
            endpoint,
            call,
            reply,
            borrows,
            attachments,
        }
    }

    fn close(self) {
        memory::close_user_address_space_handle(self.caller).unwrap();
        memory::close_user_address_space_handle(self.server).unwrap();
    }
}

fn unlocked() {
    assert!(IPC.try_write().is_some(), "cancellation cleanup held IPC");
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(), "cancellation held lifecycle");
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some(), "cancellation held table");
}

pub(crate) fn run() {
    success_and_wait();
    queued_endpoint_close();
    preparation_rollback();
    staged_close();
    partial_failure();
    serialized_bulk_success();
    serialized_bulk_failure();
    abandonment();
    crate::logln!(
        "[IPC cancellation ownership] queued/delivered close, unlocked loan cleanup, competing \
         close/reply/receive, staged close, preparation rollback, partial failure and abandonment \
         passed; bulk failure retains queued authority and exact closing roots"
    );
}

fn success_and_wait() {
    for close_reply in [false, true] {
        let fixture = Fixture::new(2, true);
        let (asid, cap) = if close_reply {
            (fixture.server.id(), fixture.reply.unwrap())
        } else {
            (fixture.caller.id(), fixture.call)
        };
        let mut operation = Some(PreparedCancellation::prepare(asid, cap).unwrap());
        assert_eq!(
            reply(fixture.server.id(), fixture.reply.unwrap(), 1),
            Err(IpcError::ReplyAlreadyUsed)
        );
        assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
        let mut waits = 0;
        assert_eq!(
            close_cap_with_wait(asid, cap, || {
                unlocked();
                for handle in [fixture.caller, fixture.server] {
                    assert_eq!(
                        memory::close_user_address_space_handle(handle),
                        Err(AddressSpaceCloseError::OperationsInFlight)
                    );
                }
                operation
                    .take()
                    .unwrap()
                    .finish_with(|loan| loan.finish_observed(unlocked))
                    .unwrap();
                waits += 1;
            }),
            Err(IpcError::UnknownCapability)
        );
        assert_eq!(waits, 1);
        for borrow in &fixture.borrows {
            assert!(!object::info(borrow.owner, borrow.owner_cap).unwrap().lent);
            assert_eq!(
                object::info(borrow.borrower, borrow.borrower_cap),
                Err(MemoryObjectError::UnknownCapability)
            );
        }
        for &cap in &fixture.attachments[fixture.borrows.len()..] {
            assert!(object::info(fixture.server.id(), cap).is_ok());
        }
        if close_reply {
            assert_eq!(
                poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().result,
                REPLY_CANCELLED
            );
        }
        fixture.close();
    }
    // Public entry points route both targets to the owning cancellation path.
    for delivered in [false, true] {
        let fixture = Fixture::new(1, delivered);
        close_cap(fixture.caller.id(), fixture.call).unwrap();
        assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::NoMessage));
        fixture.close();
    }
    let fixture = Fixture::new(1, true);
    close_cap(fixture.server.id(), fixture.reply.unwrap()).unwrap();
    assert_eq!(
        poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().result,
        REPLY_CANCELLED
    );
    fixture.close();
}

fn queued_endpoint_close() {
    let fixture = Fixture::new(2, false);
    let mut operation =
        Some(PreparedCancellation::prepare(fixture.caller.id(), fixture.call).unwrap());
    assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::Pending));
    let endpoint_id =
        receive_endpoint_id(&IPC.read(), fixture.server.id(), fixture.endpoint).unwrap();
    assert!(!endpoint_is_readable_or_closed(endpoint_id).unwrap());
    let connection =
        connection_mint(fixture.server.id(), fixture.endpoint, ConnectionRights::ALL).unwrap();
    scalar_send(fixture.server.id(), connection, 99, 0).unwrap();
    let hits = Arc::new(core::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    let observer: Arc<dyn Observer> = crate::klib::observer::CallOnNotify::new(move || {
        unlocked();
        counter.fetch_add(1, Ordering::Relaxed);
    });
    let sponsor = WaitSponsor::new(false);
    let registration = EndpointObservable {
        endpoint: endpoint_id,
    }
    .try_register_waiter(Arc::downgrade(&observer), &sponsor)
    .unwrap();
    assert!(registration.is_owned(), "claimed front incorrectly reported readable");
    let mut waits = 0;
    close_cap_with_wait(fixture.server.id(), fixture.endpoint, || {
        unlocked();
        operation.take().unwrap().finish_with(|loan| loan.finish_observed(unlocked)).unwrap();
        assert!(endpoint_is_readable_or_closed(endpoint_id).unwrap());
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        waits += 1;
    })
    .unwrap();
    assert_eq!(waits, 1);
    drop(registration);
    for &cap in &fixture.attachments {
        assert_eq!(
            object::info(fixture.server.id(), cap),
            Err(MemoryObjectError::UnknownCapability)
        );
    }
    fixture.close();
}

fn preparation_rollback() {
    let fixture = Fixture::new(2, true);
    let borrow = fixture.borrows[1];
    let blocker = LoanRevocation::prepare(
        borrow.owner,
        borrow.owner_cap,
        borrow.borrower,
        borrow.borrower_cap,
    )
    .unwrap();
    assert_eq!(close_cap(fixture.caller.id(), fixture.call), Err(IpcError::MemoryTransferFailed));
    let first = fixture.borrows[0];
    LoanRevocation::prepare(first.owner, first.owner_cap, first.borrower, first.borrower_cap)
        .unwrap()
        .cancel_prepared();
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    blocker.cancel_prepared();
    close_cap(fixture.caller.id(), fixture.call).unwrap();
    fixture.close();
    // Second-root admission fails after the first lease was acquired. All
    // preparation leases return without consuming call authority or loans.
    let fixture = Fixture::new(1, false);
    let server = ClosingAddressSpace::begin(fixture.server).unwrap();
    assert_eq!(close_cap(fixture.caller.id(), fixture.call), Err(IpcError::ResourceLimit));
    let caller = ClosingAddressSpace::begin(fixture.caller).unwrap();
    assert!(matches!(caller.poll().unwrap(), CloseProgress::Complete));
    assert!(matches!(server.poll().unwrap(), CloseProgress::Complete));
}

fn staged_close() {
    let fixture = Fixture::new(1, false);
    let operation = PreparedCancellation::prepare(fixture.caller.id(), fixture.call).unwrap();
    let closing = ClosingAddressSpace::begin(fixture.caller).unwrap();
    let CloseProgress::Pending(closing) = closing.poll().unwrap() else {
        panic!("cancellation did not retain closing root");
    };
    operation.finish_with(|loan| loan.finish_observed(unlocked)).unwrap();
    assert!(matches!(closing.poll().unwrap(), CloseProgress::Complete));
    memory::close_user_address_space_handle(fixture.server).unwrap();
}

fn partial_failure() {
    let fixture = Fixture::new(3, false);
    let operation = PreparedCancellation::prepare(fixture.caller.id(), fixture.call).unwrap();
    let identity = operation.identity;
    let mut finished = 0;
    assert_eq!(
        operation.finish_with(|loan| {
            unlocked();
            finished += 1;
            if finished == 2 {
                drop(loan);
                Err(MemoryObjectError::UnmapFailed)
            } else {
                loan.finish_observed(unlocked)
            }
        }),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    assert_eq!(close_cap(fixture.caller.id(), fixture.call), Err(IpcError::MemoryTransferFailed));
    assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::Pending));
    let endpoint = receive_endpoint_id(&IPC.read(), fixture.server.id(), fixture.endpoint).unwrap();
    assert!(
        !endpoint_is_readable_or_closed(endpoint).unwrap(),
        "failed queue front regained delivery authority"
    );
    let ipc = IPC.read();
    assert_eq!(ipc.reply_tokens[&identity.token].borrows, fixture.borrows[..2]);
    drop(ipc);
    let mut ipc = IPC.write();
    let mut notifications = WaitNotifications::empty();
    assert_eq!(
        consume_reply_token(&mut ipc, identity.token, REPLY_ENDPOINT_CLOSED, &mut notifications),
        Err(IpcError::MemoryTransferFailed)
    );
    assert!(
        ipc.pending_calls[&identity.call].result.is_none(),
        "bulk cleanup falsely reported a failed loan as terminal"
    );
    drop(ipc);
    signal_observers(notifications);
    // Ordinary failure returns the operation leases, but whole-domain cleanup
    // must retain its root/slot and fail rather than discard the failed token.
    let used = memory::budget::used(fixture.caller);
    let free = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert_eq!(
        memory::close_user_address_space_handle(fixture.caller),
        Err(AddressSpaceCloseError::IpcCleanupFailed)
    );
    assert_eq!(memory::budget::used(fixture.caller), used);
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert!(matches!(
        AddressSpaceOperation::acquire(fixture.caller),
        Err(crate::memory::operation::OperationError::Closing)
    ));
    assert_eq!(
        ClosingAddressSpace::begin(fixture.server).unwrap().poll().err(),
        Some(AddressSpaceCloseError::IpcCleanupFailed)
    );
    assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
    assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::PermissionDenied));
}

fn serialized_bulk_success() {
    // Endpoint close preflights mapped loans before returning any caller's
    // borrow. The root adapter covers both queued and delivered call ownership.
    let fixture = Fixture::new(2, false);
    close_cap(fixture.server.id(), fixture.endpoint).unwrap();
    assert_eq!(
        poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().result,
        REPLY_ENDPOINT_CLOSED
    );
    for borrow in &fixture.borrows {
        assert!(!object::info(borrow.owner, borrow.owner_cap).unwrap().lent);
        assert_eq!(
            object::info(borrow.borrower, borrow.borrower_cap),
            Err(MemoryObjectError::UnknownCapability)
        );
    }
    fixture.close();
    for delivered in [false, true] {
        let fixture = Fixture::new(2, delivered);
        memory::close_user_address_space_handle(fixture.caller).unwrap();
        assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::NoMessage));
        if let Some(reply) = fixture.reply {
            assert_eq!(
                IPC.read().cap(fixture.server.id(), reply),
                Err(IpcError::UnknownCapability)
            );
        }
        memory::close_user_address_space_handle(fixture.server).unwrap();
    }
}

fn serialized_bulk_failure() {
    for close_endpoint in [false, true] {
        let fixture = Fixture::new(3, false);
        let identity = resolve(&IPC.read(), fixture.caller.id(), fixture.call).unwrap().unwrap();
        let endpoint =
            receive_endpoint_id(&IPC.read(), fixture.server.id(), fixture.endpoint).unwrap();
        let used = memory::budget::used(fixture.caller);
        let hits = Arc::new(core::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        let observer: Arc<dyn Observer> = crate::klib::observer::CallOnNotify::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let sponsor = WaitSponsor::new(false);
        let registration = PendingCallObservable {
            call: identity.call,
        }
        .try_register_waiter(Arc::downgrade(&observer), &sponsor)
        .unwrap();
        let (asid, cap) = if close_endpoint {
            (fixture.server.id(), fixture.endpoint)
        } else {
            (fixture.caller.id(), fixture.call)
        };
        let mut finished = 0;
        assert_eq!(
            close_cap_in_mode_with_revoker(
                asid,
                cap,
                false,
                || panic!("unclaimed bulk close must not wait"),
                |borrow| {
                    let loan = LoanRevocation::prepare(
                        borrow.owner,
                        borrow.owner_cap,
                        borrow.borrower,
                        borrow.borrower_cap,
                    )
                    .unwrap();
                    finished += 1;
                    if finished == 2 {
                        // Abandon a real prepared backing pin, modeling rejected
                        // physical cleanup rather than a synthetic token flag.
                        drop(loan);
                        Err(MemoryObjectError::UnmapFailed)
                    } else {
                        loan.finish()
                    }
                }
            ),
            Err(IpcError::MemoryTransferFailed)
        );
        assert_eq!(finished, 2);
        let ipc = IPC.read();
        assert!(ipc.cap(asid, cap).is_ok());
        assert!(!ipc.endpoints[&endpoint].closed);
        assert_eq!(ipc.endpoints[&endpoint].queue.len(), 1);
        assert_eq!(ipc.reply_tokens[&identity.token].borrows, fixture.borrows[..2]);
        assert!(ipc.reply_tokens[&identity.token].cleanup_failed);
        assert!(ipc.pending_calls[&identity.call].result.is_none());
        drop(ipc);
        assert_eq!(hits.load(Ordering::Relaxed), 0);
        assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::Pending));
        assert_eq!(
            close_cap(fixture.server.id(), fixture.endpoint),
            Err(IpcError::MemoryTransferFailed)
        );
        assert_eq!(
            close_cap_serialized(fixture.caller.id(), fixture.call),
            Err(IpcError::MemoryTransferFailed)
        );
        let free = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        for handle in [fixture.caller, fixture.server] {
            assert_eq!(
                memory::close_user_address_space_handle(handle),
                Err(AddressSpaceCloseError::IpcCleanupFailed)
            );
            assert_eq!(memory::current_address_space_handle(handle.id()), Some(handle));
            assert_eq!(
                memory::close_user_address_space_handle(handle),
                Err(AddressSpaceCloseError::CloseInProgress)
            );
            assert!(matches!(
                AddressSpaceOperation::acquire(handle),
                Err(crate::memory::operation::OperationError::Closing)
            ));
        }
        assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        assert_eq!(memory::budget::used(fixture.caller), used);
        let fresh = crate::service::loader::create_user_address_space_handle();
        assert_ne!(fresh.id(), fixture.caller.id());
        assert_ne!(fresh.id(), fixture.server.id());
        memory::close_user_address_space_handle(fresh).unwrap();
        assert_eq!(hits.load(Ordering::Relaxed), 0);
        drop(registration);
    }
}

fn abandonment() {
    let fixture = Fixture::create(1, false, false);
    drop(PreparedCancellation::prepare(fixture.caller.id(), fixture.call).unwrap());
    // Probe admission directly: public close intentionally waits forever for
    // this retained claim, so do not block a same-thread fixture on itself.
    assert!(matches!(
        PreparedCancellation::prepare(fixture.caller.id(), fixture.call),
        Err(IpcError::Pending)
    ));
    assert_eq!(receive(fixture.server.id(), fixture.endpoint), Err(IpcError::Pending));
    for handle in [fixture.caller, fixture.server] {
        assert_eq!(
            memory::close_user_address_space_handle(handle),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
    }
    assert_eq!(
        object::close_cap(fixture.caller.id(), fixture.borrows[0].owner_cap),
        Err(MemoryObjectError::LendingActive)
    );
    // The queue/claim, both roots and this one charged loan remain retained.
    assert_eq!(
        memory::budget::used(fixture.caller),
        memory::budget::Amount {
            pages: 1,
            objects: 1
        }
    );
}
