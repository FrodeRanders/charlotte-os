//! Trace live bytes before System deallocation, and admission through that free.
use core::{
    alloc::Layout,
    sync::atomic::{
        AtomicBool,
        AtomicU64,
        AtomicUsize,
        Ordering,
    },
};
use std::alloc::{
    GlobalAlloc,
    System,
};

use super::*;
use crate::klib::admission::{
    self,
    ByteAllocator,
    Reservation,
    SnapshotBuffers,
};

type A = ByteAllocator<SnapshotBuffers>;
static WATCH: AtomicUsize = AtomicUsize::new(0);
static DIRTY: AtomicBool = AtomicBool::new(false);
static FREES: AtomicUsize = AtomicUsize::new(0);
static AT_FREE: AtomicU64 = AtomicU64::new(0);
static REJECT: AtomicBool = AtomicBool::new(false);
struct Tracer;
#[global_allocator]
static GLOBAL: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 257 && REJECT.swap(false, Ordering::AcqRel) {
            return core::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if WATCH.load(Ordering::Acquire) == pointer as usize {
            // This storage is still allocated here. Read the whole requested
            // capacity, including bytes beyond Vec::len(), before freeing it.
            let bytes = unsafe { core::slice::from_raw_parts(pointer, layout.size()) };
            DIRTY.store(bytes.iter().any(|byte| *byte != 0), Ordering::Release);
            AT_FREE.store(used().0[0], Ordering::Release);
            FREES.fetch_add(1, Ordering::AcqRel);
        }
        unsafe { System.dealloc(pointer, layout) };
    }
}
fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<SnapshotBuffers, 2>()
}
fn watch(value: &SensitiveBuffer<A>) {
    DIRTY.store(false, Ordering::Release);
    FREES.store(0, Ordering::Release);
    WATCH.store(value.as_ptr() as usize, Ordering::Release);
}
fn check_free(charged: u64) {
    WATCH.store(0, Ordering::Release);
    assert_eq!(FREES.load(Ordering::Acquire), 1);
    assert!(!DIRTY.load(Ordering::Acquire), "plaintext reached the backend allocator");
    assert_eq!(AT_FREE.load(Ordering::Acquire), charged, "refund preceded backend free");
}

#[test]
fn full_capacity_erased_before_free_with_original_ordinary_or_trusted_charge() {
    for platform in [false, true] {
        for truncated in [false, true] {
            let before = used();
            let mut secret = SensitiveBuffer::new_in(A::new(platform));
            secret.try_initialize_zeroed(257).unwrap();
            assert_eq!(secret.capacity(), 257);
            secret.fill(0xa7);
            if truncated {
                secret.0.truncate(3);
            }
            let charged = used();
            assert_eq!(charged.0, [before.0[0] + 257, before.0[1]]);
            assert_eq!(
                charged.1,
                [
                    before.1[0]
                        + if platform {
                            0
                        } else {
                            257
                        },
                    before.1[1]
                ]
            );
            watch(&secret);
            let moved = secret;
            drop(moved);
            check_free(charged.0[0]);
            assert_eq!(used(), before);
        }
    }
}

#[test]
fn initialized_secrets_cannot_reallocate_or_lose_their_original_owner() {
    let before = used();
    let mut secret = SensitiveBuffer::new_in(A::new(false));
    secret.try_initialize_zeroed(64).unwrap();
    secret.fill(0xb8);
    let pointer = secret.as_ptr();
    let charged = used();
    for len in [0, 8, 64, 128, usize::MAX] {
        assert!(secret.try_initialize_zeroed(len).is_err());
        assert_eq!(secret.as_ptr(), pointer);
        assert_eq!(&*secret, &[0xb8; 64]);
        assert_eq!(used(), charged);
    }
    watch(&secret);
    drop(secret);
    check_free(charged.0[0]);
    assert_eq!(used(), before);
}

#[test]
fn backend_rejection_after_a_live_package_preserves_then_refunds_the_complete_prefix() {
    let before = used();
    let mut package = Vec::new_in(A::new(false));
    package.try_reserve_exact(64).unwrap();
    package.resize(64, 0x5a);
    let prefix = used();
    let mut secret = SensitiveBuffer::new_in(package.allocator().clone());
    REJECT.store(true, Ordering::Release);
    assert!(secret.try_initialize_zeroed(257).is_err());
    assert!(!REJECT.load(Ordering::Acquire), "backend rejection was not reached");
    assert_eq!(secret.capacity(), 0);
    assert_eq!(used(), prefix);
    assert_eq!(&*package, &[0x5a; 64]);
    drop((secret, package));
    assert_eq!(used(), before);
}

#[test]
fn quota_denial_and_empty_storage_do_not_allocate_or_refund_live_bytes() {
    let before = used();
    let limit = crate::resource_policy::POLICY.snapshot_buffers.ordinary[0];
    let pressure =
        Reservation::<SnapshotBuffers, 2>::reserve(false, [limit - before.1[0], 0]).unwrap();
    let charged = used();
    let mut secret = SensitiveBuffer::new_in(A::new(false));
    secret.try_initialize_zeroed(0).unwrap();
    assert!(secret.try_initialize_zeroed(1).is_err());
    assert_eq!(secret.capacity(), 0);
    drop(secret);
    assert_eq!(used(), charged);
    drop(pressure);
    assert_eq!(used(), before);
}
