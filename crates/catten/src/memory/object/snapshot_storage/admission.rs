//! Requested buffer bytes/counts, retained until actual buffer deallocation.
use charlotte_lifecycle::resources::AdmissionPool;

use super::*;

pub(super) const TOTAL_LIMIT: [u64; 2] = crate::resource_policy::POLICY.snapshot_buffers.total;
pub(super) const ORDINARY_LIMIT: [u64; 2] =
    crate::resource_policy::POLICY.snapshot_buffers.ordinary;
static POOL: crate::memory::Mutex<AdmissionPool<2>> =
    crate::memory::Mutex::new(AdmissionPool::new(TOTAL_LIMIT, ORDINARY_LIMIT));

#[must_use]
pub(super) struct Charge {
    amount: [u64; 2],
    platform: bool,
}
pub(super) fn reserve(platform: bool, bytes: usize) -> Result<Charge, MemoryObjectError> {
    let amount = [u64::try_from(bytes).map_err(|_| MemoryObjectError::ResourceLimit)?, 1];
    let mut pool = POOL.lock();
    pool.reserve(platform, amount).map_err(|_| MemoryObjectError::ResourceLimit)?;
    Ok(Charge {
        amount,
        platform,
    })
}
impl Drop for Charge {
    fn drop(&mut self) {
        let mut pool = POOL.lock();
        pool.release(self.platform, self.amount).expect("snapshot buffer admission underflow");
    }
}
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    let pool = POOL.lock();
    pool.used()
}
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    let _pool = POOL.lock();
    work()
}
