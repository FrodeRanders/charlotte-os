//! Kernel-only white-box coverage of migrated IPC waiter sources.

use alloc::vec::Vec;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::klib::observer::CallOnNotify;

const SERVER: usize = 0x000e_a117;
const CLIENT: usize = 0x000e_a118;

fn fixture() -> (CapabilityId, CapabilityId, EndpointId) {
    let endpoint = endpoint_create(SERVER, 1, 1, 4).unwrap();
    let connection = connection_delegate(SERVER, endpoint, CLIENT, ConnectionRights::ALL).unwrap();
    let id = receive_endpoint_id(&IPC.read(), SERVER, endpoint).unwrap();
    (endpoint, connection, id)
}

pub(crate) fn test_source_admission() {
    let baseline = waiter_budget::node_used();
    let sponsor = WaitSponsor::new(false);
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        // Every source callback must run after the IPC registry is released.
        let _ipc = IPC.read();
        count.fetch_add(1, Ordering::Relaxed);
    });
    let (endpoint, connection, id) = fixture();
    let source = EndpointObservable {
        endpoint: id,
    };
    let mut tokens = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        tokens.push(source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT);
    assert!(matches!(
        source.try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::ResourceLimit)
    ));
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT);
    drop(tokens.pop());
    assert_eq!(
        IPC.read().endpoints[&id].readiness_observers.registered(),
        waiter_budget::SOURCE_LIMIT - 1
    );
    drop(tokens);
    for _ in 0..512 {
        let token = source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
        assert!(token.is_owned());
        drop(token);
        assert_eq!(IPC.read().endpoints[&id].readiness_observers.registered(), 0);
    }

    let token = source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    scalar_send(CLIENT, connection, 1, 2).unwrap();
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    assert_eq!(sponsor.used(), 0);
    assert!(source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap().is_ready());
    drop(token);
    receive(SERVER, endpoint).unwrap();
    assert_eq!(sponsor.used(), 0);

    let call_cap = scalar_call(CLIENT, connection, 3, 4).unwrap();
    let call = pending_call_id(CLIENT, call_cap).unwrap();
    let call_source = PendingCallObservable {
        call,
    };
    let reply_cap = receive(SERVER, endpoint).unwrap().reply.unwrap();
    let mut tokens = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        tokens.push(call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert!(matches!(
        call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::ResourceLimit)
    ));
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT);
    drop(tokens);
    let token = call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    reply(SERVER, reply_cap, 42).unwrap();
    assert_eq!(hits.load(Ordering::Relaxed), 2);
    assert_eq!(sponsor.used(), 0);
    assert!(
        call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap().is_ready()
    );
    assert_eq!(poll_reply(CLIENT, call_cap).unwrap().unwrap().result, 42);
    drop(token);
    close_cap(CLIENT, call_cap).unwrap();
    assert!(matches!(
        call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::Closed)
    ));

    // Dropping reply authority signals cancellation, not a lost wake.
    let call_cap = scalar_call(CLIENT, connection, 5, 6).unwrap();
    let source = PendingCallObservable {
        call: pending_call_id(CLIENT, call_cap).unwrap(),
    };
    let token = source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    let reply_cap = receive(SERVER, endpoint).unwrap().reply.unwrap();
    close_cap(SERVER, reply_cap).unwrap();
    assert_eq!(poll_reply(CLIENT, call_cap).unwrap().unwrap().result, REPLY_CANCELLED);
    assert_eq!(hits.load(Ordering::Relaxed), 3);
    assert_eq!(sponsor.used(), 0);
    drop(token);
    close_cap(CLIENT, call_cap).unwrap();

    // Closing an endpoint combines receiver and queued-call notifications
    // without allocating a callback vector while holding IPC.
    let call_cap = scalar_call(CLIENT, connection, 7, 8).unwrap();
    let second_call_cap = scalar_call(CLIENT, connection, 7, 9).unwrap();
    let call_source = PendingCallObservable {
        call: pending_call_id(CLIENT, call_cap).unwrap(),
    };
    let call_token = call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    let second_source = PendingCallObservable {
        call: pending_call_id(CLIENT, second_call_cap).unwrap(),
    };
    let second_token =
        second_source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    // The queued request is already readable. Direct registration here tests
    // combining closure batches rather than the observable's ready fast path.
    let list = IPC.read().endpoints[&id].readiness_observers.clone();
    let receiver_token = sponsor.register(&list, Arc::downgrade(&observer)).unwrap();
    close_cap(SERVER, endpoint).unwrap();
    assert_eq!(hits.load(Ordering::Relaxed), 6);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(poll_reply(CLIENT, call_cap).unwrap().unwrap().result, REPLY_ENDPOINT_CLOSED);
    assert_eq!(poll_reply(CLIENT, second_call_cap).unwrap().unwrap().result, REPLY_ENDPOINT_CLOSED);
    assert!(
        call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap().is_ready()
    );
    assert!(
        EndpointObservable {
            endpoint: id
        }
        .try_register_waiter(Arc::downgrade(&observer), &sponsor)
        .unwrap()
        .is_ready()
    );
    drop((call_token, second_token, receiver_token));
    close_cap(CLIENT, call_cap).unwrap();
    close_cap(CLIENT, second_call_cap).unwrap();
    close_cap(CLIENT, connection).unwrap();
    assert!(matches!(
        EndpointObservable {
            endpoint: id
        }
        .try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::Closed)
    ));

    // Caller close revokes a still-live read loan before waking its waiters.
    let server = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(server.id(), 1, 1, 4).unwrap();
    let owner = crate::service::loader::create_user_address_space_handle();
    let connection_owner =
        connection_delegate(server.id(), endpoint, owner.id(), ConnectionRights::CALL).unwrap();
    let memory = crate::memory::object::allocate(owner.id(), 1).unwrap();
    let call_cap =
        scalar_call_with_memory_borrow_read(owner.id(), connection_owner, 9, 10, memory).unwrap();
    let call_source = PendingCallObservable {
        call: pending_call_id(owner.id(), call_cap).unwrap(),
    };
    let request = receive(server.id(), endpoint).unwrap();
    let loan = request.memory.unwrap();
    let token = call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    close_cap(owner.id(), call_cap).unwrap();
    assert!(crate::memory::object::info(server.id(), loan).is_err());
    assert!(crate::memory::object::info(owner.id(), memory).is_ok());
    assert!(matches!(
        call_source.try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::Closed)
    ));
    drop(token);
    assert_eq!(sponsor.used(), 0);
    crate::memory::close_user_address_space_handle(owner).unwrap();
    crate::memory::close_user_address_space_handle(server).unwrap();

    // Retired wait sponsorship rejects registration against a live remote
    // source; a replacement ASID's charge remains independent.
    let (endpoint, connection, id) = fixture();
    let old_handle = crate::service::loader::create_user_address_space_handle();
    let old = crate::memory::budget::waiter_sponsor(old_handle.id());
    let source = EndpointObservable {
        endpoint: id,
    };
    let token = source.try_register_waiter(Arc::downgrade(&observer), &old).unwrap();
    crate::memory::budget::retire(old_handle);
    assert!(matches!(
        source.try_register_waiter(Arc::downgrade(&observer), &old),
        Err(RegistrationError::Closed)
    ));
    crate::memory::close_user_address_space_handle(old_handle).unwrap();
    let replacement = crate::service::loader::create_user_address_space_handle();
    assert_eq!(old_handle.id(), replacement.id());
    let new = crate::memory::budget::waiter_sponsor(replacement.id());
    let fresh = source.try_register_waiter(Arc::downgrade(&observer), &new).unwrap();
    drop(token);
    assert_eq!(old.used(), 0);
    assert_eq!(new.used(), 1);
    drop(fresh);
    crate::memory::close_user_address_space_handle(replacement).unwrap();
    close_cap(SERVER, endpoint).unwrap();
    close_cap(CLIENT, connection).unwrap();
    close_address_space(SERVER).unwrap();
    close_address_space(CLIENT).unwrap();
    assert_eq!(waiter_budget::node_used(), baseline);
    crate::logln!(
        "[ipc waiters] SUCCESS: source bounds, cancellation/rearm, message/reply/close \
         notifications, loan revocation and retired sponsorship"
    );
}

