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

/// A queued precursor must run before a wait installs its subscription, even
/// when restoring the park mask could preempt the producer before yield.
/// No watchdog saturation, physical cleanup or replacement wake is involved.
pub(crate) fn test_park_delivery() {
    use alloc::sync::{
        Arc,
        Weak,
    };
    use core::sync::atomic::AtomicUsize;

    use crate::{
        cpu::{
            isa::lp::ops::get_int_state,
            multiprocessor::interrupt_tracking::LocalInterruptMask,
            scheduler::{
                system_scheduler::{
                    Error,
                    get_thread_id,
                },
                threads::{
                    MASTER_THREAD_TABLE,
                    MigrationConstraint,
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
    let tid = get_thread_id().unwrap();
    let original_irq = get_int_state();
    for masked in [false, true] {
        for timed in [false, true] {
            // Admit all callback/list storage before the fixture's local mask.
            let source = WaiterSource::new();
            let sponsor = WaitSponsor::new(false);
            let hits = Arc::new(AtomicUsize::new(0));
            let count = hits.clone();
            let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
                assert!(!get_int_state(), "park delivery lost the publication mask");
                // Reenter the table: delivery precedes registration's guards
                // and Blocked publication, rather than depending on yield.
                assert!(matches!(
                    MASTER_THREAD_TABLE.write().get(tid).unwrap().state,
                    ThreadState::Running(_)
                ));
                count.fetch_add(1, Ordering::Release);
            });
            let token = source.register(Arc::downgrade(&observer), &sponsor).unwrap();
            let outer = masked.then(LocalInterruptMask::new);
            let irq = get_int_state();
            publish(source.drain(), None);
            assert_eq!(hits.load(Ordering::Acquire), 0);
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
        }
    }
    crate::logln!(
        "[scheduler park delivery] timed/event-only precursor delivery before registration, \
         retaining nested IRQ masks and admission rejection passed"
    );
}
