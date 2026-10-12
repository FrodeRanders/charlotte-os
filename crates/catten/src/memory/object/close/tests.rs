//! Raw kernel fixtures; target roots remain owned by the fixture. Only the
//! scheduled child consumes the selected cap. Fixture polling has a deadline
//! and verifies that production close actually parked without a watchdog.

use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    cpu::scheduler::{
        self,
        threads::{
            MASTER_THREAD_TABLE,
            ThreadState,
        },
    },
    klib::observer::CallOnNotify,
    memory::{
        AddressSpaceHandle,
        budget,
    },
};

fn roots() -> [AddressSpaceHandle; 2] {
    core::array::from_fn(|_| crate::service::loader::create_user_address_space_handle())
}

fn finish_roots(roots: [AddressSpaceHandle; 2]) {
    for root in roots {
        crate::memory::close_user_address_space_handle(root).unwrap();
    }
}

pub(crate) fn serialized(hold_ipc: fn(&mut dyn FnMut()), ipc_available: fn() -> bool) {
    deferred::drain();
    for published in [false, true] {
        let baseline = crate::klib::observer::list_budget::node_used();
        let [owner, target] = roots();
        let cap = allocate(owner.id(), 1).unwrap();
        write_bytes(owner.id(), cap, &[0xa5; 32]).unwrap();
        let mut transfer = Some(prepare_move(owner.id(), cap, target.id()).unwrap());
        let destination = transfer.as_ref().unwrap().target_cap();
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
            assert!(ipc_available());
            assert!(MEMORY_OBJECTS.try_lock().is_some());
            assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
            assert!(MASTER_THREAD_TABLE.try_write().is_some());
            count.fetch_add(1, Ordering::Relaxed);
        });
        let sponsor = WaitSponsor::new(false);
        let mut token = None;
        let mut late = None;
        let mut waits = 0;
        let result = run(owner.id(), cap, |interest| {
            assert_eq!(
                crate::memory::close_user_address_space_handle(owner),
                Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
            );
            token =
                Some(interest.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
            if published {
                commit_transfers(core::slice::from_mut(transfer.as_mut().unwrap())).unwrap();
            }
            hold_ipc(&mut || {
                drop(transfer.take().unwrap());
                assert!(interest.completed());
                assert_eq!(hits.load(Ordering::Relaxed), 0);
            });
            // A completed source is immediate readiness, with no new entry.
            assert!(
                interest
                    .try_register_waiter(Arc::downgrade(&observer), &sponsor)
                    .unwrap()
                    .is_ready()
            );
            assert_eq!(sponsor.used(), 1, "detached notification retains its original charge");
            late = Some(Interest(interest.0.clone()));
            deferred::drain();
            assert_eq!(hits.load(Ordering::Relaxed), 1);
            assert_eq!(sponsor.used(), 0);
            waits += 1;
            Ok(())
        });
        assert_eq!(
            result,
            if published {
                Err(MemoryObjectError::UnknownCapability)
            } else {
                Ok(())
            }
        );
        assert_eq!(waits, 1);
        drop((token, late));
        if published {
            assert_eq!(snapshot_bytes(target.id(), destination, 32).unwrap(), [0xa5; 32]);
            close_cap(target.id(), destination).unwrap();
        }
        assert_eq!(budget::used(owner), budget::Amount::default());
        finish_roots([owner, target]);
        assert_eq!(crate::klib::observer::list_budget::node_used(), baseline);
    }
    rejection_and_race();
    crate::logln!(
        "[memory close readiness] exact-root wait, rollback/move, closed-source late bind, \
         IPC-deferred notification/list disposal, saturation and preparation race passed"
    );
}

fn rejection_and_race() {
    let baseline = crate::klib::observer::list_budget::node_used();
    for failure in 0..3 {
        let [owner, target] = roots();
        let cap = allocate(owner.id(), 1).unwrap();
        let mut transfer = Some(prepare_move(owner.id(), cap, target.id()).unwrap());
        let sponsor = WaitSponsor::new(false);
        let observer: Arc<dyn Observer> = CallOnNotify::new(|| panic!("cancelled waiter notified"));
        let mut tokens = Vec::new();
        let result = run_with_prepare(
            owner.id(),
            cap,
            |interest| {
                assert_eq!(
                    crate::memory::close_user_address_space_handle(owner),
                    Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
                );
                for _ in 0..waiter_budget::SOURCE_LIMIT {
                    tokens.push(
                        interest.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap(),
                    );
                }
                assert_eq!(
                    interest.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap_err(),
                    RegistrationError::ResourceLimit
                );
                Err(MemoryObjectError::ResourceLimit)
            },
            |platform| {
                if failure == 0 {
                    return Err(MemoryObjectError::ResourceLimit);
                }
                let source = Source::prepare(platform)?;
                if failure == 2 {
                    // Fence completion wins between preparation and relinking.
                    drop(transfer.take().unwrap());
                }
                Ok(source)
            },
        );
        assert_eq!(
            result,
            if failure == 2 {
                Ok(())
            } else {
                Err(MemoryObjectError::ResourceLimit)
            }
        );
        drop(tokens);
        assert_eq!(sponsor.used(), 0);
        drop(transfer.take());
        if failure == 1 {
            // Even with no surviving waiter, source allocation remains in its
            // admitted disposal node until producer-LP post-guard dispatch.
            assert_eq!(crate::klib::observer::list_budget::node_used().0, baseline.0 + 1);
        }
        deferred::drain();
        if failure != 2 {
            close_cap(owner.id(), cap).unwrap();
        }
        assert_eq!(budget::used(owner), budget::Amount::default());
        finish_roots([owner, target]);
        assert_eq!(crate::klib::observer::list_budget::node_used(), baseline);
    }
}