pub(crate) fn test_scheduled_cleanup() {
    use crate::cpu::scheduler::{
        system_scheduler::{
            Error,
            SYSTEM_SCHEDULER,
            get_thread_id,
        },
        threads::{
            MASTER_THREAD_TABLE,
            ThreadState,
        },
    };
    let (endpoint, connection, id) = fixture();
    let interrupts_before = crate::cpu::isa::lp::ops::get_int_state();
    {
        let outer = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        assert!(!crate::cpu::isa::lp::ops::get_int_state());
        let inner = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        drop(inner);
        assert!(!crate::cpu::isa::lp::ops::get_int_state());
        drop(outer);
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), interrupts_before);
    let source = EndpointObservable {
        endpoint: id,
    };
    let call_cap = scalar_call(CLIENT, connection, 1, 2).unwrap();
    let call = pending_call_id(CLIENT, call_cap).unwrap();
    let call_source = PendingCallObservable {
        call,
    };
    let reply_cap = receive(SERVER, endpoint).unwrap().reply.unwrap();
    for _ in 0..64 {
        assert!(!wait_reply_timeout(CLIENT, call_cap, 1).unwrap());
        assert_eq!(IPC.read().pending_calls[&call].observers.registered(), 0);
        assert!(!crate::cpu::scheduler::block_until(&source, 1, || {
            endpoint_is_readable_or_closed(id).unwrap()
        }));
        assert_eq!(IPC.read().endpoints[&id].readiness_observers.registered(), 0);
    }
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| {});
    let tid = get_thread_id().unwrap();
    for source in [&source as &dyn Observable, &call_source as &dyn Observable] {
        let mut tokens = Vec::new();
        for _ in 0..waiter_budget::SOURCE_LIMIT {
            tokens.push(source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
        }
        let constraints = MASTER_THREAD_TABLE.read().get(tid).unwrap().migration_constraints;
        assert!(matches!(
            SYSTEM_SCHEDULER.read().block_thread(tid, source),
            Err(Error::WaitRegistrationFailed)
        ));
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert!(matches!(thread.state, ThreadState::Running(_)));
        assert_eq!(thread.migration_constraints, constraints);
        drop(table);
        assert!(!crate::cpu::scheduler::block_until(source, 1, || false));
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), interrupts_before);
        drop(tokens);
    }
    // Helpers cannot produce until a rejection is observed. This guarantees
    // untimed fallback coverage without assumptions about scheduling order.
    let mut receiver_tokens = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        receiver_tokens
            .push(source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    RECOVERY_CAP.store(connection, Ordering::Release);
    RECOVERY_RETRY_BASE
        .store(IPC_WAIT_ADMISSION_RETRIES[0].load(Ordering::Relaxed), Ordering::Release);
    crate::cpu::scheduler::spawn_thread_on_lp(
        crate::memory::KERNEL_ASID,
        recovery_sender,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    wait_readable(SERVER, endpoint).unwrap();
    assert_eq!(receive(SERVER, endpoint).unwrap().arg0, 99);
    assert_eq!(IPC.read().endpoints[&id].readiness_observers.registered(), 0);
    drop(receiver_tokens);

    close_cap(SERVER, reply_cap).unwrap();
    close_cap(CLIENT, call_cap).unwrap();
    close_cap(SERVER, endpoint).unwrap();
    close_cap(CLIENT, connection).unwrap();
    close_address_space(SERVER).unwrap();
    close_address_space(CLIENT).unwrap();
    // A real loan must remain available to the server throughout admission
    // pressure and be revoked by reply before the caller returns.
    let server = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(server.id(), 1, 1, 4).unwrap();
    let owner = crate::service::loader::create_user_address_space_handle();
    let connection_owner =
        connection_delegate(server.id(), endpoint, owner.id(), ConnectionRights::CALL).unwrap();
    let memory = crate::memory::object::allocate(owner.id(), 1).unwrap();
    let call_cap =
        scalar_call_with_memory_borrow_read(owner.id(), connection_owner, 1, 2, memory).unwrap();
    let request = receive(server.id(), endpoint).unwrap();
    let call = pending_call_id(owner.id(), call_cap).unwrap();
    let source = PendingCallObservable {
        call,
    };
    let mut call_tokens = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        call_tokens.push(source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    RECOVERY_CAP.store(request.reply.unwrap(), Ordering::Release);
    RECOVERY_LOAN.store(request.memory.unwrap(), Ordering::Release);
    RECOVERY_SERVER.store(server.id(), Ordering::Release);
    RECOVERY_RETRY_BASE
        .store(IPC_WAIT_ADMISSION_RETRIES[1].load(Ordering::Relaxed), Ordering::Release);
    crate::cpu::scheduler::spawn_thread_on_lp(
        crate::memory::KERNEL_ASID,
        recovery_replier,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    wait_reply(owner.id(), call_cap).unwrap();
    assert!(crate::memory::object::info(server.id(), request.memory.unwrap()).is_err());
    assert_eq!(IPC.read().pending_calls[&call].observers.registered(), 0);
    drop(call_tokens);
    assert_eq!(poll_reply(owner.id(), call_cap).unwrap().unwrap().result, 42);
    close_cap(owner.id(), call_cap).unwrap();
    crate::memory::close_user_address_space_handle(owner).unwrap();
    crate::memory::close_user_address_space_handle(server).unwrap();
    crate::logln!(
        "[ipc waiters] SUCCESS: 64 reply/readiness timeout cleanups; non-mutating rejection; \
         forced untimed receive/reply recovery with a live loan"
    );
}

// Kernel fixture IDs, not userspace owners. The verifier retains ownership
// and closes them; helpers borrow these identifiers only for one operation.
static RECOVERY_CAP: AtomicU64 = AtomicU64::new(0);
static RECOVERY_LOAN: AtomicU64 = AtomicU64::new(0);
static RECOVERY_RETRY_BASE: AtomicU64 = AtomicU64::new(0);
static RECOVERY_SERVER: AtomicUsize = AtomicUsize::new(0);

fn await_retry(index: usize) {
    let expected = RECOVERY_RETRY_BASE.load(Ordering::Acquire);
    let deadline = crate::self_test::results::Deadline::after_millis(5_000);
    while IPC_WAIT_ADMISSION_RETRIES[index].load(Ordering::Relaxed) == expected {
        deadline.assert_pending("untimed IPC admission fallback exercised");
        crate::cpu::scheduler::yield_lp();
    }
}

extern "C" fn recovery_sender() {
    await_retry(0);
    scalar_send(CLIENT, RECOVERY_CAP.load(Ordering::Acquire), 1, 99).unwrap();
}

extern "C" fn recovery_replier() {
    await_retry(1);
    let server = RECOVERY_SERVER.load(Ordering::Acquire);
    assert!(crate::memory::object::info(server, RECOVERY_LOAN.load(Ordering::Acquire)).is_ok());
    reply(server, RECOVERY_CAP.load(Ordering::Acquire), 42).unwrap();
}
