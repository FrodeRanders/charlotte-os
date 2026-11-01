//! Backing admission for capability, IPC endpoint/record and mailbox accounts.
//! Fixed account/holder bundles retain their original storage class through
//! actual strong/weak/allocator lifetime, independently of resource admission.
use alloc::sync::Arc;
use core::alloc::AllocError;

use charlotte_lifecycle::resources::AdmissionPool;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::charged_allocator::ChargedAllocator,
};

pub(crate) type AccountRef<T> = Arc<T, ChargedAllocator<Charge>>;
pub(crate) const NODE_LIMIT: u64 = crate::resource_policy::POLICY.namespace_accounts.total[0];
pub(crate) const ORDINARY_LIMIT: u64 =
    crate::resource_policy::POLICY.namespace_accounts.ordinary[0];
static POOL: Mutex<AdmissionPool<1>> =
    Mutex::new(AdmissionPool::new([NODE_LIMIT], [ORDINARY_LIMIT]));

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    platform: bool,
}

/// Only the four fixed-layout namespace account constructors use this pool.
pub(crate) fn allocator(platform: bool) -> Result<ChargedAllocator<Charge>, AllocError> {
    POOL.lock().reserve(platform, [1]).map_err(|_| AllocError)?;
    ChargedAllocator::try_new(Charge {
        platform,
    })
}
impl Drop for Charge {
    fn drop(&mut self) {
        POOL.lock().release(self.platform, [1]).expect("namespace account budget underflow");
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
