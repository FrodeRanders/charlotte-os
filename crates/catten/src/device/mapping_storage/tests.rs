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
    let backing_before = budget::snapshot();
    let bytes = PreparedEntry::<(u64, Record)>::allocation_bytes() as u64;
    let mut records = Records::new(false);
    let mut rejected = PendingPin::new(Some(pin()));
    reject_next(1);
    assert_eq!(records.prepare(&mut rejected), Err(Error::Memory));
    rejected.release();
    assert_eq!(budget::snapshot(), backing_before);
    let mut first = PendingPin::new(Some(pin()));
    records.prepare(&mut first).unwrap();
    assert_eq!(
        budget::snapshot(),
        (
            [backing_before.0[0] + bytes, backing_before.0[1] + 1],
            [backing_before.1[0] + bytes, backing_before.1[1] + 1]
        )
    );
    {
        let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        budget::with_locked(|| records.publish(&mut first, 4096, 1));
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
    let retained = budget::snapshot();
    assert_eq!(retained.1[1], backing_before.1[1] + 1);
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
    assert_eq!(budget::snapshot(), backing_before);

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
    assert_eq!(budget::snapshot(), backing_before);
    #[cfg(target_arch = "aarch64")]
    {
        let mut cache = WalkerCache::new(false);
        reject_next(2);
        assert_eq!(cache.prepare(), Err(Error::Memory));
        cache.prepare().unwrap();
        {
            let heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            // Metadata ABI fixture, no table backing adopted or hardware link.
            budget::with_locked(|| cache.publish(3, crate::memory::physical::PAddr::from(4096u64)));
            assert_eq!(cache.get(&3), Some(&crate::memory::physical::PAddr::from(4096u64)));
            drop(heap);
        }
        cache.prepare().unwrap();
        cache.prepare().unwrap(); // Reuse one unused node after a partial walk.
        let retained = budget::snapshot();
        assert_eq!(retained.1[1], backing_before.1[1] + 2);
        cache.release();
        assert_eq!(budget::snapshot(), backing_before);
    }
    budget::with_ordinary_pressure(|| {
        let mut denied = PendingPin::new(Some(pin()));
        let before = budget::snapshot();
        {
            let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            assert_eq!(Records::new(false).prepare(&mut denied), Err(Error::Memory));
        }
        denied.release();
        assert_eq!(budget::snapshot(), before);
        let mut trusted = Records::new(true);
        let mut admitted = PendingPin::new(Some(pin()));
        trusted.prepare(&mut admitted).unwrap();
        assert_eq!(budget::snapshot().1, before.1);
        trusted.publish(&mut admitted, 8192, 1);
        admitted.release();
        trusted.release();
        assert_eq!(budget::snapshot(), before);
        #[cfg(target_arch = "aarch64")]
        {
            let mut ordinary = WalkerCache::new(false);
            {
                let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
                assert_eq!(ordinary.prepare(), Err(Error::Memory));
            }
            let mut trusted = WalkerCache::new(true);
            trusted.prepare().unwrap();
            assert_eq!(budget::snapshot().1, before.1);
            trusted.release();
            ordinary.release();
            assert_eq!(budget::snapshot(), before);
        }
    });
    object::try_close_cap(root.id(), cap).unwrap();
    crate::memory::close_user_address_space_handle(root).unwrap();
    finish_real();
    super::super::backend_registry::tests::finish_real();
    crate::logln!(
        "[DMA metadata ownership] rejected admission releases pin; heap-held \
         publish/detach/quarantine keeps exact node; duplicate and early close reject; confirmed \
         private completion releases original storage; shared byte/count admission, heap-held \
         denial and trusted headroom passed"
    );
}
