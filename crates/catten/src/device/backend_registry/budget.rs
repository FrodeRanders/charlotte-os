//! Backing admission for domain cells, persistent requester fences and contexts.
//! Original classification follows each node through actual deallocation.
use core::alloc::AllocError;

use charlotte_lifecycle::resources::AdmissionPool;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
    },
};

pub(in crate::device) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(super) type Prepared<T> = PreparedEntry<T, Charge>;
pub(super) type Retired<T> = RetiredEntry<T, Charge>;
pub(super) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.iommu_nodes.total;
pub(super) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.iommu_nodes.ordinary;
static POOL: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(LIMIT, ORDINARY));

#[derive(Debug)]
#[must_use]
pub(in crate::device) struct Charge {
    bytes: u64,
    platform: bool,
}
impl Drop for Charge {
    fn drop(&mut self) {
        POOL.lock().release(self.platform, [self.bytes, 1]).expect("IOMMU node budget underflow");
    }
}
pub(super) fn prepare<T>(platform: bool) -> Result<Prepared<T>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}
fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Prepared<T>, AllocError>,
) -> Result<Prepared<T>, AllocError> {
    let bytes = u64::try_from(Prepared::<T>::allocation_bytes()).map_err(|_| AllocError)?;
    POOL.lock().reserve(platform, [bytes, 1]).map_err(|_| AllocError)?;
    allocate(Charge {
        bytes,
        platform,
    })
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::device) fn snapshot() -> ([u64; 2], [u64; 2]) {
    POOL.lock().used()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::device) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    let _guard = POOL.lock();
    work()
}
#[cfg(feature = "kernel_tests")]
mod tests;
#[cfg(feature = "kernel_tests")]
pub(in crate::device) use tests::{
    run as test_admission,
    with_ordinary_pressure,
};
