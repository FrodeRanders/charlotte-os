//! Requested bytes and bundle counts for stable thread contexts and their
//! preallocated retirement nodes. The existing principal history sponsors all
//! generations; no ASID lookup is needed at final post-guard deallocation.
use charlotte_lifecycle::resources::VectorBudget;

use crate::memory::{
    Mutex,
    usage::principal_history::ThreadMetadataCharge,
};

const NODE_LIMIT: [u64; 2] = [8 * 1024 * 1024, 4096];
const ORDINARY_LIMIT: [u64; 2] = [6 * 1024 * 1024, 3072];

struct NodeBudget {
    total: VectorBudget<2>,
    ordinary: VectorBudget<2>,
}

static NODE: Mutex<NodeBudget> = Mutex::new(NodeBudget {
    total: VectorBudget::new(NODE_LIMIT),
    ordinary: VectorBudget::new(ORDINARY_LIMIT),
});

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
            node.total.reserve(amount).map_err(|_| super::Error::Limit)?;
            if !platform && node.ordinary.reserve(amount).is_err() {
                node.total.release(amount).unwrap();
                return Err(super::Error::Limit);
            }
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
        node.total.release(self.amount).expect("thread metadata node underflow");
        if !self.platform {
            node.ordinary.release(self.amount).expect("thread metadata ordinary underflow");
        }
    }
}

pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}

/// Serialized boot qualification: fallback must not enter this counter lock.
pub(in crate::memory) fn with_locked(work: impl FnOnce()) {
    let _node = NODE.lock();
    work();
}
