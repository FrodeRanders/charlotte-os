use core::sync::atomic::AtomicUsize;
use std::{
    sync::Barrier,
    thread,
};

use super::*;

struct LayoutState {
    bytes: AtomicUsize,
    align: AtomicUsize,
    limit: usize,
    rollbacks: AtomicUsize,
    refunds: AtomicUsize,
}
struct LayoutCharge(Arc<LayoutState>);
impl Drop for LayoutCharge {
    fn drop(&mut self) {
        self.0.bytes.store(0, Ordering::Release);
        self.0.refunds.fetch_add(1, Ordering::Relaxed);
    }
}
struct Measured;
impl PayloadAdmission<LayoutCharge> for Measured {
    fn reserve(charge: &LayoutCharge, layout: Layout) -> Result<(), AllocError> {
        if layout.size() > charge.0.limit {
            return Err(AllocError);
        }
        assert_eq!(charge.0.bytes.swap(layout.size(), Ordering::AcqRel), 0);
        charge.0.align.store(layout.align(), Ordering::Release);
        Ok(())
    }

    fn rollback(charge: &LayoutCharge, layout: Layout) {
        assert_eq!(charge.0.bytes.swap(0, Ordering::AcqRel), layout.size());
        charge.0.rollbacks.fetch_add(1, Ordering::Relaxed);
    }
}
fn measured(limit: usize) -> (Arc<LayoutState>, ChargedAllocator<LayoutCharge, Measured>) {
    let state = Arc::new(LayoutState {
        bytes: AtomicUsize::new(0),
        align: AtomicUsize::new(0),
        limit,
        rollbacks: AtomicUsize::new(0),
        refunds: AtomicUsize::new(0),
    });
    let allocator = ChargedAllocator::try_new_with_policy(LayoutCharge(state.clone())).unwrap();
    (state, allocator)
}
#[test]
fn actual_arc_layout_includes_alignment_and_survives_final_weak() {
    #[repr(align(256))]
    struct Aligned([u8; 257]);
    let (state, allocator) = measured(4096);
    let value = allocator.try_arc(Aligned([7; 257])).unwrap();
    let bytes = state.bytes.load(Ordering::Acquire);
    assert!(bytes > core::mem::size_of::<Aligned>());
    assert_eq!(state.align.load(Ordering::Acquire), 256);
    assert_eq!(value.0[256], 7);
    let weak = Arc::downgrade(&value);
    drop(value);
    assert!(weak.upgrade().is_none());
    assert_eq!(state.bytes.load(Ordering::Acquire), bytes);
    assert_eq!(state.refunds.load(Ordering::Acquire), 0);
    drop(weak);
    assert_eq!(state.bytes.load(Ordering::Acquire), 0);
    assert_eq!(state.refunds.load(Ordering::Acquire), 1);
}
#[test]
fn layout_denial_reaches_no_allocator_and_retains_private_owner() {
    let (state, allocator) = measured(0);
    without_heap(|| {
        assert!(
            allocator
                .allocate_payload(Layout::new::<u64>(), |_| panic!("denial reached allocator"))
                .is_err()
        )
    });
    assert_eq!(state.bytes.load(Ordering::Acquire), 0);
    assert_eq!(state.refunds.load(Ordering::Acquire), 0);
    drop(allocator);
    assert_eq!(state.refunds.load(Ordering::Acquire), 1);
}
#[test]
fn allocation_failure_rolls_back_layout_before_retry_and_free_keeps_owner_charge() {
    let (state, allocator) = measured(4096);
    let layout = Layout::from_size_align(512, 64).unwrap();
    assert!(allocator.allocate_payload(layout, |_| Err(AllocError)).is_err());
    assert_eq!(state.bytes.load(Ordering::Acquire), 0);
    assert_eq!(state.rollbacks.load(Ordering::Acquire), 1);
    let backing = allocator.allocate(layout).unwrap();
    assert_eq!(state.bytes.load(Ordering::Acquire), 512);
    let other = allocator.clone();
    without_heap(|| {
        assert!(other.allocate_payload(layout, |_| panic!("clone multiplied allowance")).is_err())
    });
    unsafe {
        allocator.deallocate(backing.cast(), layout);
    }
    assert_eq!(state.bytes.load(Ordering::Acquire), 512);
    drop(allocator);
    assert_eq!(state.refunds.load(Ordering::Acquire), 0);
    drop(other);
    assert_eq!(state.bytes.load(Ordering::Acquire), 0);
    assert_eq!(state.refunds.load(Ordering::Acquire), 1);
}

