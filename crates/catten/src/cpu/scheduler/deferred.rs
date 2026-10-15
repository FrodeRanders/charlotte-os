//! Per-producer-LP delivery of already-admitted scheduler waiter nodes.
//! Producers may hold IPC/other masking guards: enqueue without callbacks or
//! allocation. Only that LP claims after guard exit, so another LP cannot run
//! a callback before its producer has left serialization. This is metadata
//! transport, not custody or authority for any physical cleanup transaction.

use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::{
        collections::retirement_list::{
            RetiredEntry,
            RetirementList,
        },
        observer::{
            registration::{
                ListRef,
                NotificationBatch,
            },
            waiter_budget,
        },
    },
};

pub(crate) type WaiterList = ListRef<waiter_budget::Charge>;
type Waiters = NotificationBatch<waiter_budget::Charge>;

struct Pending {
    waiters: Waiters,
    // A close-interest source admits this storage before registry publication.
    // Retain its original allocation/charge until post-guard disposal, even
    // if no scheduler subscription was successfully installed.
    sources: RetirementList<WaiterList>,
}

impl Pending {
    const fn new() -> Self {
        Self {
            waiters: Waiters::empty(),
            sources: RetirementList::new(),
        }
    }
}

struct Queue {
    pending: AtomicBool,
    batches: Mutex<Pending>,
}

impl Queue {
    const fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            batches: Mutex::new(Pending::new()),
        }
    }
}

static QUEUES: [Queue; super::system_scheduler::MAX_TRACKED_LPS] =
    [const { Queue::new() }; super::system_scheduler::MAX_TRACKED_LPS];

pub(crate) fn publish(waiters: Waiters, source: Option<RetiredEntry<WaiterList>>) {
    if waiters.is_empty() && source.is_none() {
        return;
    }
    // Bind enqueue to this LP before any preemption/migration can intervene.
    // Nested producer masks remain unchanged when this setup leaves.
    let _setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
    let queue = &QUEUES[crate::cpu::isa::lp::ops::get_lp_id() as usize];
    let mut pending = queue.batches.lock();
    // append traverses only this admitted source batch (at most SOURCE_LIMIT
    // for the sources here); no teardown snapshot or queue growth allocation.
    pending.waiters.append(waiters);
    if let Some(source) = source {
        pending.sources.push(source);
    }
    queue.pending.store(true, Ordering::Release);
}

/// Entry requires producer/subsystem guards to have left. Preserve IRQ state;
/// dispatch may follow a masked switch, but never performs physical cleanup.
pub(crate) fn drain() {
    let setup = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
    let queue = &QUEUES[crate::cpu::isa::lp::ops::get_lp_id() as usize];
    if !queue.pending.load(Ordering::Acquire) {
        return;
    }
    let mut pending = {
        let mut guard = queue.batches.lock();
        queue.pending.store(false, Ordering::Release);
        core::mem::replace(&mut *guard, Pending::new())
    };
    drop(setup);
    pending.waiters.notify();
    while let Some(source) = pending.sources.pop() {
        source.release();
    }
}

// One scheduled fixture arms a pre-admitted batch for an exact first dispatch.
// Enqueue under the dispatch guards, after every earlier delivery boundary:
// the incoming context itself must deliver it before entering its worker.
static FIRST_DISPATCH: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static FIRST_BATCH: Mutex<Option<Waiters>> = Mutex::new(None);
static FIRST_DELIVERED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

pub(super) fn test_dispatch(generation: super::threads::ThreadGeneration) {
    if FIRST_DISPATCH.load(Ordering::Acquire) == generation
        && FIRST_DISPATCH
            .compare_exchange(generation, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        publish(FIRST_BATCH.lock().take().expect("missing first-dispatch batch"), None);
    }
}

