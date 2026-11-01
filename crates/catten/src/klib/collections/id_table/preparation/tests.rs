use std::{
    alloc::{
        GlobalAlloc,
        System,
    },
    cell::Cell,
    sync::{
        Arc,
        Barrier,
        Mutex,
        atomic::{
            AtomicUsize,
            Ordering,
        },
    },
};

use super::*;

struct Tracer;
std::thread_local! {
    static TRACE: Cell<bool> = const { Cell::new(false) };
    static ALLOC: Cell<usize> = const { Cell::new(0) };
    static FREE: Cell<usize> = const { Cell::new(0) };
    static GUARDED: Cell<bool> = const { Cell::new(false) };
}
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACE.try_with(Cell::get).unwrap_or(false) {
            ALLOC.with(|n| n.set(n.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACE.try_with(Cell::get).unwrap_or(false) {
            FREE.with(|n| n.set(n.get() + 1));
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Tracer = Tracer;

fn trace<R>(work: impl FnOnce() -> R) -> (R, usize, usize) {
    ALLOC.with(|n| n.set(0));
    FREE.with(|n| n.set(0));
    TRACE.with(|n| n.set(true));
    let result = work();
    TRACE.with(|n| n.set(false));
    (result, ALLOC.with(Cell::get), FREE.with(Cell::get))
}
fn guarded<R>(work: impl FnOnce() -> R) -> R {
    GUARDED.with(|n| n.set(true));
    let (result, alloc, free) = trace(work);
    GUARDED.with(|n| n.set(false));
    assert_eq!((alloc, free), (0, 0), "publication allocated or destroyed backing");
    result
}
#[derive(Default)]
struct Pool {
    used: [u64; 2],
    refunds: usize,
}
struct Charge {
    pool: Arc<Mutex<Pool>>,
    amount: [u64; 2],
    freed_before_refund: Arc<AtomicUsize>,
}
impl Charge {
    fn new<T>(request: &GrowthRequest<T>, pool: &Arc<Mutex<Pool>>) -> Self {
        let amount = request.amount().unwrap();
        for (used, add) in pool.lock().unwrap().used.iter_mut().zip(amount) {
            *used += add;
        }
        Self {
            pool: pool.clone(),
            amount,
            freed_before_refund: Arc::new(AtomicUsize::new(0)),
        }
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        assert!(!GUARDED.with(Cell::get), "refund under publication guards");
        self.freed_before_refund.store(FREE.with(Cell::get), Ordering::Relaxed);
        let mut pool = self.pool.lock().unwrap();
        for (used, sub) in pool.used.iter_mut().zip(self.amount) {
            *used -= sub;
        }
        pool.refunds += 1;
    }
}
type Table = IdTable<u64, Charge>;
fn candidate(table: &Table, pool: &Arc<Mutex<Pool>>) -> PreparedGrowth<u64, Charge> {
    let request = table.growth_request().unwrap().unwrap();
    let charge = Charge::new(&request, pool);
    request.prepare(charge).unwrap()
}
fn initial(pool: &Arc<Mutex<Pool>>) -> Table {
    let mut table = Table::new();
    let mut growth = candidate(&table, pool);
    guarded(|| table.try_insert_prepared(11, Some(&mut growth)).unwrap());
    growth.finish();
    table
}

#[test]
fn growth_moves_complete_slots_without_allocator_or_refund_under_guards() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let mut table = initial(&pool);
    let first_amount = pool.lock().unwrap().used;
    let refund = table.admission.as_ref().unwrap().freed_before_refund.clone();
    let lease = table.lease(0, 1).unwrap();
    for n in 1..4 {
        table.try_insert_prepared(n, None).unwrap();
    }
    let mut growth = candidate(&table, &pool);
    assert_eq!(pool.lock().unwrap().used[1], 12); // old four plus new eight
    guarded(|| table.try_insert_prepared(44, Some(&mut growth)).unwrap());
    assert_eq!(table.generation(0), Ok(1));
    assert_eq!(table.get(0), Ok(&11));
    assert_eq!(table.take_element(0), Err(Error::Leased));
    let (_, alloc, free) = trace(|| growth.finish());
    assert_eq!((alloc, free), (0, 3));
    assert_eq!(refund.load(Ordering::Relaxed), 3);
    assert_eq!(pool.lock().unwrap().used, [first_amount[0] * 2, 8]);
    guarded(|| {
        table.finish_lease(lease).unwrap();
        assert_eq!(table.take_element(0), Ok(11));
    });
    assert_eq!(guarded(|| table.try_insert_prepared(55, None).unwrap()), 0);
    assert_eq!(table.generation(0), Ok(2));
    drop(table);
    assert_eq!(pool.lock().unwrap().used, [0, 0]);
}

#[test]
fn partial_allocation_failure_frees_each_prefix_before_refund() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let table = Table::new();
    for fail in 0..3 {
        let request = table.growth_request().unwrap().unwrap();
        let charge = Charge::new(&request, &pool);
        let observed = charge.freed_before_refund.clone();
        let (result, alloc, free) = trace(|| {
            request.prepare_with(charge, |stage| {
                if stage == fail {
                    Err(Error::AllocationFailed)
                } else {
                    Ok(())
                }
            })
        });
        assert!(matches!(result, Err(Error::AllocationFailed)));
        assert_eq!((alloc, free), (fail, fail));
        assert_eq!(observed.load(Ordering::Relaxed), fail);
        assert_eq!(pool.lock().unwrap().used, [0, 0]);
        assert_eq!(table.iter().len(), 0);
    }
}

#[test]
fn competing_candidate_and_freed_slot_do_not_replace_usable_backing() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let mut table = Table::new();
    let mut winner = candidate(&table, &pool);
    let mut loser = candidate(&table, &pool);
    guarded(|| table.try_insert_prepared(1, Some(&mut winner)).unwrap());
    winner.finish();
    let backing = table.list.as_ptr();
    guarded(|| table.try_insert_prepared(2, Some(&mut loser)).unwrap());
    assert!(!loser.installed);
    assert_eq!(table.list.as_ptr(), backing);
    loser.finish();
    for n in 2..4 {
        table.try_insert_prepared(n, None).unwrap();
    }
    let mut candidate = candidate(&table, &pool);
    guarded(|| table.take_element(0).unwrap());
    assert_eq!(guarded(|| table.try_insert_prepared(99, Some(&mut candidate)).unwrap()), 0);
    assert!(!candidate.installed);
    assert_eq!(table.list.as_ptr(), backing);
    candidate.finish();
    assert_eq!(pool.lock().unwrap().used[1], 4);
}

#[test]
fn foreign_and_undersized_candidates_reject_without_publishing_payload() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let first = Table::new();
    let mut candidate = candidate(&first, &pool);
    let mut second = Table::new();
    assert_eq!(
        guarded(|| second.try_insert_prepared(7, Some(&mut candidate))),
        Err((7, Error::AllocationFailed))
    );
    assert!(second.list.is_empty());
    for n in 0..8 {
        let mut own = self::candidate(&second, &pool);
        guarded(|| second.try_insert_prepared(n, Some(&mut own)).unwrap());
        own.finish();
        while second.has_insertion_capacity() {
            let n = second.list.len() as u64;
            second.try_insert_prepared(n, None).unwrap();
        }
        if second.list.len() >= 8 {
            break;
        }
    }
    // An authentic request can also become too small after a competing grow.
    let mut stale = self::candidate(&second, &pool);
    let mut next = self::candidate(&second, &pool);
    second.try_insert_prepared(8, Some(&mut next)).unwrap();
    next.finish();
    while second.has_insertion_capacity() {
        second.try_insert_prepared(9, None).unwrap();
    }
    let len = second.list.len();
    assert_eq!(
        guarded(|| second.try_insert_prepared(77, Some(&mut stale))),
        Err((77, Error::CapacityChanged))
    );
    assert_eq!(second.list.len(), len);
    stale.finish();
    candidate.finish();
}

