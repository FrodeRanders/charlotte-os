//! Requested-byte/count admission for the five global namespace map layouts.
//! Original classification follows prepared, published and detached storage
//! through actual node deallocation; logical retirement does not refund it.
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

pub(crate) type Map<K, V> = AdmittedMap<K, V, Charge>;
pub(crate) type Prepared<T> = PreparedEntry<T, Charge>;
pub(crate) type Retired<T> = RetiredEntry<T, Charge>;
pub(crate) const NODE_LIMIT: [u64; 2] = crate::resource_policy::POLICY.namespace_nodes.total;
pub(crate) const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.namespace_nodes.ordinary;
static POOL: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(NODE_LIMIT, ORDINARY_LIMIT));

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
            .expect("namespace node admission underflow");
    }
}
pub(crate) fn prepare<T>(platform: bool) -> Result<Prepared<T>, AllocError> {
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
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    POOL.lock().used()
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn with_locked(work: impl FnOnce()) {
    let _pool = POOL.lock();
    work();
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_allocation<T>(family: &'static str) {
    let before = snapshot();
    for _ in 0..64 {
        assert!(prepare_with::<T>(false, |_| Err(AllocError)).is_err());
        assert_eq!(snapshot(), before);
    }
    crate::logln!(
        "[namespace node allocation] family={} bytes={}; 64 allocator rejections passed",
        family,
        Prepared::<T>::allocation_bytes()
    );
}
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_byte_denial() {
    let before = snapshot();
    assert!(
        prepare_with::<[u8; ORDINARY_LIMIT[0] as usize]>(false, |_| panic!(
            "ordinary byte denial reached allocator"
        ))
        .is_err()
    );
    assert!(
        prepare_with::<[u8; NODE_LIMIT[0] as usize]>(true, |_| panic!(
            "total byte denial reached allocator"
        ))
        .is_err()
    );
    assert_eq!(snapshot(), before);
}
