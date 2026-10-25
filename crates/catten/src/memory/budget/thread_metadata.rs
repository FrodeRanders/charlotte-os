//! Requested bytes and bundle counts for stable thread contexts and their
//! preallocated retirement nodes. The existing principal history sponsors all
//! generations; no ASID lookup is needed at final post-guard deallocation.
use charlotte_lifecycle::resources::AdmissionPool;

use crate::memory::{
    Mutex,
    usage::principal_history::ThreadMetadataCharge,
};

const NODE_LIMIT: [u64; 2] = crate::resource_policy::POLICY.thread_metadata.total;
const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.thread_metadata.ordinary;

static NODE: Mutex<AdmissionPool<2>> = Mutex::new(AdmissionPool::new(NODE_LIMIT, ORDINARY_LIMIT));

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    amount: [u64; 2],
    platform: bool,
    // Field release occurs after Drop's node guard has left. No root/generation
    // lookup can redirect a late refund to a replacement domain or principal.
    _principal: Option<ThreadMetadataCharge>,
}

impl Charge {
    pub(super) fn reserve(
        principal: Option<u64>,
        platform: bool,
        bytes: usize,
    ) -> Result<Self, super::Error> {
        let amount = [u64::try_from(bytes).map_err(|_| super::Error::Limit)?, 1];
        let principal = principal
            .map(|principal| ThreadMetadataCharge::try_new(principal, amount))
            .transpose()
            .map_err(|_| super::Error::Limit)?;
        {
            let mut node = NODE.lock();
            node.reserve(platform, amount).map_err(|_| super::Error::Limit)?;
        }
        Ok(Self {
            amount,
            platform,
            _principal: principal,
        })
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut node = NODE.lock();
        node.release(self.platform, self.amount).expect("thread metadata node underflow");
    }
}

pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    let node = NODE.lock();
    node.used()
}

/// Serialized boot qualification: fallback must not enter this counter lock.
pub(in crate::memory) fn with_locked(work: impl FnOnce()) {
    let _node = NODE.lock();
    work();
}
