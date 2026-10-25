//! Requested buffer bytes/counts, retained until actual buffer deallocation.
use charlotte_lifecycle::resources::VectorBudget;

use super::*;

pub(super) const TOTAL_LIMIT: [u64; 2] = crate::resource_policy::POLICY.snapshot_buffers.total;
pub(super) const ORDINARY_LIMIT: [u64; 2] =
    crate::resource_policy::POLICY.snapshot_buffers.ordinary;
struct Pool {
    total: VectorBudget<2>,
    ordinary: VectorBudget<2>,
}
static POOL: crate::memory::Mutex<Pool> = crate::memory::Mutex::new(Pool {
    total: VectorBudget::new(TOTAL_LIMIT),
    ordinary: VectorBudget::new(ORDINARY_LIMIT),
});

#[must_use]
pub(super) struct Charge {
    amount: [u64; 2],
    platform: bool,
}
pub(super) fn reserve(platform: bool, bytes: usize) -> Result<Charge, MemoryObjectError> {
    let amount = [u64::try_from(bytes).map_err(|_| MemoryObjectError::ResourceLimit)?, 1];
    let mut pool = POOL.lock();
    pool.total.reserve(amount).map_err(|_| MemoryObjectError::ResourceLimit)?;
    if !platform && pool.ordinary.reserve(amount).is_err() {
        pool.total.release(amount).unwrap();
        return Err(MemoryObjectError::ResourceLimit);
    }
    Ok(Charge {
        amount,
        platform,
    })
}
impl Drop for Charge {
    fn drop(&mut self) {
        let mut pool = POOL.lock();
        pool.total.release(self.amount).expect("snapshot buffer admission underflow");
        if !self.platform {
            pool.ordinary.release(self.amount).expect("ordinary snapshot admission underflow");
        }
    }
}
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    let pool = POOL.lock();
    (pool.total.used(), pool.ordinary.used())
}
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    let _pool = POOL.lock();
    work()
}