// Borrowed raw fixture target; the parent retains both exact root owners and
// the transfer until the child is observably parked. No production registry.
static SCHEDULED_TARGET: crate::memory::Mutex<Option<(AddressSpaceId, MemoryObjectCap)>> =
    crate::memory::Mutex::new(None);
static SCHEDULED_RESULT: AtomicUsize = AtomicUsize::new(0);
const SCHEDULED_CLIENT: AddressSpaceId = 0x5d06;

extern "C" fn close_worker() {
    let (owner, cap) = SCHEDULED_TARGET.lock().take().unwrap();
    SCHEDULED_RESULT.store(1, Ordering::Release); // Entered the close operation.
    close_cap(owner, cap).expect("scheduled memory close failed");
    SCHEDULED_RESULT.store(2, Ordering::Release);
}

pub(crate) fn scheduled() {
    crate::completion::open_address_space(SCHEDULED_CLIENT, 4);
    let [owner, target] = roots();
    let cap = allocate(owner.id(), 1).unwrap();
    let transfer = prepare_move(owner.id(), cap, target.id()).unwrap();
    let object = MEMORY_OBJECTS.lock().caps[&owner.id()].caps[&cap].object;
    *SCHEDULED_TARGET.lock() = Some((owner.id(), cap));
    SCHEDULED_RESULT.store(0, Ordering::Release);
    let tid = scheduler::spawn_thread_on_lp(
        crate::memory::KERNEL_ASID,
        close_worker,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let watched = crate::completion::observe_thread_exit_with_generation(
        SCHEDULED_CLIENT,
        tid,
        Some(generation),
    )
    .unwrap();
    let deadline = scheduler::monotonic_millis().saturating_add(5_000);
    loop {
        let registered = MEMORY_OBJECTS.lock().objects[&object]
            .close_interest
            .as_ref()
            .is_some_and(|source| source.0.list.registered() == 1);
        if registered {
            break;
        }
        assert!(scheduler::monotonic_millis() < deadline, "memory closer never registered");
        scheduler::sleep_millis(1); // Bounded fixture sampling, never production close.
    }
    assert!(matches!(MASTER_THREAD_TABLE.read().get(tid).unwrap().state, ThreadState::Blocked(_)));
    assert_eq!(
        crate::memory::close_user_address_space_handle(owner),
        Err(crate::memory::AddressSpaceCloseError::OperationsInFlight)
    );
    drop(transfer); // Rollback closes the exact source and queues its admitted wake.
    let result = crate::completion::wait_timeout(SCHEDULED_CLIENT, watched, 5_000);
    if result != Ok(true) {
        // Copied exact-generation observations, never completion/recovery
        // authority. Release every inspected registry before logging/panic.
        let state = {
            let table = MASTER_THREAD_TABLE.read();
            table.get(tid).ok().filter(|thread| thread.generation == generation).map(|thread| {
                match thread.state {
                    ThreadState::NeedsLpAssignment => "unassigned",
                    ThreadState::Ready(_) => "ready",
                    ThreadState::Running(_) => "running",
                    ThreadState::Blocked(_) => "blocked",
                }
            })
        };
        let staged = crate::cpu::scheduler::threads::staged_retirement_snapshot(generation);
        let completion = crate::completion::state_of(SCHEDULED_CLIENT, watched);
        let phase = SCHEDULED_RESULT.load(Ordering::Acquire);
        crate::logln!(
            "[memory close readiness] rejected exit wait: result={result:?} tid={tid} \
             generation={generation} worker_phase={phase} exact_state={state:?} staged={staged:?} \
             completion={completion:?}"
        );
    }
    assert_eq!(result, Ok(true), "scheduled close must observe exact exit within its deadline");
    assert_eq!(SCHEDULED_RESULT.load(Ordering::Acquire), 2);
    crate::completion::close(SCHEDULED_CLIENT, watched).unwrap();
    crate::completion::close_address_space(SCHEDULED_CLIENT);
    crate::capability::close_address_space(SCHEDULED_CLIENT);
    assert_eq!(budget::used(owner), budget::Amount::default());
    finish_roots([owner, target]);
    crate::logln!(
        "[memory close readiness] scheduled closer parked on the exact transfer; rollback woke it \
         and both roots closed"
    );
}