#[test]
fn abandoned_candidate_and_replaced_backing_retain_charges_without_cleanup() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let mut table = initial(&pool);
    for n in 1..4 {
        table.try_insert_prepared(n, None).unwrap();
    }
    let unused = candidate(&table, &pool);
    let retained = pool.lock().unwrap().used;
    {
        let _account = pool.lock().unwrap();
        guarded(|| drop(unused));
    }
    assert_eq!(pool.lock().unwrap().used, retained);
    let mut installed = candidate(&table, &pool);
    table.try_insert_prepared(5, Some(&mut installed)).unwrap();
    let retained = pool.lock().unwrap().used;
    {
        let _account = pool.lock().unwrap();
        guarded(|| drop(installed));
    }
    assert_eq!(pool.lock().unwrap().used, retained);
}

#[test]
fn exact_closing_retirement_survives_replacement_and_rejects_stale_generation() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let mut table = initial(&pool);
    let close = table.begin_close(0, 1).unwrap();
    for n in 1..4 {
        table.try_insert_prepared(n, None).unwrap();
    }
    let mut growth = candidate(&table, &pool);
    table.try_insert_prepared(8, Some(&mut growth)).unwrap();
    growth.finish();
    guarded(|| {
        table.seal_close(&close).unwrap();
        table.prepare_closing_retirement(&close).unwrap();
    });
    let retired = table.retire_closing(close).unwrap();
    let (slot, ()) = retired.release_value_with(|v| assert_eq!(*v, 11));
    guarded(|| table.finish_retirement(slot).unwrap());
    assert_eq!(guarded(|| table.try_insert_prepared(88, None).unwrap()), 0);
    assert!(matches!(table.lease(0, 1), Err(Error::WrongLease)));
}