std::thread_local! {
    static TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static HEAP_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
struct TracedAllocator;
#[global_allocator]
static ALLOCATOR: TracedAllocator = TracedAllocator;
fn note_heap_call() {
    if TRACE.try_with(std::cell::Cell::get).unwrap_or(false) {
        let _ = HEAP_CALLS.try_with(|count| count.set(count.get() + 1));
    }
}
// SAFETY: Forward identical pointer/layout operations to System; only count
// calls on the current test thread, without allocating in the tracer.
unsafe impl std::alloc::GlobalAlloc for TracedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_heap_call();
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_heap_call();
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
fn without_heap<R>(f: impl FnOnce() -> R) -> R {
    struct Trace;
    impl Drop for Trace {
        fn drop(&mut self) {
            TRACE.with(|trace| trace.set(false));
        }
    }
    HEAP_CALLS.with(|count| count.set(0));
    TRACE.with(|trace| trace.set(true));
    let trace = Trace;
    let result = f();
    drop(trace);
    assert_eq!(HEAP_CALLS.with(std::cell::Cell::get), 0);
    result
}

struct Charge(Arc<AtomicUsize>);
impl Charge {
    fn new(used: &Arc<AtomicUsize>) -> Self {
        used.fetch_add(1, Ordering::SeqCst);
        Self(used.clone())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        assert_eq!(self.0.fetch_sub(1, Ordering::SeqCst), 1);
    }
}
struct Payload {
    used: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl Drop for Payload {
    fn drop(&mut self) {
        assert_eq!(self.used.load(Ordering::SeqCst), 1, "payload must drop before refund");
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn strong_and_weak_storage_return_one_original_charge() {
    let used = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let value = allocator
        .try_arc(Payload {
            used: used.clone(),
            dropped: dropped.clone(),
        })
        .unwrap();
    let weak = Arc::downgrade(&value);
    let aliases: Vec<_> = (0..128).map(|_| weak.clone()).collect();
    let upgraded = weak.upgrade().unwrap();
    drop(value);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(upgraded);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_none());
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(aliases);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(weak);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn allocation_owner_failure_returns_unused_charge() {
    let used = Arc::new(AtomicUsize::new(0));
    assert!(ChargedAllocator::try_new_with(Charge::new(&used), || Err(AllocError)).is_err());
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn cloned_allocators_cannot_multiply_or_revive_allocations() {
    let used = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let value = Arc::try_new_in(17u64, allocator.clone()).unwrap();
    assert!(Arc::try_new_in(18u64, allocator.clone()).is_err());
    assert_eq!(*value, 17);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(value);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    assert!(Arc::try_new_in(19u64, allocator.clone()).is_err());
    drop(allocator);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

struct RejectAllocation<A>(A);
// SAFETY: Refuses every allocation; any forwarded deallocation belongs to A.
unsafe impl<A: Allocator> Allocator for RejectAllocation<A> {
    fn allocate(&self, _: Layout) -> Result<NonNull<[u8]>, AllocError> {
        Err(AllocError)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { self.0.deallocate(ptr, layout) };
    }
}

#[test]
fn payload_allocation_failure_drops_payload_before_charge() {
    let used = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let rejected = allocator.try_arc_with(
        Payload {
            used: used.clone(),
            dropped: dropped.clone(),
        },
        |allocator| match Arc::<Payload, _>::try_new_uninit_in(RejectAllocation(allocator)) {
            Err(error) => Err(error),
            Ok(_) => panic!("injected allocation unexpectedly succeeded"),
        },
    );
    assert!(rejected.is_err());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn concurrent_last_weak_destruction_refunds_once() {
    for _ in 0..64 {
        let used = Arc::new(AtomicUsize::new(0));
        let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
        let value = Arc::try_new_in(23u64, allocator).unwrap();
        let weak = Arc::downgrade(&value);
        let barrier = Arc::new(Barrier::new(4));
        let joins: Vec<_> = (0..4)
            .map(|_| {
                let weak = weak.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    drop(weak);
                })
            })
            .collect();
        drop(value);
        assert_eq!(used.load(Ordering::SeqCst), 1);
        drop(weak);
        for join in joins {
            join.join().unwrap();
        }
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn concurrent_allocator_clones_admit_only_one_backing() {
    let used = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let barrier = Arc::new(Barrier::new(4));
    let joins: Vec<_> = (0..4)
        .map(|number| {
            let allocator = allocator.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                allocator.try_arc(number)
            })
        })
        .collect();
    let outcomes: Vec<_> = joins.into_iter().map(|join| join.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(outcomes);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(allocator);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn holder_rejection_returns_before_original_charge_disposal() {
    use std::sync::Mutex;
    struct GuardedCharge {
        used: Arc<AtomicUsize>,
        returned: Arc<AtomicBool>,
        guard: Arc<Mutex<()>>,
    }
    impl Drop for GuardedCharge {
        fn drop(&mut self) {
            assert!(self.returned.load(Ordering::SeqCst));
            assert!(self.guard.try_lock().is_ok());
            assert_eq!(self.used.fetch_sub(1, Ordering::SeqCst), 1);
        }
    }
    let used = Arc::new(AtomicUsize::new(1));
    let returned = Arc::new(AtomicBool::new(false));
    let guard = Arc::new(Mutex::new(()));
    let charge = GuardedCharge {
        used: used.clone(),
        returned: returned.clone(),
        guard: guard.clone(),
    };
    assert!(
        ChargedAllocator::try_new_with(charge, || {
            let _guard = guard.lock().unwrap();
            assert_eq!(used.load(Ordering::SeqCst), 1);
            returned.store(true, Ordering::SeqCst);
            Err(AllocError)
        })
        .is_err()
    );
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
#[allow(clippy::drop_non_drop)]
fn abandoned_holder_phases_retain_original_charge() {
    for allocated in [false, true] {
        let used = Arc::new(AtomicUsize::new(0));
        let guard = std::sync::Mutex::new(());
        let mut preparing = PreparingAllocation::<_>::new(Charge::new(&used));
        if allocated {
            preparing.allocate(Arc::try_new_uninit).unwrap();
        }
        let _guard = guard.lock().unwrap();
        drop(preparing);
        assert_eq!(used.load(Ordering::SeqCst), 1);
        // Deliberate host-only negative retention until process shutdown.
    }
}

#[test]
fn empty_holder_initialization_preserves_original_charge_and_allowance() {
    let used = Arc::new(AtomicUsize::new(0));
    let guard = std::sync::Mutex::new(());
    let mut preparing = PreparingAllocation::<_>::new(Charge::new(&used));
    preparing.allocate(Arc::try_new_uninit).unwrap();
    let allocator = {
        let _guard = guard.lock().unwrap();
        preparing.complete()
    };
    assert_eq!(used.load(Ordering::SeqCst), 1);
    let value = allocator.clone().try_arc(37u64).unwrap();
    assert!(allocator.clone().try_arc(38u64).is_err());
    let weak = Arc::downgrade(&value);
    drop((value, allocator));
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(weak);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn unused_empty_holder_storage_cancels_explicitly() {
    let used = Arc::new(AtomicUsize::new(0));
    let mut preparing = PreparingAllocation::<_>::new(Charge::new(&used));
    preparing.allocate(Arc::try_new_uninit).unwrap();
    let mut checks = 0;
    preparing.cancel_with(|| {
        assert_eq!(used.load(Ordering::SeqCst), 1);
        checks += 1;
    });
    assert_eq!(checks, 2);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn borrowed_holder_rejection_retains_parent_reservation() {
    let used = Arc::new(AtomicUsize::new(0));
    let mut charge = Some(Charge::new(&used));
    assert!(ChargedAllocator::try_new_retaining_with(&mut charge, || Err(AllocError)).is_err());
    assert!(charge.is_some());
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(charge.take());
    assert_eq!(used.load(Ordering::SeqCst), 0);
    charge = Some(Charge::new(&used));
    let allocator = ChargedAllocator::try_new_retaining(&mut charge).unwrap();
    assert!(charge.is_none());
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(allocator);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn payload_rejection_returns_before_resource_disposal() {
    struct GuardedPayload {
        payload: Payload,
        returned: Arc<AtomicBool>,
        guard: Arc<std::sync::Mutex<()>>,
    }
    impl Drop for GuardedPayload {
        fn drop(&mut self) {
            assert!(self.returned.load(Ordering::SeqCst));
            assert!(self.guard.try_lock().is_ok());
            assert_eq!(self.payload.used.load(Ordering::SeqCst), 1);
        }
    }
    let used = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let returned = Arc::new(AtomicBool::new(false));
    let guard = Arc::new(std::sync::Mutex::new(()));
    let value = GuardedPayload {
        payload: Payload {
            used: used.clone(),
            dropped: dropped.clone(),
        },
        returned: returned.clone(),
        guard: guard.clone(),
    };
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    assert!(
        allocator
            .try_arc_with(value, |allocator| {
                let _guard = guard.lock().unwrap();
                drop(allocator);
                assert_eq!(dropped.load(Ordering::SeqCst), 0);
                assert_eq!(used.load(Ordering::SeqCst), 1);
                returned.store(true, Ordering::SeqCst);
                Err(AllocError)
            })
            .is_err()
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
#[allow(clippy::drop_non_drop)]
fn abandoned_payload_phases_retain_complete_owner() {
    for allocated in [false, true] {
        let used = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
        let mut preparing = PreparingArc::new(
            Payload {
                used: used.clone(),
                dropped: dropped.clone(),
            },
            allocator,
        );
        if allocated {
            preparing.allocate(Arc::<Payload, _>::try_new_uninit_in).unwrap();
        }
        let guard = std::sync::Mutex::new(());
        let _guard = guard.lock().unwrap();
        without_heap(|| drop(preparing));
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        assert_eq!(used.load(Ordering::SeqCst), 1);
        // Host-only negative retention until process shutdown.
    }
}

#[test]
fn unused_payload_storage_disposes_payload_before_original_admission() {
    let used = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let mut preparing = PreparingArc::new(
        Payload {
            used: used.clone(),
            dropped: dropped.clone(),
        },
        allocator,
    );
    preparing.allocate(Arc::<Payload, _>::try_new_uninit_in).unwrap();
    let mut checks = 0;
    preparing.cancel_with(|| {
        assert_eq!(used.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), usize::from(checks > 0));
        checks += 1;
    });
    assert_eq!(checks, 3);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn payload_initialization_needs_no_allocator_or_payload_disposal() {
    let used = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let allocator = ChargedAllocator::try_new(Charge::new(&used)).unwrap();
    let mut preparing = PreparingArc::new(
        Payload {
            used: used.clone(),
            dropped: dropped.clone(),
        },
        allocator,
    );
    preparing.allocate(Arc::<Payload, _>::try_new_uninit_in).unwrap();
    let guard = std::sync::Mutex::new(());
    let value = {
        let _guard = guard.lock().unwrap();
        without_heap(|| preparing.complete())
    };
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    let weak = Arc::downgrade(&value);
    drop(value);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(used.load(Ordering::SeqCst), 1);
    drop(weak);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[derive(Default)]
struct BackingState {
    used: AtomicUsize,
    limit: AtomicUsize,
    allocations: AtomicUsize,
    frees: AtomicUsize,
    held_at_free: AtomicUsize,
}
#[derive(Clone)]
struct Backing(Arc<BackingState>);
// SAFETY: This host adapter forwards unchanged Global pointers/layouts. It
// observes the actual Arc layouts supplied by the production owning adapter.
unsafe impl Allocator for Backing {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(layout.size())
                    .filter(|bytes| *bytes <= self.0.limit.load(Ordering::Acquire))
            })
            .map_err(|_| AllocError)?;
        match Global.allocate(layout) {
            Ok(block) => {
                self.0.allocations.fetch_add(1, Ordering::AcqRel);
                Ok(block)
            }
            Err(error) => {
                self.0.used.fetch_sub(layout.size(), Ordering::AcqRel);
                Err(error)
            }
        }
    }

    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        assert!(self.0.used.load(Ordering::Acquire) >= layout.size());
        unsafe { Global.deallocate(pointer, layout) };
        self.0
            .held_at_free
            .store(self.0.used.fetch_sub(layout.size(), Ordering::AcqRel), Ordering::Release);
        self.0.frees.fetch_add(1, Ordering::AcqRel);
    }
}
fn backing(limit: usize) -> (Arc<BackingState>, Backing) {
    let state = Arc::new(BackingState::default());
    state.limit.store(limit, Ordering::Release);
    (state.clone(), Backing(state))
}
#[test]
fn captured_backing_funds_actual_holder_payload_and_final_weak_release() {
    #[repr(align(256))]
    struct Aligned([u8; 257]);
    let used = Arc::new(AtomicUsize::new(0));
    let (state, backing) = backing(4096);
    let allocator: ChargedAllocator<_, PreAdmitted, _> =
        ChargedAllocator::try_new_with_backing(Charge::new(&used), backing).unwrap();
    let holder = state.used.load(Ordering::Acquire);
    assert!(holder > 0);
    let retained = allocator.clone();
    assert_eq!(state.allocations.load(Ordering::Acquire), 1);
    let value = allocator.try_arc(Aligned([7; 257])).unwrap();
    assert_eq!(value.0[256], 7);
    let total = state.used.load(Ordering::Acquire);
    assert!(total > holder + core::mem::size_of::<Aligned>());
    let weak = Arc::downgrade(&value);
    drop(value);
    assert_eq!(state.used.load(Ordering::Acquire), total);
    assert_eq!(used.load(Ordering::Acquire), 1);
    drop(weak);
    assert_eq!(state.used.load(Ordering::Acquire), holder);
    assert_eq!(state.frees.load(Ordering::Acquire), 1);
    assert_eq!(used.load(Ordering::Acquire), 1);
    assert!(retained.clone().try_arc(0u8).is_err());
    drop(retained);
    assert_eq!(state.used.load(Ordering::Acquire), 0);
    assert_eq!(state.frees.load(Ordering::Acquire), 2);
    assert_eq!(state.held_at_free.load(Ordering::Acquire), holder);
    assert_eq!(used.load(Ordering::Acquire), 0);
}
#[test]
fn captured_backing_rejects_holder_and_payload_without_losing_original_charge() {
    let used = Arc::new(AtomicUsize::new(0));
    let (state, adapter) = backing(0);
    let rejected: Result<ChargedAllocator<_, PreAdmitted, _>, _> =
        ChargedAllocator::try_new_with_backing(Charge::new(&used), adapter);
    assert!(rejected.is_err());
    assert_eq!(used.load(Ordering::Acquire), 0);
    assert_eq!(state.allocations.load(Ordering::Acquire), 0);
    let dropped = Arc::new(AtomicUsize::new(0));
    let (state, adapter) = backing(4096);
    let allocator: ChargedAllocator<_, PreAdmitted, _> =
        ChargedAllocator::try_new_with_backing(Charge::new(&used), adapter).unwrap();
    state.limit.store(state.used.load(Ordering::Acquire), Ordering::Release);
    assert!(
        allocator
            .try_arc(Payload {
                used: used.clone(),
                dropped: dropped.clone()
            })
            .is_err()
    );
    assert_eq!(state.used.load(Ordering::Acquire), 0);
    assert_eq!(state.frees.load(Ordering::Acquire), 1);
    assert_eq!(used.load(Ordering::Acquire), 0);
    assert_eq!(dropped.load(Ordering::Acquire), 1);
}
