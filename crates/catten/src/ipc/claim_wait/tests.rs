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
        ThreadStateKind,
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
// Fixture progress only; no stage or return marker proves exit/quiescence.
static WORKER_STAGE: AtomicUsize = AtomicUsize::new(0);
static REGISTERED_BEFORE_PARK: AtomicUsize = AtomicUsize::new(0);
const CLIENT: AddressSpaceId = 0x5d07;

extern "C" fn close_worker() {
    let target = TARGET.lock().take().unwrap();
    WORKER_STAGE.store(1, Ordering::Release);
    let result = match target {
        Target::Capability(asid, cap) => close_cap(asid, cap),
        Target::Endpoint(owner) => owner.finish_with_cleanup(
            memory::object::LoanRevocation::finish,
            |owner| {
                WORKER_STAGE.store(2, Ordering::Release);
                let result = endpoint_close::wait_front(owner);
                WORKER_STAGE.store(3, Ordering::Release);
                result
            },
            attachments::release_one,
        ),
    };
    result.expect("scheduled IPC close failed");
    WORKER_STAGE.store(4, Ordering::Release);
    RESULT.store(1, Ordering::Release);
}

/// Diagnose the captured worker episode without treating a return marker,
/// missing table entry or staged snapshot as exit/physical completion.
fn await_worker_exit(
    tid: crate::cpu::scheduler::threads::ThreadId,
    generation: crate::cpu::scheduler::threads::ThreadGeneration,
    watched: u64,
    interest: &Interest,
) {
    if crate::completion::wait_timeout(CLIENT, watched, 5_000).unwrap() {
        return;
    }
    let live = {
        let table = MASTER_THREAD_TABLE.read();
        table.get(tid).ok().filter(|thread| thread.generation == generation).map(|thread| {
            let state = match thread.state {
                ThreadState::Running(lp) => {
                    (crate::cpu::scheduler::threads::ThreadStateKind::Running, Some(lp))
                }
                ThreadState::Ready(lp) => {
                    (crate::cpu::scheduler::threads::ThreadStateKind::Ready, Some(lp))
                }
                ThreadState::NeedsLpAssignment => {
                    (crate::cpu::scheduler::threads::ThreadStateKind::NeedsLpAssignment, None)
                }
                ThreadState::Blocked(_) => {
                    (crate::cpu::scheduler::threads::ThreadStateKind::Blocked, None)
                }
            };
            (state, thread.affinity_lp, thread.pinned_lp)
        })
    };
    let staged = crate::cpu::scheduler::threads::staged_retirement_snapshot(generation);
    crate::logln!(
        "[IPC claim readiness] exit timeout tid={tid} generation={generation} returned={} \
         stage={} source_closed={} source_registered={} live={live:?} staged={staged:?}",
        RESULT.load(Ordering::Acquire),
        WORKER_STAGE.load(Ordering::Acquire),
        interest.completed(),
        interest.0.registered()
    );
    panic!("exact-generation IPC close worker exit did not complete");
}

pub(crate) fn scheduled() {
    REGISTERED_BEFORE_PARK.store(0, Ordering::Release);
    let producer_lp = crate::cpu::isa::lp::ops::get_lp_id();
    let lp_count = crate::cpu::multiprocessor::get_lp_count();
    let probe_lps = lp_count.min(4);
    // Repeat real claims within a fixed bound, leaving allocator/watchdog pressure
    // to separate tests. Each episode requires exact exit and closes its roots before the next.
    for round in 0..8 {
        for offset in 0..probe_lps {
            let worker_lp = (producer_lp + offset) % lp_count;
            crate::logln!(
                "[IPC claim readiness] round={round} producer_lp={producer_lp} \
                 worker_lp={worker_lp}"
            );
            scheduled_call(worker_lp);
            scheduled_front(worker_lp);
            crate::logln!(
                "[IPC claim readiness] round={round} worker_lp={worker_lp} call and endpoint \
                 roots closed after exact exit watches"
            );
        }
    }
    crate::logln!(
        "[IPC claim readiness] {} scheduled claim/endpoint close episodes completed with exact \
         exit watches",
        8 * probe_lps * 2
    );
    crate::logln!(
        "[IPC claim readiness] sampled registration before Blocked {} time(s); registration alone \
         was never accepted as a parked worker",
        REGISTERED_BEFORE_PARK.load(Ordering::Acquire)
    );
}

