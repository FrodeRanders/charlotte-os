//! Raw ABI fixtures retain the roots and active claim. Only the scheduled
//! worker consumes its selected close capability. Sampling is fixture-only.

use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    cpu::scheduler::threads::{
        MASTER_THREAD_TABLE,
        ThreadState,
    },
    klib::observer::CallOnNotify,
    memory::{
        self,
        AddressSpaceCloseError,
    },
};

fn unlocked() {
    assert!(IPC.try_write().is_some());
    assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(MASTER_THREAD_TABLE.try_write().is_some());
}

/// Existing loan/source fixtures supply their real completion or failure.
/// This observes the exact active claim without invoking production park.
pub(in crate::ipc) fn observe(asid: AddressSpaceId, cap: CapabilityId, finish: impl FnOnce()) {
    deferred::drain();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        unlocked();
        count.fetch_add(1, Ordering::Relaxed);
    });
    let sponsor = WaitSponsor::new(false);
    let mut owner = Wait::new();
    let mut token = None;
    owner
        .wait_with(
            asid,
            cap,
            |interest| {
                token = Some(
                    interest.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap(),
                );
                finish();
                assert!(interest.completed());
                assert_eq!(
                    hits.load(Ordering::Relaxed),
                    0,
                    "claim callbacks ran before deferred dispatch"
                );
                assert!(
                    interest
                        .try_register_waiter(Arc::downgrade(&observer), &sponsor)
                        .unwrap()
                        .is_ready()
                );
                deferred::drain();
                assert_eq!(hits.load(Ordering::Relaxed), 1);
                Ok(())
            },
            Source::prepare,
        )
        .unwrap();
    owner.finish(Ok(())).unwrap();
    drop(token);
    assert_eq!(sponsor.used(), 0);
}

pub(in crate::ipc) fn preparation_race(
    asid: AddressSpaceId,
    cap: CapabilityId,
    mut replace: impl FnMut(),
) {
    let mut owner = Wait::new();
    let result = owner.wait_with(
        asid,
        cap,
        |_| panic!("bound a successor claim episode"),
        |platform| {
            replace();
            Source::prepare(platform)
        },
    );
    assert_eq!(result, Ok(()));
    owner.finish(result).unwrap();
}

struct Fixture {
    caller: AddressSpaceHandle,
    server: AddressSpaceHandle,
    call: CapabilityId,
    reply: CapabilityId,
}

impl Fixture {
    fn new() -> Self {
        let caller = crate::service::loader::create_user_address_space_handle();
        let server = crate::service::loader::create_user_address_space_handle();
        let endpoint = endpoint_create(server.id(), 0x434c_4149, 1, 4).unwrap();
        let connection =
            connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
        let call = scalar_call(caller.id(), connection, 1, 0).unwrap();
        let reply = receive(server.id(), endpoint).unwrap().reply.unwrap();
        Self {
            caller,
            server,
            call,
            reply,
        }
    }

    fn close(self) {
        memory::close_user_address_space_handle(self.caller).unwrap();
        memory::close_user_address_space_handle(self.server).unwrap();
    }
}

