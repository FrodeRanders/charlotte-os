//! Shared capability-record accounting. Legacy paths are counted but bypass
//! policy admission during migration; bounded callers reserve before mutation.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::CountBudget;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const DOMAIN_LIMIT: usize = 4_096;
pub(crate) const NODE_LIMIT: usize = 65_536;
pub(crate) const ORDINARY_LIMIT: usize = 49_152;

#[derive(Debug)]
struct Account {
    used: CountBudget,
    retired: bool,
}

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<Account>);

impl DomainBudget {
    pub(crate) fn try_new() -> Result<Arc<Self>, super::AllocationError> {
        Arc::try_new(Self(Mutex::new(Account {
            used: CountBudget::new(usize::MAX),
            retired: false,
        })))
        .map_err(|_| super::AllocationError::AllocationFailed)
    }

    pub(crate) fn accepting(&self) -> bool {
        !self.0.lock().retired
    }

    pub(crate) fn retire(&self) {
        self.0.lock().retired = true;
    }

    pub(crate) fn used(&self) -> usize {
        self.0.lock().used.used()
    }
}

struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
}

impl NodeBudget {
    const fn new() -> Self {
        Self {
            total: CountBudget::new(usize::MAX),
            ordinary: CountBudget::new(usize::MAX),
        }
    }

    fn reserve(&mut self, platform: bool, bounded: bool) -> Result<(), super::AllocationError> {
        if bounded
            && (self.total.used() >= NODE_LIMIT
                || (!platform && self.ordinary.used() >= ORDINARY_LIMIT))
        {
            return Err(super::AllocationError::ResourceLimit);
        }
        self.total.reserve().map_err(|_| super::AllocationError::ResourceLimit)?;
        if !platform && self.ordinary.reserve().is_err() {
            self.total.release().unwrap();
            return Err(super::AllocationError::ResourceLimit);
        }
        Ok(())
    }

    fn release(&mut self, platform: bool) {
        self.total.release().expect("capability node count underflow");
        if !platform {
            self.ordinary.release().expect("capability ordinary count underflow");
        }
    }
}

static NODE: Mutex<NodeBudget> = Mutex::new(NodeBudget::new());

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
}

pub(crate) fn reserve(
    domain: &Arc<DomainBudget>,
    platform: bool,
    bounded: bool,
) -> Result<Charge, super::AllocationError> {
    let mut local = domain.0.lock();
    if bounded && local.retired {
        return Err(super::AllocationError::Retired);
    }
    if bounded && local.used.used() >= DOMAIN_LIMIT {
        return Err(super::AllocationError::ResourceLimit);
    }
    local.used.reserve().map_err(|_| super::AllocationError::ResourceLimit)?;
    if let Err(error) = NODE.lock().reserve(platform, bounded) {
        local.used.release().unwrap();
        return Err(error);
    }
    Ok(Charge {
        domain: domain.clone(),
        platform,
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut local = self.domain.0.lock();
        local.used.release().expect("capability domain count underflow");
        NODE.lock().release(self.platform);
    }
}

pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}

pub(crate) fn test_node_admission() {
    use super::AllocationError;
    let mut node = NodeBudget::new();
    for _ in 0..ORDINARY_LIMIT {
        node.reserve(false, true).unwrap();
    }
    assert_eq!(node.reserve(false, true), Err(AllocationError::ResourceLimit));
    for _ in ORDINARY_LIMIT..NODE_LIMIT {
        node.reserve(true, true).unwrap();
    }
    assert_eq!(node.reserve(true, true), Err(AllocationError::ResourceLimit));
    // Explicit legacy bypass is counted. It must not silently admit another
    // bounded request or turn policy saturation into a legacy-path panic.
    node.reserve(false, false).unwrap();
    assert_eq!(node.total.used(), NODE_LIMIT + 1);
    assert_eq!(node.reserve(true, true), Err(AllocationError::ResourceLimit));
    node.release(false);
    node.release(false);
    node.reserve(false, true).unwrap();
    for _ in ORDINARY_LIMIT..NODE_LIMIT {
        node.release(true);
    }
    for _ in 0..ORDINARY_LIMIT {
        node.release(false);
    }
    assert_eq!(node.total.used(), 0);
    assert_eq!(node.ordinary.used(), 0);
}
