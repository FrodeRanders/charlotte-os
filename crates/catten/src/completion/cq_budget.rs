//! CQ registration count and kernel-owned ring/backlog backing bytes. Physical
//! ring frames remain owned by their mappings, not by the CQ registry.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::VectorBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

// Dimensions: registered CQs and allocated kernel backing bytes.
pub(crate) const DOMAIN_LIMIT: [u64; 2] = [32, 256 * 1024];
pub(crate) const NODE_LIMIT: [u64; 2] = [2048, 4 * 1024 * 1024];
pub(crate) const ORDINARY_LIMIT: [u64; 2] = [1536, 3 * 1024 * 1024];

struct NodeBudget {
    total: VectorBudget<2>,
    ordinary: VectorBudget<2>,
}
static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: VectorBudget::new(NODE_LIMIT),
        ordinary: VectorBudget::new(ORDINARY_LIMIT),
    })
});

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<VectorBudget<2>>);
impl DomainBudget {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(VectorBudget::new(DOMAIN_LIMIT))))
    }

    pub(crate) fn used(&self) -> [u64; 2] {
        self.0.lock().used()
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
    amount: [u64; 2],
}
pub(crate) fn reserve(
    domain: &Arc<DomainBudget>,
    platform: bool,
    amount: [u64; 2],
) -> Result<Charge, ()> {
    let mut local = domain.0.lock();
    local.reserve(amount).map_err(|_| ())?;
    let mut node = NODE.lock();
    if node.total.reserve(amount).is_err() {
        local.release(amount).unwrap();
        return Err(());
    }
    if !platform && node.ordinary.reserve(amount).is_err() {
        node.total.release(amount).unwrap();
        local.release(amount).unwrap();
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
        local.release(self.amount).expect("CQ domain budget underflow");
        node.total.release(self.amount).expect("CQ node budget underflow");
        if !self.platform {
            node.ordinary.release(self.amount).expect("CQ ordinary budget underflow");
        }
    }
}
pub(crate) fn node_used() -> ([u64; 2], [u64; 2]) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
