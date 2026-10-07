//! Charges retained by completion objects and undelivered detached results.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::CountBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const MAX_DOMAIN_RECORDS: usize = 1_024;
pub(crate) const MAX_NODE_RECORDS: usize = 8_192;
pub(crate) const MAX_ORDINARY_RECORDS: usize = MAX_NODE_RECORDS * 3 / 4;

struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
}
static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: CountBudget::new(MAX_NODE_RECORDS),
        ordinary: CountBudget::new(MAX_ORDINARY_RECORDS),
    })
});

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<CountBudget>);
impl DomainBudget {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self(Mutex::new(CountBudget::new(capacity.min(MAX_DOMAIN_RECORDS)))))
    }

    pub(crate) fn used(&self) -> usize {
        self.0.lock().used()
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
}

impl Charge {
    pub(crate) fn platform(&self) -> bool {
        self.platform
    }
}

pub(crate) fn reserve(domain: &Arc<DomainBudget>, platform: bool) -> Result<Charge, ()> {
    let mut local = domain.0.lock();
    local.reserve().map_err(|_| ())?;
    let mut node = NODE.lock();
    if node.total.reserve().is_err() {
        local.release().unwrap();
        return Err(());
    }
    if !platform && node.ordinary.reserve().is_err() {
        node.total.release().unwrap();
        local.release().unwrap();
        return Err(());
    }
    Ok(Charge {
        domain: domain.clone(),
        platform,
    })
}
impl Drop for Charge {
    fn drop(&mut self) {
        let mut local = self.domain.0.lock();
        let mut node = NODE.lock();
        local.release().expect("completion record domain budget underflow");
        node.total.release().expect("completion record node budget underflow");
        if !self.platform {
            node.ordinary.release().expect("completion record ordinary budget underflow");
        }
    }
}
pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