/// Registration is prepared before Blocked publication. Observe both phases
/// for this exact worker generation before releasing the held claim. Neither
/// observation proves exit; the existing exact exit watch remains mandatory.
fn await_claim_park(
    token: ReplyTokenId,
    tid: crate::cpu::scheduler::threads::ThreadId,
    generation: crate::cpu::scheduler::threads::ThreadGeneration,
) -> Interest {
    let deadline = scheduler::monotonic_millis().saturating_add(5_000);
    loop {
        let interest = {
            let ipc = IPC.read();
            ipc.reply_tokens[&token]
                .claim_interest
                .as_ref()
                .filter(|source| source.0.list.registered() == 1)
                .map(Source::interest)
        };
        let state = {
            let table = MASTER_THREAD_TABLE.read();
            let thread = table.get(tid).expect("IPC closer disappeared before park observation");
            assert_eq!(
                thread.generation, generation,
                "IPC closer generation changed before park observation"
            );
            match thread.state {
                ThreadState::Blocked(_) => ThreadStateKind::Blocked,
                ThreadState::Running(_) => ThreadStateKind::Running,
                ThreadState::Ready(_) => ThreadStateKind::Ready,
                ThreadState::NeedsLpAssignment => ThreadStateKind::NeedsLpAssignment,
            }
        };
        if let Some(interest) = interest {
            if state == ThreadStateKind::Blocked {
                assert!(!interest.completed(), "fixture released its claim before observing park");
                return interest;
            }
            REGISTERED_BEFORE_PARK.fetch_add(1, Ordering::Relaxed);
        }
        assert!(
            scheduler::monotonic_millis() < deadline,
            "IPC closer did not register and park: tid={tid} generation={generation} \
             state={state:?} stage={} returned={}",
            WORKER_STAGE.load(Ordering::Acquire),
            RESULT.load(Ordering::Acquire)
        );
        // Bounded fixture observation of the publication phase, never a
        // production wait or permission to finish/replay an uncertain close.
        scheduler::sleep_millis(1);
    }
}

fn scheduled_call(worker_lp: crate::cpu::isa::lp::LpId) {
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
    WORKER_STAGE.store(0, Ordering::Release);
    let tid = scheduler::spawn_thread_on_lp(KERNEL_ASID, close_worker, worker_lp);
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let watched =
        crate::completion::observe_thread_exit_with_generation(CLIENT, tid, Some(generation))
            .unwrap();
    let interest = await_claim_park(token, tid, generation);
    assert_eq!(
        memory::close_user_address_space_handle(fixture.caller),
        Err(AddressSpaceCloseError::OperationsInFlight)
    );
    operation.finish_with(|_| panic!("scalar claim has loans")).unwrap();
    await_worker_exit(tid, generation, watched, &interest);
    assert_eq!(RESULT.load(Ordering::Acquire), 1);
    crate::completion::close(CLIENT, watched).unwrap();
    crate::completion::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    fixture.close();
}

fn scheduled_front(worker_lp: crate::cpu::isa::lp::LpId) {
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
    WORKER_STAGE.store(0, Ordering::Release);
    let tid = scheduler::spawn_thread_on_lp(KERNEL_ASID, close_worker, worker_lp);
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let watched =
        crate::completion::observe_thread_exit_with_generation(CLIENT, tid, Some(generation))
            .unwrap();
    let interest = await_claim_park(token, tid, generation);
    for root in [caller, server] {
        assert_eq!(
            memory::close_user_address_space_handle(root),
            Err(AddressSpaceCloseError::OperationsInFlight)
        );
    }
    operation.finish_with(memory::object::LoanRevocation::finish).unwrap();
    await_worker_exit(tid, generation, watched, &interest);
    assert_eq!(RESULT.load(Ordering::Acquire), 1);
    crate::completion::close(CLIENT, watched).unwrap();
    crate::completion::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    assert!(!memory::object::info(caller.id(), memory).unwrap().lent);
    for root in [caller, server] {
        memory::close_user_address_space_handle(root).unwrap();
    }
}
