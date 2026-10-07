//! Thread-exit admission, transactional publication and cancellation fixtures.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::{
    CancelState,
    CompletionExitObserver,
    EventSubmission,
    OpResult,
    SubmitError,
    watch_budget as budget,
};
use crate::{
    cpu::scheduler::{
        self,
        system_scheduler::publish_thread,
        threads::{
            MASTER_THREAD_TABLE,
            Thread,
            ThreadGeneration,
            ThreadId,
            exit_source::ExitSource,
        },
    },
    klib::observer::{
        CallOnNotify,
        Observer,
        registration::RegistrationError,
    },
    memory::KERNEL_ASID,
};

const CLIENT: usize = 0x5d00;
const SCHEDULED_CLIENT: usize = 0x5d01;

static WORKER_RUNS: AtomicUsize = AtomicUsize::new(0);
extern "C" fn empty_worker() {
    WORKER_RUNS.fetch_add(1, Ordering::SeqCst);
}

/// A never-admitted kernel thread for deterministic exit/reuse fixtures.
struct Target {
    tid: ThreadId,
    generation: ThreadGeneration,
}
impl Target {
    fn new() -> Self {
        let thread = Thread::new(KERNEL_ASID, empty_worker);
        let generation = thread.generation;
        let tid = publish_thread(thread).unwrap();
        Self {
            tid,
            generation,
        }
    }

    fn count(&self) -> usize {
        MASTER_THREAD_TABLE.read().get(self.tid).unwrap().exit_watch_count()
    }

    fn watch(&self, asid: usize) -> Result<super::CompletionCap, SubmitError> {
        super::observe_thread_exit_with_generation(asid, self.tid, Some(self.generation))
    }
}
impl Drop for Target {
    fn drop(&mut self) {
        let thread = MASTER_THREAD_TABLE.write().take_element(self.tid).unwrap();
        assert_eq!(thread.generation, self.generation);
        drop(thread); // Notification must run outside the master table guard.
    }
}

fn observer(submission: &EventSubmission) -> Arc<dyn Observer> {
    Arc::new(CompletionExitObserver {
        asid: submission.asid,
        cap: submission.cap(),
        result: OpResult::Ok(0),
        completion: Arc::downgrade(submission.completion()),
    })
}

