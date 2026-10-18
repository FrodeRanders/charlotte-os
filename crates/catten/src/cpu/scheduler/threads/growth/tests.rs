//! One exact-root EL0 fixture, armed before thread publication. No production
//! registry/custody or caller-controlled classification is introduced.

use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    cpu::{
        isa::lp::ops,
        scheduler::{
            self,
            system_scheduler::SYSTEM_SCHEDULER,
        },
    },
    memory::AddressSpaceHandle,
};

static ROOT_GENERATION: AtomicUsize = AtomicUsize::new(0);
static ROOT_ID: AtomicUsize = AtomicUsize::new(0);
static MODE: AtomicUsize = AtomicUsize::new(0);
static PHASES: AtomicUsize = AtomicUsize::new(0);
static WORK_PHASES: AtomicUsize = AtomicUsize::new(0);
static YIELDS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn arm(root: AddressSpaceHandle, mode: usize) {
    assert_eq!(ROOT_GENERATION.load(Ordering::Acquire), 0);
    ROOT_ID.store(root.id(), Ordering::Relaxed);
    MODE.store(mode, Ordering::Relaxed);
    PHASES.store(0, Ordering::Relaxed);
    YIELDS.store(0, Ordering::Relaxed);
    WORK_PHASES.store(0, Ordering::Relaxed);
    ROOT_GENERATION.store(root.generation(), Ordering::Release);
}

fn matches(root: AddressSpaceHandle) -> bool {
    ROOT_GENERATION.load(Ordering::Acquire) == root.generation()
        && ROOT_ID.load(Ordering::Relaxed) == root.id()
}

pub(super) fn before_growth(root: AddressSpaceHandle, identity: (ThreadId, ThreadGeneration)) {
    if !matches(root) {
        return;
    }
    let irq = ops::get_int_state();
    // Reenter rather than assuming global try-lock availability. All original
    // stack metadata stays readable with its original committed prefix.
    {
        let table = MASTER_THREAD_TABLE.write();
        let thread = table.get(identity.0).unwrap();
        assert_eq!(thread.generation, identity.1);
        assert_eq!(thread.address_space, Some(root));
        assert_eq!(thread.context.user_stack_committed_pages(), 1);
        assert_eq!(thread.operation_executor_lp.load(Ordering::Acquire), ops::get_lp_id() as usize);
        assert!(!thread.is_fully_migratable());
    }
    assert!(matches!(
        ThreadExecution::acquire(),
        Err(crate::cpu::scheduler::system_scheduler::Error::ThreadRetirementFailed)
    ));
    if MODE.load(Ordering::Relaxed) == 1 {
        SYSTEM_SCHEDULER.read().abort_thread_generation(identity.0, identity.1).unwrap();
    }
    // Only the IRQ-qualified fixture schedules here. Masked EL0 faults retain
    // their policy; no IRQ is enabled to manufacture preemption coverage.
    if irq {
        for _ in 0..8 {
            scheduler::yield_lp();
            YIELDS.fetch_add(1, Ordering::Relaxed);
            let table = MASTER_THREAD_TABLE.read();
            let thread = table.get(identity.0).unwrap();
            assert_eq!(thread.generation, identity.1);
            assert_eq!(thread.address_space, Some(root));
            assert!(!thread.abort_ready());
            assert_eq!(
                thread.operation_executor_lp.load(Ordering::Acquire),
                ops::get_lp_id() as usize
            );
        }
    }
    assert_eq!(ops::get_int_state(), irq);
    PHASES.fetch_or(1, Ordering::Release);
}

pub(super) fn before_release(
    root: AddressSpaceHandle,
    identity: (ThreadId, ThreadGeneration),
    low: Option<usize>,
) {
    if !matches(root) {
        return;
    }
    assert!(!ops::get_int_state());
    let collision = MODE.load(Ordering::Relaxed) == 2;
    assert_eq!(low.is_none(), collision);
    let table = MASTER_THREAD_TABLE.read();
    let thread = table.get(identity.0).unwrap();
    assert_eq!(thread.generation, identity.1);
    assert_eq!(thread.address_space, Some(root));
    assert_eq!(
        thread.context.user_stack_committed_pages(),
        if collision {
            1
        } else {
            3
        }
    );
    assert_eq!(thread.operation_executor_lp.load(Ordering::Acquire), ops::get_lp_id() as usize);
    assert!(!thread.abort_ready());
    assert_eq!(thread.abort_requested.load(Ordering::Acquire), MODE.load(Ordering::Relaxed) == 1);
    PHASES.fetch_or(2, Ordering::Release);
}

pub(crate) fn verify(root: AddressSpaceHandle) {
    assert!(matches(root));
    assert_eq!(PHASES.load(Ordering::Acquire), 3);
    assert_eq!(
        WORK_PHASES.load(Ordering::Relaxed),
        if MODE.load(Ordering::Relaxed) == 2 {
            7
        } else {
            3
        }
    );
    let yields = YIELDS.load(Ordering::Relaxed);
    assert!(yields == 0 || yields == 8);
    crate::logln!(
        "[stack growth execution] exact-root handback, table reentry and retained abort/migration \
         fence passed; mode={} irq-qualified-yields={}",
        MODE.load(Ordering::Relaxed),
        yields
    );
    ROOT_GENERATION.store(0, Ordering::Release);
}

// Called immediately before the real allocator, mapper and physical rollback.
// Reacquiring the table checks that its guard is released at each work phase.
pub(crate) fn phase(root: AddressSpaceHandle, bit: usize) {
    if !matches(root) {
        return;
    }
    let irq = ops::get_int_state();
    drop(MASTER_THREAD_TABLE.write());
    assert_eq!(ops::get_int_state(), irq);
    WORK_PHASES.fetch_or(bit, Ordering::Relaxed);
}
