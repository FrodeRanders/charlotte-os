//! Mapping-node layout admission follows actual allocation lifetime, including
//! unused candidates and detached or abandoned retirement records.
use charlotte_lifecycle::resources::VectorBudget;

use super::*;

const TOTAL_LIMIT: [u64; 2] = crate::resource_policy::POLICY.mapping_nodes.total;
const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.mapping_nodes.ordinary;
struct Pool {
    total: VectorBudget<2>,
    ordinary: VectorBudget<2>,
}
static POOL: crate::memory::Mutex<Pool> = crate::memory::Mutex::new(Pool {
    total: VectorBudget::new(TOTAL_LIMIT),
    ordinary: VectorBudget::new(ORDINARY_LIMIT),
});

#[derive(Debug)]
#[must_use]
pub(in crate::memory::object) struct Charge {
    bytes: u64,
    platform: bool,
}
impl Charge {
    fn reserve(platform: bool) -> Result<Self, MemoryObjectError> {
        let bytes = Storage::allocation_bytes() as u64;
        let mut pool = POOL.lock();
        pool.total.reserve([bytes, 1]).map_err(|_| MemoryObjectError::ResourceLimit)?;
        if !platform && pool.ordinary.reserve([bytes, 1]).is_err() {
            pool.total.release([bytes, 1]).unwrap();
            return Err(MemoryObjectError::ResourceLimit);
        }
        Ok(Self {
            bytes,
            platform,
        })
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        let mut pool = POOL.lock();
        pool.total.release([self.bytes, 1]).expect("mapping node admission underflow");
        if !self.platform {
            pool.ordinary.release([self.bytes, 1]).expect("ordinary mapping admission underflow");
        }
    }
}

pub(super) fn prepare(platform: bool) -> Result<Storage, MemoryObjectError> {
    prepare_with(platform, |charge| {
        Storage::try_new_charged(charge).map_err(|_| MemoryObjectError::ResourceLimit)
    })
}
fn prepare_with(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Storage, MemoryObjectError>,
) -> Result<Storage, MemoryObjectError> {
    allocate(Charge::reserve(platform)?)
}
pub(super) fn snapshot() -> ([u64; 2], [u64; 2]) {
    let pool = POOL.lock();
    (pool.total.used(), pool.ordinary.used())
}
pub(super) fn with_locked<T>(work: impl FnOnce() -> T) -> T {
    let _pool = POOL.lock();
    work()
}

pub(super) mod tests;