pub(crate) fn serialized() {
    deferred::drain();
    let baseline = crate::klib::observer::list_budget::node_used();
    for failure in 0..3 {
        let fixture = Fixture::new();
        let mut operation = Some(
            cancellation::PreparedCancellation::prepare(fixture.server.id(), fixture.reply)
                .unwrap(),
        );
        let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("canceled waiter notified"));
        let sponsor = WaitSponsor::new(false);
        let mut owner = Wait::new();
        let result = owner.wait_with(
            fixture.caller.id(),
            fixture.call,
            |interest| {
                assert_eq!(
                    memory::close_user_address_space_handle(fixture.caller),
                    Err(AddressSpaceCloseError::OperationsInFlight)
                );
                let mut tokens = Vec::new();
                for _ in 0..waiter_budget::SOURCE_LIMIT {
                    tokens.push(
                        interest.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap(),
                    );
                }
                assert!(matches!(
                    interest.try_register_waiter(Arc::downgrade(&observer), &sponsor),
                    Err(RegistrationError::ResourceLimit)
                ));
                drop(tokens);
                if failure == 0 {
                    operation
                        .take()
                        .unwrap()
                        .finish_with(|_| panic!("scalar claim has loans"))
                        .unwrap();
                    assert!(interest.completed());
                    assert!(
                        interest
                            .try_register_waiter(Arc::downgrade(&observer), &sponsor)
                            .unwrap()
                            .is_ready()
                    );
                    Ok(())
                } else {
                    Err(IpcError::ResourceLimit)
                }
            },
            |platform| {
                if failure == 2 {
                    Err(IpcError::ResourceLimit)
                } else {
                    Source::prepare(platform)
                }
            },
        );
        assert_eq!(
            result,
            if failure == 0 {
                Ok(())
            } else {
                Err(IpcError::ResourceLimit)
            }
        );
        assert_eq!(owner.finish(result), result);
        if let Some(operation) = operation {
            assert_eq!(poll_reply(fixture.caller.id(), fixture.call), Ok(None));
            operation.finish_with(|_| panic!("scalar claim has loans")).unwrap();
        }
        deferred::drain();
        assert_eq!(sponsor.used(), 0);
        fixture.close();
        assert_eq!(crate::klib::observer::list_budget::node_used(), baseline);
    }
    // Completion wins after fallible preparation but before source relinking.
    let fixture = Fixture::new();
    let mut operation = Some(
        cancellation::PreparedCancellation::prepare(fixture.server.id(), fixture.reply).unwrap(),
    );
    let mut owner = Wait::new();
    let result = owner.wait_with(
        fixture.caller.id(),
        fixture.call,
        |_| panic!("completed claim parked"),
        |platform| {
            operation.take().unwrap().finish_with(|_| panic!("scalar claim has loans")).unwrap();
            Source::prepare(platform)
        },
    );
    assert_eq!(result, Ok(()));
    owner.finish(result).unwrap();
    fixture.close();
    assert_eq!(crate::klib::observer::list_budget::node_used(), baseline);
    crate::logln!(
        "[IPC claim readiness] exact-root wait, late closed bind, saturation, admission rejection \
         and completion-before-relink passed"
    );
}

enum Target {
    Capability(AddressSpaceId, CapabilityId),
    Endpoint(endpoint_close::PreparedEndpointClose),
}

static TARGET: memory::Mutex<Option<Target>> = memory::Mutex::new(None);
static RESULT: AtomicUsize = AtomicUsize::new(0);
const CLIENT: AddressSpaceId = 0x5d07;

extern "C" fn close_worker() {
    let target = TARGET.lock().take().unwrap();
    let result = match target {
        Target::Capability(asid, cap) => close_cap(asid, cap),
        Target::Endpoint(owner) => owner.finish_with_cleanup(
            memory::object::LoanRevocation::finish,
            endpoint_close::wait_front,
            attachments::release_one,
        ),
    };
    result.expect("scheduled IPC close failed");
    RESULT.store(1, Ordering::Release);
}

/// Diagnose the captured worker episode without treating a return marker,
/// missing table entry or staged snapshot as exit/physical completion.
fn await_worker_exit(
    tid: crate::cpu::scheduler::threads::ThreadId,
    generation: crate::cpu::scheduler::threads::ThreadGeneration,
    watched: u64,
) {
    if crate::completion::wait_timeout(CLIENT, watched, 5_000).unwrap() {
        return;
    }
    let live = {
        let table = MASTER_THREAD_TABLE.read();
        table.get(tid).ok().filter(|thread| thread.generation == generation).map(|thread| {
            let state = match thread.state {
                ThreadState::Running(lp) => (1, Some(lp)),
                ThreadState::Ready(lp) => (2, Some(lp)),
                ThreadState::NeedsLpAssignment => (3, None),
                ThreadState::Blocked(_) => (4, None),
            };
            (state, thread.affinity_lp, thread.pinned_lp)
        })
    };
    let staged = crate::cpu::scheduler::threads::staged_retirement_snapshot(generation);
    crate::logln!(
        "[IPC claim readiness] exit timeout tid={tid} generation={generation} returned={} \
         live={live:?} staged={staged:?}",
        RESULT.load(Ordering::Acquire)
    );
    panic!("exact-generation IPC close worker exit did not complete");
}

