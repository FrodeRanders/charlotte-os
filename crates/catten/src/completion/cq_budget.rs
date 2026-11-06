//! CQ registration count and kernel-owned ring/backlog backing bytes.
//! Mappings retain ownership of the physical ring frames independently of the CQ registry.

use charlotte_lifecycle::resources::VectorBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) type AccountRef = super::account_budget::AccountRef<DomainBudget>;

// Dimensions: registered CQs and allocated kernel backing bytes.
pub(crate) const DOMAIN_LIMIT: [u64; 2] = crate::resource_policy::POLICY.completion_queues.domain;
pub(crate) const NODE_LIMIT: [u64; 2] = crate::resource_policy::POLICY.completion_queues.pool.total;
pub(crate) const ORDINARY_LIMIT: [u64; 2] =
    crate::resource_policy::POLICY.completion_queues.pool.ordinary;

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
    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn new() -> AccountRef {
        Self::try_new(false).expect("mandatory CQ account allocation failed")
    }

    pub(crate) fn try_new(
        storage: impl Into<crate::memory::kernel_buffers::CapturedStorage>,
    ) -> Result<AccountRef, core::alloc::AllocError> {
        let allocator = super::account_budget::allocator(storage)?;
        allocator.try_arc(Self(Mutex::new(VectorBudget::new(DOMAIN_LIMIT))))
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn used(&self) -> [u64; 2] {
        self.0.lock().used()
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    domain: AccountRef,
    platform: bool,
    amount: [u64; 2],
}
pub(crate) fn reserve(domain: &AccountRef, platform: bool, amount: [u64; 2]) -> Result<Charge, ()> {
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
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn node_used() -> ([u64; 2], [u64; 2]) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
