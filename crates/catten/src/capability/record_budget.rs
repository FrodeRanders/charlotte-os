//! Authority node backing, independent of published capability admission.
//! Candidates and detached nodes retain their original byte/count reservation.
use core::alloc::AllocError;

use charlotte_lifecycle::resources::AdmissionPool;

use super::{
    AllocationError,
    Entry,
    ObjectCapability,
};
use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
    },
};

pub(super) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(super) type Prepared = PreparedEntry<(ObjectCapability, Entry), Charge>;
pub(super) type Retired = RetiredEntry<(ObjectCapability, Entry), Charge>;
pub(super) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.authority_nodes.total;
pub(super) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.authority_nodes.ordinary;
static POOL: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(LIMIT, ORDINARY));

#[derive(Debug)]
#[must_use]
pub(super) struct Charge {
    bytes: u64,
    platform: bool,
}
impl Drop for Charge {
    fn drop(&mut self) {
        POOL.lock()
            .release(self.platform, [self.bytes, 1])
            .expect("authority node budget underflow");
    }
}
fn reserve(bytes: u64, platform: bool) -> Result<Charge, AllocationError> {
    POOL.lock().reserve(platform, [bytes, 1]).map_err(|_| AllocationError::ResourceLimit)?;
    Ok(Charge {
        bytes,
        platform,
    })
}
pub(super) fn prepare(platform: bool) -> Result<Prepared, AllocationError> {
    prepare_with(platform, Prepared::try_new_charged)
}
fn prepare_with(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Prepared, AllocError>,
) -> Result<Prepared, AllocationError> {
    let bytes =
        u64::try_from(Prepared::allocation_bytes()).map_err(|_| AllocationError::ResourceLimit)?;
    let charge = reserve(bytes, platform)?;
    allocate(charge).map_err(|_| AllocationError::AllocationFailed)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    POOL.lock().used()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn with_locked(work: impl FnOnce()) {
    let _pool = POOL.lock();
    work();
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(super) fn test_rejection() {
    let before = snapshot();
    for _ in 0..64 {
        assert!(matches!(
            prepare_with(false, |_| Err(AllocError)),
            Err(AllocationError::AllocationFailed)
        ));
        assert_eq!(snapshot(), before);
    }
    // The real layout is currently count-bound. Independently exercise byte
    // denial before invoking the allocator, including future layout growth.
    assert!(matches!(reserve(ORDINARY[0] + 1, false), Err(AllocationError::ResourceLimit)));
    assert!(matches!(reserve(LIMIT[0] + 1, true), Err(AllocationError::ResourceLimit)));
    assert_eq!(snapshot(), before);
}
