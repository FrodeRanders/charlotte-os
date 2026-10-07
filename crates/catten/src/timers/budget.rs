//! Shared node admission for completion and scheduler timer events, including cancelled events
//! awaiting reclamation on another LP. A charge follows event/node backing and
//! cancellation references, not the shorter-lived completion record or queue membership.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::CountBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const MAX_DOMAIN_TIMERS: usize = 1_024;
pub(crate) const MAX_NODE_TIMERS: usize = 8_192;
pub(crate) const MAX_ORDINARY_TIMERS: usize = MAX_NODE_TIMERS * 3 / 4;

struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
}

static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: CountBudget::new(MAX_NODE_TIMERS),
        ordinary: CountBudget::new(MAX_ORDINARY_TIMERS),
    })
});

/// A fresh namespace gets a fresh owner. Outstanding old-generation events
/// retain this Arc and cannot credit a replacement's admission counter.
#[derive(Debug)]
struct Account {
    count: CountBudget,
    platform: bool,
    retired: bool,
}

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<Account>);

impl DomainBudget {
    pub(crate) fn new(completion_capacity: usize) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Account {
            count: CountBudget::new(completion_capacity.min(MAX_DOMAIN_TIMERS)),
            platform: false,
            retired: false,
        })))
    }

    pub(crate) fn used(&self) -> usize {
        self.0.lock().count.used()
    }
}

/// Captured from the generation-owned memory ledger before Thread publication.
/// Scheduling paths reserve only counter locks, not the address-space table.
#[derive(Debug, Clone)]
pub(crate) struct SchedulerSponsor(Arc<DomainBudget>);

impl SchedulerSponsor {
    pub(crate) fn new(platform: bool) -> Self {
        let domain = DomainBudget::new(MAX_DOMAIN_TIMERS);
        domain.0.lock().platform = platform;
        Self(domain)
    }

    pub(crate) fn reserve(&self) -> Result<Charge, ()> {
        reserve_inner(&self.0, None)
    }

    pub(crate) fn retire(&self) {
        self.0.0.lock().retired = true;
    }

    pub(crate) fn mark_platform(&self) {
        self.0.0.lock().platform = true;
    }

    pub(crate) fn used(&self) -> usize {
        self.0.used()
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
}

pub(crate) fn reserve(domain: &Arc<DomainBudget>, platform: bool) -> Result<Charge, ()> {
    reserve_inner(domain, Some(platform))
}

fn reserve_inner(domain: &Arc<DomainBudget>, platform: Option<bool>) -> Result<Charge, ()> {
    let mut local = domain.0.lock();
    if local.retired {
        return Err(());
    }
    let platform = platform.unwrap_or(local.platform);
    local.count.reserve().map_err(|_| ())?;
    let mut node = NODE.lock();
    if node.total.reserve().is_err() {
        local.count.release().unwrap();
        return Err(());
    }
    if !platform && node.ordinary.reserve().is_err() {
        node.total.release().unwrap();
        local.count.release().unwrap();
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
        local.count.release().expect("timer domain budget underflow");
        node.total.release().expect("timer node budget underflow");
        if !self.platform {
            node.ordinary.release().expect("timer ordinary budget underflow");
        }
    }
}

pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
