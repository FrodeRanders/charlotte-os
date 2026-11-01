//! Backing admission for scheduler and completion timer-account bundles.
//! Original classification follows the account Arc and private allocator holder
//! through final strong/weak/allocator release, independently of event admission.
use core::alloc::AllocError;

use charlotte_lifecycle::resources::AdmissionPool;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.timer_accounts.total[0];
pub(crate) const ORDINARY_LIMIT: u64 = crate::resource_policy::POLICY.timer_accounts.ordinary[0];

static POOL: Mutex<AdmissionPool<1>> =
    Mutex::new(AdmissionPool::new([NODE_LIMIT], [ORDINARY_LIMIT]));

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    platform: bool,
}

pub(super) fn reserve(platform: bool) -> Result<Charge, AllocError> {
    POOL.lock().reserve(platform, [1]).map_err(|_| AllocError)?;
    Ok(Charge {
        platform,
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        POOL.lock().release(self.platform, [1]).expect("timer account budget underflow");
    }
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> (u64, u64) {
    let (total, ordinary) = POOL.lock().used();
    (total[0], ordinary[0])
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    let _pool = POOL.lock();
    work();
}
