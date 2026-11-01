//! Endpoint-node backing admission, independent of mailbox resource counts.
//! Candidates and detached storage keep their original byte/count charge.
use core::alloc::AllocError;

use charlotte_lifecycle::resources::AdmissionPool;

use super::{
    AdmittedMailboxEndpoint,
    MailboxCap,
    mailbox_budget::Error,
};
use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::collections::retirement_list::{
        AdmittedMap,
        PreparedEntry,
        RetiredEntry,
    },
};
pub(super) type Map = AdmittedMap<MailboxCap, AdmittedMailboxEndpoint, Charge>;
pub(super) type Prepared = PreparedEntry<(MailboxCap, AdmittedMailboxEndpoint), Charge>;
type Node = RetiredEntry<(MailboxCap, AdmittedMailboxEndpoint), Charge>;
pub(super) const LIMIT: [u64; 2] = crate::resource_policy::POLICY.mailbox_nodes.total;
pub(super) const ORDINARY: [u64; 2] = crate::resource_policy::POLICY.mailbox_nodes.ordinary;
static POOL: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(LIMIT, ORDINARY));

#[derive(Debug)]
#[must_use]
pub(super) struct Charge {
    bytes: u64,
    platform: bool,
}
impl Drop for Charge {
    fn drop(&mut self) {
        POOL.lock().release(self.platform, [self.bytes, 1]).expect("mailbox node budget underflow");
    }
}
fn reserve(bytes: u64, platform: bool) -> Result<Charge, Error> {
    POOL.lock().reserve(platform, [bytes, 1]).map_err(|_| Error::ResourceLimit)?;
    Ok(Charge {
        bytes,
        platform,
    })
}
pub(super) fn prepare(platform: bool) -> Result<Prepared, Error> {
    prepare_with(platform, Prepared::try_new_charged)
}
fn prepare_with(
    platform: bool,
    allocate: impl FnOnce(Charge) -> Result<Prepared, AllocError>,
) -> Result<Prepared, Error> {
    let bytes = u64::try_from(Prepared::allocation_bytes()).map_err(|_| Error::ResourceLimit)?;
    allocate(reserve(bytes, platform)?).map_err(|_| Error::AllocationFailed)
}

/// Detached storage retains both backing admission and its mailbox resource
/// charge. Inner RetiredEntry abandonment retains the entire node inertly.
#[must_use]
pub(super) struct Retired(Node);
impl Retired {
    pub(super) fn new(node: Node) -> Self {
        Self(node)
    }

    pub(super) fn release(self) {
        let mut resource_charge = None;
        self.0.release_with(|(_, endpoint)| resource_charge = Some(endpoint._charge));
        // Both node admission and resource admission outlive node deallocation.
        drop(resource_charge);
    }
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
pub(super) fn test_rejection() {
    let before = snapshot();
    for _ in 0..64 {
        assert!(matches!(prepare_with(false, |_| Err(AllocError)), Err(Error::AllocationFailed)));
        assert_eq!(snapshot(), before);
    }
    assert!(matches!(reserve(ORDINARY[0] + 1, false), Err(Error::ResourceLimit)));
    assert!(matches!(reserve(LIMIT[0] + 1, true), Err(Error::ResourceLimit)));
    assert_eq!(snapshot(), before);
}