pub(crate) fn test_admission() {
    let baseline = budget::node_used();
    super::open_address_space(CLIENT, 129);
    let account = super::watch_admission(CLIENT).unwrap();
    let records = super::record_admission(CLIENT).unwrap();
    let target = Target::new();
    let mut watches = Vec::new();
    for _ in 0..budget::MAX_THREAD_WATCHES {
        watches.push(target.watch(CLIENT).unwrap());
    }
    assert_eq!(target.count(), 128);
    assert_eq!(target.watch(CLIENT), Err(SubmitError::WouldBlock));
    assert_eq!(account.used(), 128);
    assert_eq!(records.used(), 128, "source rejection must roll back its staged record");
    for cap in watches {
        assert_eq!(super::cancel(CLIENT, cap), Ok(CancelState::CancelRequested));
        assert_eq!(super::poll(CLIENT, cap).unwrap().unwrap().result, OpResult::Cancelled);
        super::close(CLIENT, cap).unwrap();
    }
    assert_eq!(target.count(), 0);
    assert_eq!(account.used(), 0);
    for _ in 0..512 {
        let cap = target.watch(CLIENT).unwrap();
        super::cancel(CLIENT, cap).unwrap();
        super::close(CLIENT, cap).unwrap();
        assert_eq!(target.count(), 0);
    }
    let tid = target.tid;
    let generation = target.generation;
    let cap = target.watch(CLIENT).unwrap();
    // Capture/reenter scheduler and completion registries from an exit callback.
    let called = Arc::new(AtomicUsize::new(0));
    let calls = called.clone();
    let reentrant: Arc<dyn Observer> = CallOnNotify::new(move || {
        assert!(MASTER_THREAD_TABLE.read().get(tid).is_err());
        let cap = super::submit(CLIENT, super::OpCode::Nop, None).unwrap();
        super::complete(CLIENT, cap, OpResult::Ok(7)).unwrap();
        super::close(CLIENT, cap).unwrap();
        calls.fetch_add(1, Ordering::SeqCst);
    });
    let token = scheduler::observe_thread_exit(
        tid,
        Arc::downgrade(&reentrant),
        budget::reserve(&account, false).unwrap(),
    )
    .unwrap();
    drop(target);
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert_eq!(super::poll(CLIENT, cap).unwrap().unwrap().result, OpResult::Ok(0));
    assert_eq!(account.used(), 0, "a retained token must not retain a dead source entry");
    drop(token);
    super::close(CLIENT, cap).unwrap();
    let replacement = Target::new();
    assert_eq!(replacement.tid, tid);
    assert_ne!(replacement.generation, generation);
    let stale = super::observe_thread_exit_with_generation(CLIENT, tid, Some(generation)).unwrap();
    assert_eq!(super::poll(CLIENT, stale).unwrap().unwrap().result, OpResult::Ok(0));
    assert_eq!(replacement.count(), 0);
    super::close(CLIENT, stale).unwrap();
    let retained_cap = replacement.watch(CLIENT).unwrap();
    let retained = super::completion_of(CLIENT, retained_cap).unwrap();
    super::close_address_space(CLIENT);
    assert_eq!(replacement.count(), 0);
    assert_eq!(account.used(), 0);
    assert_eq!(records.used(), 1, "retained completion still owns its record charge");
    drop(retained);
    assert_eq!(records.used(), 0);
    drop(replacement);

    // The watch account is shared across event types, not an extra allowance.
    super::open_address_space(CLIENT, 2);
    let account = super::watch_admission(CLIENT).unwrap();
    let target = Target::new();
    let charge1 = budget::reserve(&account, false).unwrap();
    let charge2 = budget::reserve(&account, false).unwrap();
    assert_eq!(target.watch(CLIENT), Err(SubmitError::WouldBlock));
    assert_eq!(target.count(), 0);
    assert_eq!(super::record_admission(CLIENT).unwrap().used(), 0);
    drop((charge1, charge2));
    // Missing targets complete successfully; admission errors above do not.
    let gone = super::observe_thread_exit_with_generation(CLIENT, usize::MAX, None).unwrap();
    assert_eq!(super::poll(CLIENT, gone).unwrap().unwrap().result, OpResult::Ok(0));
    super::close(CLIENT, gone).unwrap();
    drop(target);

    // Cancellation winning before owner installation finishes an external
    // watch, but a worker must keep its terminal subscription until real exit.
    for worker in [false, true] {
        let mut source = ExitSource::new();
        let mut staged = EventSubmission::new(CLIENT).unwrap();
        let callback = observer(&staged);
        let token = source.register(Arc::downgrade(&callback), staged.take_charge()).unwrap();
        super::cancel(CLIENT, staged.cap()).unwrap();
        let cancelled = staged.install_observation(callback, token, worker).unwrap();
        if cancelled {
            super::complete(CLIENT, staged.cap(), OpResult::Cancelled).unwrap();
        }
        let cap = staged.commit();
        if worker {
            assert!(super::poll(CLIENT, cap).unwrap().is_none());
            assert_eq!(source.registered(), 1);
            source.notify_exit();
        } else {
            assert_eq!(source.registered(), 0);
        }
        assert_eq!(super::poll(CLIENT, cap).unwrap().unwrap().result, OpResult::Cancelled);
        super::close(CLIENT, cap).unwrap();
    }

    // Namespace death before a late installation cannot retain an old watch,
    // even if an unrelated kernel owner retains its completion object.
    super::close_address_space(CLIENT);
    let handle = crate::service::loader::create_user_address_space_handle();
    super::open_address_space(handle.id(), 2);
    let source = ExitSource::new();
    let mut staged = EventSubmission::new(handle.id()).unwrap();
    let retained = staged.completion().clone();
    let old_account = super::watch_admission(handle.id()).unwrap();
    let callback = observer(&staged);
    let token = source.register(Arc::downgrade(&callback), staged.take_charge()).unwrap();
    crate::memory::close_user_address_space_handle(handle).unwrap();
    let replacement = crate::service::loader::create_user_address_space_handle();
    assert_eq!(replacement.id(), handle.id());
    assert_ne!(replacement.generation(), handle.generation());
    super::open_address_space(replacement.id(), 2);
    let fresh = super::submit(replacement.id(), super::OpCode::Nop, None).unwrap();
    assert_eq!(fresh, staged.cap());
    assert_eq!(
        staged.install_observation(callback, token, false),
        Err(SubmitError::UnknownAddressSpace)
    );
    assert_eq!(source.registered(), 0);
    assert_eq!(old_account.used(), 0);
    drop((staged, retained));
    assert!(super::poll(replacement.id(), fresh).unwrap().is_none());
    super::complete(replacement.id(), fresh, OpResult::Ok(0)).unwrap();
    super::close(replacement.id(), fresh).unwrap();
    crate::memory::close_user_address_space_handle(replacement).unwrap();

    // Closed source rejection returns its staged charge; discard never invokes
    // callbacks, whereas explicit exit closes/notifies once.
    let account = budget::DomainBudget::new(2);
    let source = ExitSource::new();
    let callback: Arc<dyn Observer> = CallOnNotify::new(|| panic!("discard notified"));
    let runs = WORKER_RUNS.load(Ordering::SeqCst);
    assert_eq!(
        scheduler::spawn_worker_after_observe(
            empty_worker,
            Arc::downgrade(&callback),
            budget::reserve(&account, false).unwrap(),
            |_| Err(SubmitError::UnknownAddressSpace),
        ),
        Err(SubmitError::UnknownAddressSpace)
    );
    assert_eq!(WORKER_RUNS.load(Ordering::SeqCst), runs);
    assert_eq!(account.used(), 0);
    let token = source
        .register(Arc::downgrade(&callback), budget::reserve(&account, false).unwrap())
        .unwrap();
    drop(source);
    assert_eq!(account.used(), 0);
    drop(token);
    // Generic source close semantics are exercised without a Thread allocation.
    let list = crate::klib::observer::registration::ObserverList::try_new(1, false).unwrap();
    drop(list.close());
    assert_eq!(
        list.register(Arc::downgrade(&callback), budget::reserve(&account, false).unwrap())
            .unwrap_err(),
        RegistrationError::Closed
    );
    assert_eq!(account.used(), 0);
    let mut source = ExitSource::new();
    source.notify_exit();
    assert_eq!(budget::node_used(), baseline);
    crate::logln!(
        "[exit watches] SUCCESS: bounded source/shared admission, rollback, cancel churn, \
         generation reuse, reentrant exit and teardown/install fencing"
    );
}