pub(crate) fn test_first_entry(asid: crate::memory::AddressSpaceId) {
    use alloc::sync::Arc;

    use crate::{
        completion,
        cpu::{
            isa::lp::ops,
            scheduler::{
                system_scheduler::{
                    SYSTEM_SCHEDULER,
                    publish_thread,
                },
                threads::{
                    MASTER_THREAD_TABLE,
                    Thread,
                },
            },
        },
        klib::observer::{
            CallOnNotify,
            Observer,
            WaitSponsor,
            waiter_source::WaiterSource,
        },
        memory::KERNEL_ASID,
    };

    extern "C" fn first_entry() {
        assert_eq!(
            FIRST_DELIVERED.load(Ordering::Acquire),
            1,
            "fresh context stranded its dispatch-boundary readiness delivery"
        );
    }

    for lp in 0..crate::cpu::multiprocessor::get_lp_count().min(4) {
        let thread = Thread::new(KERNEL_ASID, first_entry);
        let generation = thread.generation;
        let tid = publish_thread(thread).unwrap();
        let watched =
            completion::observe_thread_exit_with_generation(asid, tid, Some(generation)).unwrap();
        let source = WaiterSource::new();
        let sponsor = WaitSponsor::new(false);
        let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
            assert!(!ops::get_int_state(), "dispatch delivery restored IRQs prematurely");
            // Reentry requires dispatch guards to have left; a table lookup
            // here is diagnostic, never authority to release the stack/root.
            assert_eq!(MASTER_THREAD_TABLE.write().get(tid).unwrap().generation, generation);
            FIRST_DELIVERED.fetch_add(1, Ordering::Release);
        });
        let token = source.register(Arc::downgrade(&observer), &sponsor).unwrap();
        FIRST_DELIVERED.store(0, Ordering::Release);
        assert!(FIRST_BATCH.lock().replace(source.drain()).is_none());
        assert_eq!(FIRST_DISPATCH.swap(generation, Ordering::AcqRel), 0);
        SYSTEM_SCHEDULER.read().submit_to_lp(tid, lp).unwrap();
        assert!(completion::wait_timeout(asid, watched, 5_000).unwrap());
        assert_eq!(
            completion::poll(asid, watched).unwrap().unwrap().result,
            completion::OpResult::Ok(0)
        );
        completion::close(asid, watched).unwrap();
        assert_eq!(FIRST_DISPATCH.load(Ordering::Acquire), 0);
        assert!(FIRST_BATCH.lock().is_none());
        assert_eq!(FIRST_DELIVERED.load(Ordering::Acquire), 1);
        drop(token);
        assert_eq!(sponsor.used(), 0);
    }
    crate::logln!(
        "[scheduler first entry] exact-dispatch readiness delivered before fresh kernel entry on \
         all selected LPs; exact exit watches completed"
    );
}

