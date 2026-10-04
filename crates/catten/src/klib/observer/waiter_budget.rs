//! Generation-owned admission for scheduler waiter entries. Kernel callback
//! observer implementations remain explicitly outside this budget.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::CountBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const DOMAIN_LIMIT: usize = 1_024;
pub(crate) const NODE_LIMIT: usize = 8_192;
pub(crate) const ORDINARY_LIMIT: usize = 6_144;
pub(crate) const SOURCE_LIMIT: usize = 64;

#[derive(Debug)]
struct Account {
    count: CountBudget,
    platform: bool,
    retired: bool,
}
#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<Account>);
impl DomainBudget {
    pub(crate) fn new(platform: bool) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Account {
            count: CountBudget::new(DOMAIN_LIMIT),
            platform,
            retired: false,
        })))
    }

    pub(crate) fn used(&self) -> usize {
        self.0.lock().count.used()
    }

    pub(crate) fn retire(&self) {
        self.0.lock().retired = true;
    }

    pub(crate) fn mark_platform(&self) {
        self.0.lock().platform = true;
    }
}
struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
}
static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: CountBudget::new(NODE_LIMIT),
        ordinary: CountBudget::new(ORDINARY_LIMIT),
    })
});
#[derive(Debug)]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
}
pub(crate) fn reserve(
    domain: &Arc<DomainBudget>,
) -> Result<Charge, super::registration::RegistrationError> {
    use super::registration::RegistrationError;
    let mut local = domain.0.lock();
    if local.retired {
        return Err(RegistrationError::Closed);
    }
    local.count.reserve().map_err(|_| RegistrationError::ResourceLimit)?;
    let mut node = NODE.lock();
    if node.total.reserve().is_err() {
        local.count.release().unwrap();
        return Err(RegistrationError::ResourceLimit);
    }
    if !local.platform && node.ordinary.reserve().is_err() {
        node.total.release().unwrap();
        local.count.release().unwrap();
        return Err(RegistrationError::ResourceLimit);
    }
    Ok(Charge {
        domain: domain.clone(),
        platform: local.platform,
    })
}
impl Drop for Charge {
    fn drop(&mut self) {
        let mut local = self.domain.0.lock();
        let mut node = NODE.lock();
        local.count.release().expect("waiter domain budget underflow");
        node.total.release().expect("waiter node budget underflow");
        if !self.platform {
            node.ordinary.release().expect("waiter ordinary budget underflow");
        }
    }
}
pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