#[test]
fn concurrent_preparations_have_one_install_and_bounded_rejection() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let mut table = initial(&pool);
    for n in 1..4 {
        table.try_insert_prepared(n, None).unwrap();
    }
    let table = Arc::new(Mutex::new(table));
    let barrier = Arc::new(Barrier::new(8));
    let workers: std::vec::Vec<_> = (0..8)
        .map(|n| {
            let (table, pool, barrier) = (table.clone(), pool.clone(), barrier.clone());
            std::thread::spawn(move || {
                let request = { table.lock().unwrap().growth_request().unwrap().unwrap() };
                let charge = Charge::new(&request, &pool);
                let mut growth = request.prepare(charge).unwrap();
                barrier.wait();
                let accepted = {
                    let mut table = table.lock().unwrap();
                    guarded(|| table.try_insert_prepared(n + 100, Some(&mut growth))).is_ok()
                };
                growth.finish();
                accepted
            })
        })
        .collect();
    assert_eq!(workers.into_iter().map(|w| usize::from(w.join().unwrap())).sum::<usize>(), 4);
    assert_eq!(table.lock().unwrap().list.len(), 8);
    assert_eq!(pool.lock().unwrap().used[1], 8);
    drop(table);
    assert_eq!(pool.lock().unwrap().used, [0, 0]);
}

#[test]
fn allocator_tracer_detects_the_previous_guarded_growth_control() {
    let mut table = IdTable::new();
    let (_, alloc, _) = trace(|| table.try_add_element(1).unwrap());
    assert!(alloc >= 3, "unsafe control did not allocate");
}

#[test]
fn empty_fixture_install_uses_the_same_prepared_backing_contract() {
    let pool = Arc::new(Mutex::new(Pool::default()));
    let mut table = Table::new();
    let mut growth = candidate(&table, &pool);
    guarded(|| table.install_empty_growth(&mut growth).unwrap());
    growth.finish();
    assert_eq!(table.iter().len(), 0);
    table.try_insert_prepared(5, None).unwrap();
    let close = table.begin_close(0, 1).unwrap();
    assert_eq!(table.closing_lease_count(&close), Ok(0));
    table.seal_close(&close).unwrap();
    let retired = table.retire_closing(close).unwrap();
    let (slot, ()) = retired.release_value_with(|_| ());
    table.finish_retirement(slot).unwrap();
}

#[test]
fn scalar_rejection_preserves_payload_in_the_same_containing_owner() {
    struct Payload {
        token: std::boxed::Box<u64>,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Payload {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let mut pending = Some(Payload {
        token: std::boxed::Box::new(17),
        drops: drops.clone(),
    });
    let address = pending.as_ref().unwrap() as *const Payload;
    let token = pending.as_ref().unwrap().token.as_ref() as *const u64;
    let mut table: IdTable<Payload> = IdTable::new();
    assert_eq!(guarded(|| table.try_insert_from(&mut pending, None)), Err(Error::CapacityChanged));
    assert_eq!(pending.as_ref().unwrap() as *const Payload, address);
    assert_eq!(pending.as_ref().unwrap().token.as_ref() as *const u64, token);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    let mut growth = table.growth_request().unwrap().unwrap().prepare(()).unwrap();
    let tid = guarded(|| table.try_insert_from(&mut pending, Some(&mut growth)).unwrap());
    assert!(pending.is_none());
    growth.finish();
    assert_eq!(*table.get(tid).unwrap().token, 17);
    drop(table.take_element(tid).unwrap());
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}