/// A queued precursor must run before a wait installs its subscription, even
/// when restoring the park mask could preempt the producer before yield.
/// No watchdog saturation, physical cleanup or replacement wake is involved.
pub(crate) fn test_park_delivery(asid: crate::memory::AddressSpaceId) {
    use alloc::sync::{
        Arc,
        Weak,
    };
    use core::sync::atomic::AtomicUsize;

    use crate::{
        completion,
        cpu::{
            isa::lp::ops::{
                self,
                get_int_state,
            },
            multiprocessor::interrupt_tracking::LocalInterruptMask,
            scheduler::{
                system_scheduler::{
                    Error,
                    SYSTEM_SCHEDULER,
                    get_thread_id,
                    publish_thread,
                },
                threads::{
                    MASTER_THREAD_TABLE,
                    MigrationConstraint,
                    Thread,
                    ThreadState,
                },
            },
        },
        klib::observer::{
            CallOnNotify,
            Observable,
            Observer,
            WaitRegistration,
            WaitSponsor,
            registration::RegistrationError,
            waiter_source::WaiterSource,
        },
        memory::KERNEL_ASID,
    };

    struct CheckedReject(Arc<AtomicUsize>);
    impl Observable for CheckedReject {
        fn try_register_waiter(
            &self,
            _: Weak<dyn Observer>,
            _: &WaitSponsor,
        ) -> Result<WaitRegistration, RegistrationError> {
            assert_eq!(self.0.load(Ordering::Acquire), 1, "park stranded producer delivery");
            Err(RegistrationError::ResourceLimit)
        }
    }

    drain();
    extern "C" fn preemption_peer() {}

    let tid = get_thread_id().unwrap();
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let original_irq = get_int_state();
    for preempted in [false, true] {
        for masked in [false, true] {
            for timed in [false, true] {
                // Admit all callback/list storage before the fixture's local mask.
                let source = WaiterSource::new();
                let sponsor = WaitSponsor::new(false);
                let hits = Arc::new(AtomicUsize::new(0));
                let count = hits.clone();
                let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
                    assert!(!get_int_state(), "park delivery lost the publication mask");
                    // A timer switch may drain this producer-LP batch on an
                    // incoming context while the publisher is Ready. It must
                    // still precede this exact producer's Blocked publication.
                    let table = MASTER_THREAD_TABLE.write();
                    let producer = table.get(tid).unwrap();
                    assert_eq!(producer.generation, generation);
                    assert!(matches!(
                        producer.state,
                        ThreadState::Running(_) | ThreadState::Ready(_)
                    ));
                    if preempted {
                        assert!(matches!(producer.state, ThreadState::Ready(_)));
                    }
                    drop(table);
                    count.fetch_add(1, Ordering::Release);
                });
                let token = source.register(Arc::downgrade(&observer), &sponsor).unwrap();
                let peer = preempted.then(|| {
                    let thread = Thread::new(KERNEL_ASID, preemption_peer);
                    let generation = thread.generation;
                    let tid = publish_thread(thread).unwrap();
                    let exited = completion::observe_thread_exit_with_generation(
                        asid,
                        tid,
                        Some(generation),
                    )
                    .unwrap();
                    (tid, exited)
                });
                let outer = masked.then(LocalInterruptMask::new);
                let irq = get_int_state();
                // Keep the zero-hit observation atomic with publication. Otherwise
                // legitimate IRQ-tail delivery can also race this assertion.
                let publication = LocalInterruptMask::new();
                if let Some((peer, _)) = peer {
                    SYSTEM_SCHEDULER.read().submit_to_lp(peer, ops::get_lp_id()).unwrap();
                }
                publish(source.drain(), None);
                assert_eq!(hits.load(Ordering::Acquire), 0);
                if preempted {
                    // Model timer preemption directly: cooperative yield drains
                    // before switching and would miss the Ready-producer case.
                    SYSTEM_SCHEDULER.read().get_lp_scheduler().lock().set_ctx_switch_pending();
                    ops::cond_yield_lp();
                    assert_eq!(hits.load(Ordering::Acquire), 1);
                }
                drop(publication);
                let checked = CheckedReject(hits.clone());
                let result = if timed {
                    super::try_block_until(&checked, 5_000, || false).map(|_| ())
                } else {
                    super::try_park_until(&checked, MigrationConstraint::GeneralWait, || false)
                };
                assert!(matches!(result, Err(Error::WaitRegistrationFailed)));
                assert_eq!(get_int_state(), irq, "wait changed the enclosing IRQ policy");
                assert_eq!(hits.load(Ordering::Acquire), 1);
                assert_eq!(sponsor.used(), 0);
                assert!(matches!(
                    MASTER_THREAD_TABLE.read().get(tid).unwrap().state,
                    ThreadState::Running(_)
                ));
                drop(token);
                drop(outer);
                assert_eq!(get_int_state(), original_irq);
                if let Some((_, exited)) = peer {
                    assert!(completion::wait_timeout(asid, exited, 5_000).unwrap());
                    assert_eq!(
                        completion::poll(asid, exited).unwrap().unwrap().result,
                        completion::OpResult::Ok(0)
                    );
                    completion::close(asid, exited).unwrap();
                }
            }
        }
    }
    crate::logln!(
        "[scheduler park delivery] timed/event-only precursor delivery before registration, \
         retaining nested IRQ masks and admission rejection; four forced Ready-producer controls \
         passed"
    );
}
