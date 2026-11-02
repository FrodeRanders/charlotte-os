//! Uniform observer allocation ownership through final weak/allocator release.
//! Count admission covers the fixed private charge holder; actual Arc layout
//! bytes (including header/alignment) are admitted before payload allocation.
use core::{
    alloc::{
        AllocError,
        Layout,
    },
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use charlotte_lifecycle::resources::AdmissionPool;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::charged_allocator::{
        ChargedAllocator,
        PayloadAdmission,
    },
};
pub(crate) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.observer_backing.total;
pub(crate) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.observer_backing.ordinary;
static POOL: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(LIMIT, ORDINARY));

#[derive(Debug)]
pub(crate) struct Charge {
    bytes: AtomicU64,
    platform: bool,
}
impl Drop for Charge {
    fn drop(&mut self) {
        POOL.lock()
            .release(self.platform, [self.bytes.load(Ordering::Acquire), 1])
            .expect("observer backing budget underflow");
    }
}
#[derive(Debug)]
pub(crate) struct LayoutAdmission;
impl PayloadAdmission<Charge> for LayoutAdmission {
    fn reserve(charge: &Charge, layout: Layout) -> Result<(), AllocError> {
        let bytes = u64::try_from(layout.size()).map_err(|_| AllocError)?;
        POOL.lock().reserve(charge.platform, [bytes, 0]).map_err(|_| AllocError)?;
        assert_eq!(charge.bytes.swap(bytes, Ordering::AcqRel), 0);
        Ok(())
    }

    fn rollback(charge: &Charge, layout: Layout) {
        let bytes = charge.bytes.swap(0, Ordering::AcqRel);
        assert_eq!(bytes, layout.size() as u64);
        POOL.lock().release(charge.platform, [bytes, 0]).expect("observer byte rollback underflow");
    }
}
pub(crate) type Allocator = ChargedAllocator<Charge, LayoutAdmission>;
pub(super) fn allocator(platform: bool) -> Result<Allocator, AllocError> {
    POOL.lock().reserve(platform, [0, 1]).map_err(|_| AllocError)?;
    Allocator::try_new_with_policy(Charge {
        bytes: AtomicU64::new(0),
        platform,
    })
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    POOL.lock().used()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    let _guard = POOL.lock();
    work();
}

#[cfg(feature = "kernel_tests")]
#[path = "backing/tests.rs"]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(crate) use tests::test_admission;
