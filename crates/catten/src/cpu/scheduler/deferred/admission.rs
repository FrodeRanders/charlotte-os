//! Independent backing admission for close/readiness source disposal nodes.
//! Original bytes/class follow preparation and producer-LP queued storage.
use core::alloc::AllocError;

use charlotte_lifecycle::resources::AdmissionPool;

use super::PreparedDisposal;
use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.deferred_nodes.total;
pub(crate) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.deferred_nodes.ordinary;
static POOL: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(LIMIT, ORDINARY));

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    bytes: u64,
    platform: bool,
}
impl Drop for Charge {
    fn drop(&mut self) {
        POOL.lock()
            .release(self.platform, [self.bytes, 1])
            .expect("deferred node budget underflow");
    }
}
fn reserve(bytes: u64, platform: bool) -> Result<Charge, AllocError> {
    POOL.lock().reserve(platform, [bytes, 1]).map_err(|_| AllocError)?;
    Ok(Charge {
        bytes,
        platform,
    })
}
pub(crate) fn prepare(platform: bool) -> Result<PreparedDisposal, AllocError> {
    prepare_with(platform, PreparedDisposal::try_new_charged)
}
fn prepare_with(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<PreparedDisposal, AllocError>,
) -> Result<PreparedDisposal, AllocError> {
    let bytes = u64::try_from(PreparedDisposal::allocation_bytes()).map_err(|_| AllocError)?;
    allocate(reserve(bytes, platform)?)
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    POOL.lock().used()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    let _pool = POOL.lock();
    work();
}
#[cfg(any(test, feature = "kernel_tests"))]
mod tests;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) use tests::test_source;
