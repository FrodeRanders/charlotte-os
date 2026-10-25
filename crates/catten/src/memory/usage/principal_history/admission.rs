//! One charge follows each candidate/published history node through actual
//! allocation lifetime. Ordinary work cannot consume the reserved quarter.
use core::alloc::AllocError;

use charlotte_lifecycle::resources::VectorBudget;

use crate::memory::Mutex;

const NODE_LIMIT: [u64; 2] = crate::resource_policy::POLICY.principal_history.total;
const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.principal_history.ordinary;

struct Pool {
    total: VectorBudget<2>,
    ordinary: VectorBudget<2>,
}
static POOL: Mutex<Pool> = Mutex::new(Pool {
    total: VectorBudget::new(NODE_LIMIT),
    ordinary: VectorBudget::new(ORDINARY_LIMIT),
});

#[derive(Debug)]
#[must_use]
pub(in crate::memory) struct Charge {
    bytes: u64,
    platform: bool,
}

pub(super) fn reserve(platform: bool, bytes: usize) -> Result<Charge, AllocError> {
    let bytes = u64::try_from(bytes).map_err(|_| AllocError)?;
    let mut pool = POOL.lock();
    pool.total.reserve([bytes, 1]).map_err(|_| AllocError)?;
    if !platform && pool.ordinary.reserve([bytes, 1]).is_err() {
        pool.total.release([bytes, 1]).unwrap();
        return Err(AllocError);
    }
    Ok(Charge {
        bytes,
        platform,
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut pool = POOL.lock();
        pool.total.release([self.bytes, 1]).expect("history admission node underflow");
        if !self.platform {
            pool.ordinary.release([self.bytes, 1]).expect("history admission ordinary underflow");
        }
    }
}

pub(in crate::memory) fn snapshot() -> ([u64; 2], [u64; 2]) {
    let pool = POOL.lock();
    (pool.total.used(), pool.ordinary.used())
}

pub(super) fn with_locked(work: impl FnOnce()) {
    let _pool = POOL.lock();
    work();
}
