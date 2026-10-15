//! Never-scheduled local owners exercise the production candidate scan while
//! both fixture LP locks, the actual master-table lock and the heap are held.
//! No live queue, load summary, thread policy or timer sponsor is replaced.

use super::*;
use crate::{
    cpu::{
        multiprocessor::spin::mutex::Mutex,
        scheduler::threads::MigrationConstraint,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

extern "C" fn unused_entry() {}

fn queue(lp_id: LpId) -> RoundRobin {
    RoundRobin {
        lp_id,
        quantum: ExtDuration::from_millis(10),
        is_idle: true,
        timer_event_observer: Arc::new(super::super::TimerEventObserver::new()),
        run_queue: VecDeque::with_capacity(4),
        current_handle: None,
        idle_tid: usize::MAX,
    }
}

fn release(table: &mut ThreadTable, (tid, generation): (ThreadId, ThreadGeneration)) {
    let mut thread = table.take_element(tid).unwrap();
    assert_eq!(thread.generation, generation);
    assert_eq!(thread.dispatch_count, 0);
    assert!(!thread.context.is_on_cpu());
    // Only this private fixture table ever owned the context; Ready was a
    // synthetic selection input, not scheduler publication. Restore its
    // constructor state before the ordinary never-admitted release adapter.
    thread.state = ThreadState::NeedsLpAssignment;
    thread.release_unstarted().unwrap();
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let mut table = ThreadTable::new();
    let identities = core::array::from_fn::<_, 2, _>(|_| {
        let mut thread = Thread::new(KERNEL_ASID, unused_entry);
        thread.migration_safe = true;
        thread.state = ThreadState::Ready(0);
        let generation = thread.generation;
        (table.add_element(thread), generation)
    });
    let [first, second] = identities;
    let mut source = queue(0);
    source.run_queue.extend([
        ThreadHandle {
            tid: usize::MAX,
            generation: 1,
        },
        ThreadHandle {
            tid: first.0,
            generation: first.1.wrapping_add(1),
        },
        ThreadHandle {
            tid: first.0,
            generation: first.1,
        },
        ThreadHandle {
            tid: second.0,
            generation: second.1,
        },
    ]);
    let source = Mutex::new(source);
    let destination = Mutex::new(queue(1));
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let source = source.lock();
        let _destination = destination.lock();
        let _master = MASTER_THREAD_TABLE.write();
        assert_eq!(source.ready_migration_candidate(&table), Some(first));
        assert_eq!(source.run_queue.len(), 4);

        table.get_mut(first.0).unwrap().dispatch_count = 1;
        #[cfg(target_arch = "x86_64")]
        assert_eq!(source.ready_migration_candidate(&table), Some(second));
        #[cfg(target_arch = "aarch64")]
        assert_eq!(source.ready_migration_candidate(&table), Some(first));
        table.get_mut(first.0).unwrap().dispatch_count = 0;

        table.get_mut(first.0).unwrap().pinned_lp = Some(0);
        assert_eq!(source.ready_migration_candidate(&table), Some(second));
        table.get_mut(first.0).unwrap().pinned_lp = None;

        table.get_mut(first.0).unwrap().migration_safe = false;
        assert_eq!(source.ready_migration_candidate(&table), Some(second));
        table.get_mut(first.0).unwrap().migration_safe = true;

        table.get(first.0).unwrap().abort_requested.store(true, Ordering::Release);
        assert_eq!(source.ready_migration_candidate(&table), Some(second));
        table.get(first.0).unwrap().abort_requested.store(false, Ordering::Release);

        table.get(first.0).unwrap().operation_executor_lp.store(0, Ordering::Release);
        assert_eq!(source.ready_migration_candidate(&table), Some(second));
        table.get(first.0).unwrap().operation_executor_lp.store(usize::MAX, Ordering::Release);

        table.get_mut(first.0).unwrap().add_migration_constraint(MigrationConstraint::GeneralWait);
        assert_eq!(source.ready_migration_candidate(&table), Some(second));
        table.get_mut(first.0).unwrap().clear_blocking_migration_constraints();

        for state in
            [ThreadState::Ready(1), ThreadState::Running(0), ThreadState::NeedsLpAssignment]
        {
            table.get_mut(first.0).unwrap().state = state;
            assert_eq!(source.ready_migration_candidate(&table), Some(second));
        }
        table.get_mut(first.0).unwrap().state = ThreadState::Ready(0);

        #[cfg(target_arch = "aarch64")]
        {
            table.get(first.0).unwrap().context.on_cpu.store(1, Ordering::Release);
            assert_eq!(source.ready_migration_candidate(&table), Some(second));
            table.get(first.0).unwrap().context.on_cpu.store(0, Ordering::Release);
        }

        table.get_mut(first.0).unwrap().pinned_lp = Some(0);
        table.get_mut(second.0).unwrap().migration_safe = false;
        assert_eq!(source.ready_migration_candidate(&table), None);
        table.get_mut(first.0).unwrap().pinned_lp = None;
        table.get_mut(second.0).unwrap().migration_safe = true;
        assert_eq!(source.ready_migration_candidate(&table), Some(first));
        assert_eq!(source.run_queue.len(), 4);
    }

    // Reuse a local table slot while the source retains the old generation.
    // These owners never entered a live scheduler, so explicit unstarted
    // cleanup is valid after every local guard has left.
    release(&mut table, first);
    let mut replacement = Thread::new(KERNEL_ASID, unused_entry);
    replacement.migration_safe = true;
    replacement.state = ThreadState::Ready(0);
    assert_ne!(replacement.generation, first.1);
    assert_ne!(replacement.generation, first.1.wrapping_add(1));
    let replacement_generation = replacement.generation;
    assert_eq!(table.add_element(replacement), first.0);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let mut source = source.lock();
        let _destination = destination.lock();
        let _master = MASTER_THREAD_TABLE.write();
        assert_eq!(source.ready_migration_candidate(&table), Some(second));
        source.run_queue.clear();
        assert_eq!(source.ready_migration_candidate(&table), None);
    }
    for identity in [(first.0, replacement_generation), second] {
        release(&mut table, identity);
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[scheduler selection] heap-held load ties, stale/reused generations, policy fences, \
         architecture dispatch gate, exact Ready owner and empty/no-candidate scans passed"
    );
}