static RELEASE_WORKER: AtomicBool = AtomicBool::new(false);
extern "C" fn held_worker() {
    while !RELEASE_WORKER.load(Ordering::Acquire) {
        scheduler::sleep_millis(1);
    }
}

extern "C" fn held_self_exit_worker() {
    held_worker();
    let tid = scheduler::system_scheduler::get_thread_id().unwrap();
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let lp = crate::cpu::isa::lp::ops::get_lp_id();
    scheduler::system_scheduler::SYSTEM_SCHEDULER
        .read()
        .abort_thread_generation(tid, generation)
        .unwrap();
    // Self-exit must retain the exact outgoing handle/context until the
    // switch has saved it and released the architecture's CPU ownership.
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert_eq!(thread.generation, generation);
        assert!(thread.abort_requested.load(Ordering::Acquire));
        assert_eq!(thread.abort_owner_lp.load(Ordering::Acquire), lp as usize);
        #[cfg(target_arch = "aarch64")]
        assert!(thread.context.is_on_cpu());
    }
    assert_eq!(
        scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .get_lp_scheduler()
            .lock()
            .get_current_handle(),
        Some((tid, generation))
    );
    scheduler::yield_lp();
    panic!("self-exit resumed after its retirement request");
}

pub(crate) fn test_scheduled_cleanup() {
    super::open_address_space(SCHEDULED_CLIENT, 4);
    let account = super::watch_admission(SCHEDULED_CLIENT).unwrap();
    let mut generation = 0;
    RELEASE_WORKER.store(false, Ordering::Release);
    let tid =
        scheduler::spawn_thread_after_publish(KERNEL_ASID, held_self_exit_worker, |_, captured| {
            generation = captured
        });
    for _ in 0..128 {
        let cap =
            super::observe_thread_exit_with_generation(SCHEDULED_CLIENT, tid, Some(generation))
                .unwrap();
        super::cancel(SCHEDULED_CLIENT, cap).unwrap();
        super::close(SCHEDULED_CLIENT, cap).unwrap();
    }
    assert_eq!(MASTER_THREAD_TABLE.read().get(tid).unwrap().exit_watch_count(), 0);
    assert_eq!(account.used(), 0);
    let cap = super::observe_thread_exit_with_generation(SCHEDULED_CLIENT, tid, Some(generation))
        .unwrap();
    RELEASE_WORKER.store(true, Ordering::Release);
    assert!(super::wait_timeout(SCHEDULED_CLIENT, cap, 5_000).unwrap());
    assert_eq!(super::poll(SCHEDULED_CLIENT, cap).unwrap().unwrap().result, OpResult::Ok(0));
    super::close(SCHEDULED_CLIENT, cap).unwrap();
    let runs = WORKER_RUNS.load(Ordering::SeqCst);
    for _ in 0..32 {
        let cap = super::submit_worker(SCHEDULED_CLIENT, empty_worker, OpResult::Ok(9)).unwrap();
        assert!(super::wait_timeout(SCHEDULED_CLIENT, cap, 5_000).unwrap());
        assert_eq!(super::poll(SCHEDULED_CLIENT, cap).unwrap().unwrap().result, OpResult::Ok(9));
        super::close(SCHEDULED_CLIENT, cap).unwrap();
    }
    assert_eq!(WORKER_RUNS.load(Ordering::SeqCst), runs + 32);
    RELEASE_WORKER.store(false, Ordering::Release);
    let cap = super::submit_worker(SCHEDULED_CLIENT, held_worker, OpResult::Ok(11)).unwrap();
    super::cancel(SCHEDULED_CLIENT, cap).unwrap();
    assert!(super::poll(SCHEDULED_CLIENT, cap).unwrap().is_none());
    assert_eq!(account.used(), 1);
    RELEASE_WORKER.store(true, Ordering::Release);
    assert!(super::wait_timeout(SCHEDULED_CLIENT, cap, 5_000).unwrap());
    assert_eq!(super::poll(SCHEDULED_CLIENT, cap).unwrap().unwrap().result, OpResult::Cancelled);
    super::close(SCHEDULED_CLIENT, cap).unwrap();
    assert_eq!(account.used(), 0);
    super::close_address_space(SCHEDULED_CLIENT);
    crate::logln!(
        "[exit watches] SUCCESS: live-target cancel/rearm, self-exit switch ownership, fast \
         worker registration-before-admission and deferred producer cancellation"
    );
}
