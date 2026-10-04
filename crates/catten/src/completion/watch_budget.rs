//! Endpoint/thread lifecycle and kernel completion-callback entries retain admission until unlinked
//! or detached notification storage is actually released.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::CountBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const MAX_DOMAIN_WATCHES: usize = 1_024;
pub(crate) const MAX_NODE_WATCHES: usize = 8_192;
pub(crate) const MAX_ORDINARY_WATCHES: usize = 6_144;
pub(crate) const MAX_ENDPOINT_WATCHES: usize = 128;
pub(crate) const MAX_THREAD_WATCHES: usize = 128;
pub(crate) const MAX_COMPLETION_CALLBACKS: usize = 128;

struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
}
static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: CountBudget::new(MAX_NODE_WATCHES),
        ordinary: CountBudget::new(MAX_ORDINARY_WATCHES),
    })
});

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<CountBudget>);
impl DomainBudget {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self(Mutex::new(CountBudget::new(capacity.min(MAX_DOMAIN_WATCHES)))))
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
        local.release().expect("event-watch domain budget underflow");
        node.total.release().expect("event-watch node budget underflow");
        if !self.platform {
            node.ordinary.release().expect("event-watch ordinary budget underflow");
        }
    }
}
pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
