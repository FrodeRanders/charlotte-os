//! Kernel-only admission and scheduled contention fixtures. Standalone raw
//! cores test observer lifetimes without protecting application data. Scheduled
//! holders never park/yield with a data guard; workers contend on another LP.

use alloc::{
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicU32,
    AtomicUsize,
    Ordering,
};

use lock_api::{
    RawMutex,
    RawRwLock,
};

use super::{
    LOCK_WAIT_ADMISSION_RETRIES,
    mutex,
    rwlock,
};
use crate::{
    cpu::{
        isa::lp::ops::get_lp_id,
        multiprocessor::get_lp_count,
        scheduler::{
            block_until,
            spawn_thread_on_lp,
            system_scheduler::{
                SYSTEM_SCHEDULER,
                get_thread_id,
            },
            threads::{
                MASTER_THREAD_TABLE,
                ThreadState,
            },
            yield_lp,
        },
    },
    klib::observer::{
        CallOnNotify,
        Observable,
        Observer,
        WaitSponsor,
        registration::RegistrationError,
        waiter_budget,
        waiter_source::WaiterSource,
    },
    memory::KERNEL_ASID,
    self_test::results::Deadline,
};

pub(crate) fn test_admission() {
    let baseline = waiter_budget::node_used();
    let sponsor = WaitSponsor::new(false);
    let mutex = Arc::new(mutex::MutexCore::new());
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let target = mutex.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        // Both list and lazy-initialization guards must be released first.
        assert_eq!(target.waiter_count(), 0);
        assert!(target.try_lock());
        unsafe { target.unlock() };
        count.fetch_add(1, Ordering::Relaxed);
    });
    assert!(mutex.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap().is_ready());
    assert_eq!(sponsor.used(), 0);
    mutex.lock();
    let mut registrations = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        registrations.push(mutex.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert_eq!(sponsor.used(), 64);
    assert!(matches!(
        mutex.try_register_waiter(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::ResourceLimit)
    ));
    assert_eq!(sponsor.used(), 64);
    drop(registrations.pop());
    assert_eq!(mutex.waiter_count(), 63);
    registrations.push(mutex.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    unsafe { mutex.unlock() };
    assert_eq!(hits.load(Ordering::Relaxed), 64);
    assert_eq!(sponsor.used(), 0);
    drop(registrations);
    // Re-arm/cancel repeatedly; the source must not accumulate dead weak refs.
    mutex.lock();
    for _ in 0..512 {
        drop(mutex.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
        assert_eq!(mutex.waiter_count(), 0);
    }
    unsafe { mutex.unlock() };

    let rw = Arc::new(rwlock::RwLockCore::new());
    let count = hits.clone();
    let target = rw.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        assert_eq!(target.waiter_source(true).registered(), 0);
        assert_eq!(target.waiter_source(false).registered(), 0);
        assert!(target.try_lock_shared());
        unsafe { target.unlock_shared() };
        count.fetch_add(1, Ordering::Relaxed);
    });
    rw.lock_exclusive();
    let mut registrations = Vec::new();
    for shared in [true, false] {
        let source = rw.waiter_source(shared);
        for _ in 0..waiter_budget::SOURCE_LIMIT {
            registrations
                .push(source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
        }
        assert!(matches!(
            source.try_register_waiter(Arc::downgrade(&observer), &sponsor),
            Err(RegistrationError::ResourceLimit)
        ));
    }
    assert_eq!(sponsor.used(), 128);
    unsafe { rw.unlock_exclusive() };
    assert_eq!(hits.load(Ordering::Relaxed), 192);
    assert_eq!(sponsor.used(), 0);
    drop(registrations);

    // An expired exclusive candidate must not suppress reader notification.
    rw.lock_exclusive();
    let expired: Arc<dyn Observer> = CallOnNotify::new(|| panic!("expired observer fired"));
    let stale =
        rw.waiter_source(false).try_register_waiter(Arc::downgrade(&expired), &sponsor).unwrap();
    drop(expired);
    let reader =
        rw.waiter_source(true).try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    unsafe { rw.unlock_exclusive() };
    assert_eq!(hits.load(Ordering::Relaxed), 193);
    assert_eq!(sponsor.used(), 0);
    drop((stale, reader));

    // Only the final reader releases ownership; earlier read unlocks must not
    // drain the exclusive candidate list and cause pointless re-parking.
    rw.lock_shared();
    rw.lock_shared();
    let writer =
        rw.waiter_source(false).try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap();
    unsafe { rw.unlock_shared() };
    assert_eq!(rw.waiter_source(false).registered(), 1);
    assert_eq!(hits.load(Ordering::Relaxed), 193);
    unsafe { rw.unlock_shared() };
    assert_eq!(hits.load(Ordering::Relaxed), 194);
    drop(writer);

    let source = WaiterSource::new();
    let mut old = Vec::new();
    for _ in 0..64 {
        old.push(source.register(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    let batch = source.drain();
    let mut new = Vec::new();
    for _ in 0..64 {
        new.push(source.register(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert_eq!(sponsor.used(), 128);
    drop(old); // Detached storage still holds its charges.
    assert_eq!(sponsor.used(), 128);
    drop(batch);
    assert_eq!(sponsor.used(), 64);
    drop(source); // Source destruction releases entries despite retained tokens.
    assert_eq!(sponsor.used(), 0);
    drop(new);
    let source = WaiterSource::new();
    sponsor.retire();
    assert!(matches!(
        source.register(Arc::downgrade(&observer), &sponsor),
        Err(RegistrationError::Closed)
    ));
    assert_eq!(source.registered(), 0);
    drop((observer, rw, mutex));
    assert_eq!(waiter_budget::node_used(), baseline);
    crate::logln!(
        "[lock waiters] SUCCESS: bounds/rollback, ready fast path, cancel/rearm, broadcast \
         reentrancy, expired writer, final reader and source destruction"
    );
}

static MUTEX: mutex::Mutex<u64> = mutex::Mutex::new(0);
static RW: rwlock::RwLock<u64> = rwlock::RwLock::new(0);
static MUTEX_DONE: AtomicU32 = AtomicU32::new(0);
static READER_DONE: AtomicU32 = AtomicU32::new(0);
static WRITER_DONE: AtomicU32 = AtomicU32::new(0);
static FINAL_WRITER_DONE: AtomicU32 = AtomicU32::new(0);

extern "C" fn mutex_worker() {
    {
        let mut value = MUTEX.lock();
        assert_eq!(*value, 41);
        *value += 1;
    }
    MUTEX_DONE.store(1, Ordering::Release);
}
extern "C" fn reader_worker() {
    {
        let value = RW.read();
        assert!((41..=42).contains(&*value));
    }
    READER_DONE.store(1, Ordering::Release);
}
extern "C" fn writer_worker() {
    {
        let mut value = RW.write();
        assert_eq!(*value, 41);
        *value += 1;
    }
    WRITER_DONE.store(1, Ordering::Release);
}
extern "C" fn final_writer_worker() {
    {
        let mut value = RW.write();
        assert_eq!(*value, 42);
        *value += 1;
    }
    FINAL_WRITER_DONE.store(1, Ordering::Release);
}

fn poll_peer(condition: impl Fn() -> bool) {
    let deadline = Deadline::after_millis(5_000);
    while !condition() {
        deadline.assert_pending("remote blocking-lock waiter");
        core::hint::spin_loop(); // Holder never parks with its data guard.
    }
}
fn await_done(flag: &AtomicU32) {
    let deadline = Deadline::after_millis(5_000);
    while flag.load(Ordering::Acquire) == 0 {
        deadline.assert_pending("blocking-lock worker completion");
        yield_lp();
    }
}

pub(crate) fn test_scheduled_cleanup() {
    let tid = get_thread_id().unwrap();
    let constraints = MASTER_THREAD_TABLE.read().get(tid).unwrap().migration_constraints;
    let source = WaiterSource::new();
    for _ in 0..64 {
        assert!(!block_until(&source, 1, || false));
        assert_eq!(source.registered(), 0);
    }
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| {});
    let mut full = Vec::new();
    for _ in 0..64 {
        full.push(source.register(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    assert!(matches!(
        SYSTEM_SCHEDULER.read().block_thread(tid, &source),
        Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed)
    ));
    assert!(!block_until(&source, 1, || false));
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).unwrap();
        assert!(matches!(thread.state, ThreadState::Running(_)));
        assert_eq!(thread.migration_constraints, constraints);
    }
    drop(full);

    let lp = (get_lp_id() + 1) % get_lp_count();
    assert_ne!(lp, get_lp_id(), "blocking-lock fixture needs another LP");
    let mut guard = MUTEX.lock();
    *guard = 41;
    // SAFETY: borrow lock_api's raw core only to inspect/register test waiters;
    // the guard remains the sole data owner, and no raw unlock is performed.
    let raw = unsafe { MUTEX.raw() };
    let mut full = Vec::new();
    for _ in 0..64 {
        full.push(raw.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    let retries = LOCK_WAIT_ADMISSION_RETRIES[0].load(Ordering::Relaxed);
    spawn_thread_on_lp(KERNEL_ASID, mutex_worker, lp);
    poll_peer(|| LOCK_WAIT_ADMISSION_RETRIES[0].load(Ordering::Relaxed) > retries);
    drop(full);
    poll_peer(|| raw.waiter_count() == 1);
    drop(guard);
    await_done(&MUTEX_DONE);
    assert_eq!(raw.waiter_count(), 0);
    assert_eq!(*MUTEX.lock(), 42);

    let mut guard = RW.write();
    *guard = 41;
    // SAFETY: diagnostics borrow the core, never alter data-guard ownership.
    let raw = unsafe { RW.raw() };
    let mut full = Vec::new();
    for shared in [true, false] {
        for _ in 0..64 {
            full.push(
                raw.waiter_source(shared).register(Arc::downgrade(&observer), &sponsor).unwrap(),
            );
        }
    }
    let readers = LOCK_WAIT_ADMISSION_RETRIES[1].load(Ordering::Relaxed);
    let writers = LOCK_WAIT_ADMISSION_RETRIES[2].load(Ordering::Relaxed);
    spawn_thread_on_lp(KERNEL_ASID, reader_worker, lp);
    spawn_thread_on_lp(KERNEL_ASID, writer_worker, lp);
    poll_peer(|| {
        LOCK_WAIT_ADMISSION_RETRIES[1].load(Ordering::Relaxed) > readers
            && LOCK_WAIT_ADMISSION_RETRIES[2].load(Ordering::Relaxed) > writers
    });
    drop(full);
    poll_peer(|| {
        raw.waiter_source(true).registered() == 1 && raw.waiter_source(false).registered() == 1
    });
    drop(guard);
    await_done(&READER_DONE);
    await_done(&WRITER_DONE);
    assert_eq!(raw.waiter_source(true).registered(), 0);
    assert_eq!(raw.waiter_source(false).registered(), 0);

    let first = RW.read();
    let second = RW.read();
    spawn_thread_on_lp(KERNEL_ASID, final_writer_worker, lp);
    poll_peer(|| raw.waiter_source(false).registered() == 1);
    drop(first);
    assert_eq!(raw.waiter_source(false).registered(), 1);
    assert_eq!(FINAL_WRITER_DONE.load(Ordering::Acquire), 0);
    drop(second);
    await_done(&FINAL_WRITER_DONE);
    assert_eq!(*RW.read(), 43);
    assert_eq!(raw.waiter_source(false).registered(), 0);
    assert_eq!(sponsor.used(), 0);
    crate::logln!(
        "[lock waiters] SUCCESS: timed cleanup, non-mutating rejection, forced \
         mutex/reader/writer retries, remote contention and final-reader wake"
    );
}
