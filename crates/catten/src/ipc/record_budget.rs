//! Record sponsorship survives delegation and namespace retirement. Dimensions
//! are connection capabilities, retained calls, and outstanding reply tokens.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::VectorBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const DOMAIN_LIMIT: [u64; 3] = [512, 512, 512];
pub(crate) const NODE_LIMIT: [u64; 3] = [8_192, 8_192, 8_192];
pub(crate) const ORDINARY_LIMIT: [u64; 3] = [6_144, 6_144, 6_144];

struct NodeBudget {
    total: VectorBudget<3>,
    ordinary: VectorBudget<3>,
}

static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: VectorBudget::new(NODE_LIMIT),
        ordinary: VectorBudget::new(ORDINARY_LIMIT),
    })
});

#[derive(Debug)]
struct DomainState {
    records: VectorBudget<3>,
    retired: bool,
}

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<DomainState>);

impl DomainBudget {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(DomainState {
            records: VectorBudget::new(DOMAIN_LIMIT),
            retired: false,
        })))
    }

    pub(crate) fn used(&self) -> [u64; 3] {
        self.0.lock().records.used()
    }

    pub(crate) fn retire(&self) {
        self.0.lock().retired = true;
    }

    pub(crate) fn accepting(&self) -> bool {
        !self.0.lock().retired
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
    amount: [u64; 3],
}

impl Charge {
    pub(crate) fn platform(&self) -> bool {
        self.platform
    }

    /// Divide one atomic reservation into independently retained record owners.
    pub(crate) fn split(&mut self, amount: [u64; 3]) -> Self {
        for (remaining, part) in self.amount.iter_mut().zip(amount) {
            *remaining = remaining.checked_sub(part).expect("IPC record charge split underflow");
        }
        Self {
            domain: self.domain.clone(),
            platform: self.platform,
            amount,
        }
    }
}

pub(crate) fn reserve(
    domain: &Arc<DomainBudget>,
    platform: bool,
    amount: [u64; 3],
) -> Result<Charge, ()> {
    let mut local = domain.0.lock();
    if local.retired {
        return Err(());
    }
    local.records.reserve(amount).map_err(|_| ())?;
    let mut node = NODE.lock();
    if node.total.reserve(amount).is_err() {
        local.records.release(amount).unwrap();
        return Err(());
    }
    if !platform && node.ordinary.reserve(amount).is_err() {
        node.total.release(amount).unwrap();
        local.records.release(amount).unwrap();
        return Err(());
    }
    Ok(Charge {
        domain: domain.clone(),
        platform,
        amount,
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut local = self.domain.0.lock();
        let mut node = NODE.lock();
        local.records.release(self.amount).expect("IPC record domain budget underflow");
        node.total.release(self.amount).expect("IPC record node budget underflow");
        if !self.platform {
            node.ordinary.release(self.amount).expect("IPC record ordinary budget underflow");
        }
    }
}

pub(crate) fn node_used() -> ([u64; 3], [u64; 3]) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
