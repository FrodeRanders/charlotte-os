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
