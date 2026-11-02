//! Actual production owner with traced System deallocation. Physical retirement
//! is a host adapter; native fixtures qualify real memory and exact roots.
use core::sync::atomic::{
    AtomicU64,
    AtomicUsize,
    Ordering,
};
use std::alloc::{
    GlobalAlloc,
    System,
};

use super::*;
use crate::klib::admission;

struct Tracer;
#[global_allocator]
static ALLOCATOR: Tracer = Tracer;
static WATCH: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static AT_FREE: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer as usize == WATCH.load(Ordering::Acquire) {
            AT_FREE.store(used().0[0], Ordering::Release);
            FREES.fetch_add(1, Ordering::AcqRel);
        }
        unsafe { System.dealloc(pointer, layout) }
    }
}
fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<IpcNodes, 2>()
}
fn bytes(n: usize) -> u64 {
    (n * core::mem::size_of::<RetiredMemory>()) as u64
}
fn caps_in(n: usize, platform: bool) -> Buffer<MemoryObjectCap> {
    let mut result = buffer(platform);
    result.try_reserve_exact(n).unwrap();
    result.extend(0..n as u64);
    result
}
fn caps(n: usize) -> Buffer<MemoryObjectCap> {
    caps_in(n, false)
}
fn held(n: usize) -> u64 {
    bytes(n) + (n * core::mem::size_of::<MemoryObjectCap>()) as u64
}
fn watch(retired: &[RetiredMemory]) {
    FREES.store(0, Ordering::Release);
    WATCH.store(retired.as_ptr() as usize, Ordering::Release);
}
fn freed(expected: u64) {
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 1);
    assert_eq!(AT_FREE.load(Ordering::Acquire), expected);
}
#[test]
fn delivered_storage_frees_before_original_byte_refund() {
    let before = used();
    let owner = MemoryAttachments::try_new(caps(3)).unwrap();
    assert_eq!(used().0, [before.0[0] + held(3), before.0[1]]);
    watch(&owner.0.retired);
    owner.finish();
    freed(before.0[0] + bytes(3));
    assert_eq!(used(), before);
}
#[test]
fn allocated_constructor_rejection_frees_before_refund() {
    let before = used();
    assert!(
        MemoryAttachments::try_new_with(caps(3), |retired, n| {
            retired.try_reserve_exact(n).unwrap();
            watch(retired);
            Err(IpcError::ResourceLimit)
        })
        .is_err()
    );
    freed(before.0[0] + bytes(3));
    assert_eq!(used(), before);
}
#[test]
fn empty_bounds_exact_capacity_and_quota_precede_allocator() {
    let before = used();
    let empty =
        MemoryAttachments::try_new_with(buffer(false), |_, _| panic!("empty allocation")).unwrap();
    empty.finish();
    assert!(
        MemoryAttachments::try_new_with(caps(CAP_VECTOR_MAX + 1), |_, _| panic!(
            "bounds reached allocator"
        ))
        .is_err()
    );
    assert!(
        MemoryAttachments::try_new_with(caps(3), |retired, _| {
            retired.try_reserve_exact(2).unwrap();
            Ok(())
        })
        .is_err()
    );
    let limit = crate::resource_policy::POLICY.ipc_nodes.ordinary[0];
    let input = caps(1);
    let pressure = Charge::reserve(false, [limit - used().1[0], 0]).unwrap();
    assert!(
        MemoryAttachments::try_new_with(input, |_, _| panic!("quota reached allocator")).is_err()
    );
    let trusted = MemoryAttachments::try_new(caps_in(1, true)).unwrap();
    assert_eq!(used().1[0], limit - core::mem::size_of::<MemoryObjectCap>() as u64);
    trusted.finish();
    drop(pressure);
    assert_eq!(used(), before);
}
#[test]
fn fifo_callbacks_keep_full_capacity_and_charge_until_iterator_free() {
    let before = used();
    let mut owner = MemoryAttachments::try_new(caps(3)).unwrap();
    for id in 0..3 {
        owner.0.retired.push(RetiredMemory(id));
    }
    watch(&owner.0.retired);
    let mut expected = 0;
    owner.release_with(|retired| {
        assert_eq!(retired.0, expected);
        expected += 1;
        assert_eq!(used().0[0], before.0[0] + held(3));
        assert_eq!(FREES.load(Ordering::Acquire), 0);
        drop(retired);
    });
    assert_eq!(expected, 3);
    freed(before.0[0] + bytes(3));
    assert_eq!(used(), before);
}
#[test]
fn abandonment_is_inert_before_and_under_pool_lock() {
    let before = used();
    for guarded in [false, true] {
        let owner = MemoryAttachments::try_new(caps(1)).unwrap();
        watch(&owner.0.retired);
        if guarded {
            admission::with_locked::<IpcNodes, 2, _>(|| drop(owner));
        } else {
            drop(owner);
        }
        WATCH.store(0, Ordering::Release);
        assert_eq!(FREES.load(Ordering::Acquire), 0);
    }
    assert_eq!(
        used(),
        ([before.0[0] + held(2), before.0[1]], [before.1[0] + held(2), before.1[1]])
    );
}
#[test]
fn interrupted_callback_retains_remaining_owners_and_full_capacity() {
    let before = used();
    let drops = crate::memory::object::DROPS.load(Ordering::Acquire);
    let mut owner = MemoryAttachments::try_new(caps(3)).unwrap();
    for id in 0..3 {
        owner.0.retired.push(RetiredMemory(id));
    }
    watch(&owner.0.retired);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| owner.release_with(|retired| {
            assert_eq!(retired.0, 0);
            drop(retired);
            std::panic::panic_any(());
        })))
        .is_err()
    );
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 0);
    assert_eq!(crate::memory::object::DROPS.load(Ordering::Acquire), drops + 1);
    assert_eq!(
        used(),
        ([before.0[0] + held(3), before.0[1]], [before.1[0] + held(3), before.1[1]])
    );
}