pub(crate) fn scheduled() {
    crate::completion::open_address_space(CLIENT, 4);
    let fixture = Fixture::new();
    let operation =
        cancellation::PreparedCancellation::prepare(fixture.server.id(), fixture.reply).unwrap();
    let token = match IPC.read().cap(fixture.server.id(), fixture.reply).unwrap() {
        Capability::ReplyToken {
            token,
        } => token,
        _ => unreachable!(),
    };
    *TARGET.lock() = Some(Target::Capability(fixture.caller.id(), fixture.call));
    RESULT.store(0, Ordering::Release);
    let tid = scheduler::spawn_thread_on_lp(
        KERNEL_ASID,
        close_worker,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let watched =
        crate::completion::observe_thread_exit_with_generation(CLIENT, tid, Some(generation))
            .unwrap();
    let deadline = scheduler::monotonic_millis().saturating_add(5_000);
    loop {
        let registered = IPC.read().reply_tokens[&token]
            .claim_interest
            .as_ref()
            .is_some_and(|source| source.0.list.registered() == 1);
        if registered {
            break;
        }
        assert!(scheduler::monotonic_millis() < deadline, "IPC closer never registered");
        scheduler::sleep_millis(1); // Bounded fixture sampling only.
    }
    assert!(matches!(MASTER_THREAD_TABLE.read().get(tid).unwrap().state, ThreadState::Blocked(_)));
    assert_eq!(
        memory::close_user_address_space_handle(fixture.caller),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    operation.finish_with(|_| panic!("scalar claim has loans")).unwrap();
    await_worker_exit(tid, generation, watched);
    assert_eq!(RESULT.load(Ordering::Acquire), 1);
    crate::completion::close(CLIENT, watched).unwrap();
    crate::completion::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    fixture.close();
    scheduled_front();
    crate::logln!(
        "[IPC claim readiness] scheduled call and owned endpoint closers parked on exact \
         cancellation claims; completion woke both and their roots closed"
    );
}

fn scheduled_front() {
    crate::completion::open_address_space(CLIENT, 4);
    let caller = crate::service::loader::create_user_address_space_handle();
    let server = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(server.id(), 0x434c_4149, 1, 4).unwrap();
    let connection =
        connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
    let memory = memory::object::allocate(caller.id(), 1).unwrap();
    let call = scalar_call_with_memory_borrow_read(caller.id(), connection, 1, 0, memory).unwrap();
    let owner = endpoint_close::PreparedEndpointClose::prepare(server.id(), endpoint).unwrap();
    let operation = cancellation::PreparedCancellation::prepare(caller.id(), call).unwrap();
    let token = {
        let ipc = IPC.read();
        let Capability::PendingCall {
            call,
        } = ipc.cap(caller.id(), call).unwrap()
        else {
            unreachable!()
        };
        ipc.reply_tokens.iter().find_map(|(&id, token)| (token.call == call).then_some(id)).unwrap()
    };
    *TARGET.lock() = Some(Target::Endpoint(owner));
    RESULT.store(0, Ordering::Release);
    let tid = scheduler::spawn_thread_on_lp(
        KERNEL_ASID,
        close_worker,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let watched =
        crate::completion::observe_thread_exit_with_generation(CLIENT, tid, Some(generation))
            .unwrap();
    let deadline = scheduler::monotonic_millis().saturating_add(5_000);
    loop {
        let registered = IPC.read().reply_tokens[&token]
            .claim_interest
            .as_ref()
            .is_some_and(|source| source.0.list.registered() == 1);
        if registered {
            break;
        }
        assert!(scheduler::monotonic_millis() < deadline, "owned endpoint closer never registered");
        scheduler::sleep_millis(1);
    }
    assert!(matches!(MASTER_THREAD_TABLE.read().get(tid).unwrap().state, ThreadState::Blocked(_)));
    for root in [caller, server] {
        assert_eq!(
            memory::close_user_address_space_handle(root),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
    }
    operation.finish_with(memory::object::LoanRevocation::finish).unwrap();
    await_worker_exit(tid, generation, watched);
    assert_eq!(RESULT.load(Ordering::Acquire), 1);
    crate::completion::close(CLIENT, watched).unwrap();
    crate::completion::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    assert!(!memory::object::info(caller.id(), memory).unwrap().lent);
    for root in [caller, server] {
        memory::close_user_address_space_handle(root).unwrap();
    }
}
