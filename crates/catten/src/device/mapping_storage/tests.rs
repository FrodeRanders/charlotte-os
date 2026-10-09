//! Serialized metadata adapters; no device sees their records.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::*;

static REJECT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PREPARED: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicUsize = AtomicUsize::new(0);
pub(in crate::device) fn reject_next(stage: usize) {
    REJECT.store(stage, Ordering::Release);
}
pub(super) fn reject(stage: usize) -> bool {
    REJECT.compare_exchange(stage, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
}
pub(super) fn boundary(dispose: bool) {
    super::super::backend_registry::tests::boundary(dispose);
    if ACTIVE.load(Ordering::Acquire) {
        if dispose {
            &DISPOSED
        } else {
            &PREPARED
        }
        .fetch_add(1, Ordering::Relaxed);
    }
}
pub(in crate::device) fn begin_real() {
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    PREPARED.store(0, Ordering::Relaxed);
    DISPOSED.store(0, Ordering::Relaxed);
}
pub(in crate::device) fn finish_real() {
    ACTIVE.store(false, Ordering::Release);
    crate::logln!(
        "[DMA metadata phases] {} preparation and {} disposal entries outside local guards; entry \
         IRQ state preserved",
        PREPARED.load(Ordering::Acquire),
        DISPOSED.load(Ordering::Acquire)
    );
}
pub(in crate::device) fn run() {
    super::super::backend_registry::tests::begin_real();
    begin_real();
    let root = crate::service::loader::create_user_address_space_handle();
    let cap = object::allocate(root.id(), 1).unwrap();
    let pin = || object::pin_for_dma(root.id(), cap, true, true, false).unwrap();
    let mut records = Records::new();
    let mut rejected = PendingPin::new(Some(pin()));
    reject_next(1);
    assert_eq!(records.prepare(&mut rejected), Err(Error::Memory));
    rejected.release();
    let mut first = PendingPin::new(Some(pin()));
    records.prepare(&mut first).unwrap();
    {
        let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        records.publish(&mut first, 4096, 1);
        drop(heap);
    }
    first.release();
    let mut detached = PendingPin::new(None);
    let before;
    {
        let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let node = records.take(4096).unwrap();
        before = node.value() as *const (u64, Record);
        detached.retain_record(node);
        records.quarantine(&mut detached);
        drop(heap);
    }
    detached.release();
    let mut duplicate = PendingPin::new(Some(pin()));
    assert_eq!(records.prepare(&mut duplicate), Err(Error::AlreadyMapped));
    duplicate.release();
    assert_eq!(
        object::try_close_cap(root.id(), cap),
        Err(object::MemoryObjectError::LendingActive)
    );
    let node = records.quarantined.pop().unwrap();
    assert_eq!(node.value() as *const (u64, Record), before);
    release_record(node);

    // Failed prefix keeps its original pre-leaf allocation too.
    let mut prefix = PendingPin::new(Some(pin()));
    records.prepare(&mut prefix).unwrap();
    {
        let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        records.quarantine(&mut prefix);
        drop(heap);
    }
    prefix.release();
    records.release();
    #[cfg(target_arch = "aarch64")]
    {
        let mut cache = WalkerCache::new();
        reject_next(2);
        assert_eq!(cache.prepare(), Err(Error::Memory));
        cache.prepare().unwrap();
        {
            let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            // Metadata ABI fixture, no table backing adopted or hardware link.
            cache.publish(3, crate::memory::physical::PAddr::from(4096u64));
            assert_eq!(cache.get(&3), Some(&crate::memory::physical::PAddr::from(4096u64)));
            drop(heap);
        }
        cache.prepare().unwrap();
        cache.prepare().unwrap(); // Reuse one unused node after a partial walk.
        cache.release();
    }
    object::try_close_cap(root.id(), cap).unwrap();
    crate::memory::close_user_address_space_handle(root).unwrap();
    finish_real();
    super::super::backend_registry::tests::finish_real();
    crate::logln!(
        "[DMA metadata ownership] rejected admission releases pin; heap-held \
         publish/detach/quarantine keeps exact node; duplicate and early close reject; confirmed \
         private completion releases original storage"
    );
}
