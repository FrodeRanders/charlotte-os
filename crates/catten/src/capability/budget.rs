//! Shared capability-record admission. Every kind reserves before mutation;
//! there is no unbounded allocation or retirement bypass.

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
            used: CountBudget::new(DOMAIN_LIMIT),
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
            total: CountBudget::new(NODE_LIMIT),
            ordinary: CountBudget::new(ORDINARY_LIMIT),
        }
    }

    fn reserve(&mut self, platform: bool) -> Result<(), super::AllocationError> {
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
) -> Result<Charge, super::AllocationError> {
    let mut local = domain.0.lock();
    if local.retired {
        return Err(super::AllocationError::Retired);
    }
    local.used.reserve().map_err(|_| super::AllocationError::ResourceLimit)?;
    if let Err(error) = NODE.lock().reserve(platform) {
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
        node.reserve(false).unwrap();
    }
    assert_eq!(node.reserve(false), Err(AllocationError::ResourceLimit));
    for _ in ORDINARY_LIMIT..NODE_LIMIT {
        node.reserve(true).unwrap();
    }
    assert_eq!(node.reserve(true), Err(AllocationError::ResourceLimit));
    assert_eq!(node.reserve(false), Err(AllocationError::ResourceLimit));
    assert_eq!(node.total.used(), NODE_LIMIT);
    node.release(false);
    node.reserve(false).unwrap();
    for _ in ORDINARY_LIMIT..NODE_LIMIT {
        node.release(true);
    }
    for _ in 0..ORDINARY_LIMIT {
        node.release(false);
    }
    assert_eq!(node.total.used(), 0);
    assert_eq!(node.ordinary.used(), 0);
}
