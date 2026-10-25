//! Root policy/usage/authority node admission follows actual allocation lifetime,
//! including partial preparation, detached metadata and terminal retention.
use core::alloc::AllocError;

use charlotte_lifecycle::resources::AdmissionPool;

use crate::{
    klib::collections::retirement_list::PreparedEntry,
    memory::Mutex,
};

const NODE_LIMIT: [u64; 2] = crate::resource_policy::POLICY.root_metadata.total;
const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.root_metadata.ordinary;

static POOL: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(NODE_LIMIT, ORDINARY_LIMIT));

#[derive(Debug)]
#[must_use]
pub(in crate::memory) struct Charge {
    bytes: u64,
    platform: bool,
}

impl Charge {
    fn reserve(platform: bool, bytes: usize) -> Result<Self, AllocError> {
        let bytes = u64::try_from(bytes).map_err(|_| AllocError)?;
        let mut pool = POOL.lock();
        pool.reserve(platform, [bytes, 1]).map_err(|_| AllocError)?;
        Ok(Self {
            bytes,
            platform,
        })
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut pool = POOL.lock();
        pool.release(self.platform, [self.bytes, 1]).expect("root metadata node underflow");
    }
}

pub(super) fn prepare<T>(platform: bool) -> Result<PreparedEntry<T, Charge>, AllocError> {
    prepare_with(platform, PreparedEntry::try_new_charged)
}

pub(super) fn prepare_with<T>(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<PreparedEntry<T, Charge>, AllocError>,
) -> Result<PreparedEntry<T, Charge>, AllocError> {
    let charge = Charge::reserve(platform, PreparedEntry::<T, Charge>::allocation_bytes())?;
    allocate(charge)
}

pub(in crate::memory) fn snapshot() -> ([u64; 2], [u64; 2]) {
    let pool = POOL.lock();
    pool.used()
}

pub(super) fn with_locked(work: impl FnOnce()) {
    let _pool = POOL.lock();
    work();
}
