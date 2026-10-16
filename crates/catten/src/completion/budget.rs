//! Charges retained by completion objects and undelivered detached results.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::VectorBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const MAX_DOMAIN_RECORDS: usize = 1_024;
pub(crate) const MAX_NODE_RECORDS: usize = 8_192;
pub(crate) const MAX_ORDINARY_RECORDS: usize = MAX_NODE_RECORDS * 3 / 4;

struct NodeBudget {
    // One checked dimension permits constant-time refunds of an owning
    // batch, using the existing shared primitive without changing its API.
    total: VectorBudget<1>,
    ordinary: VectorBudget<1>,
}
static NODE: LazyLock<Mutex<NodeBudget>> = LazyLock::new(|| {
    Mutex::new(NodeBudget {
        total: VectorBudget::new([MAX_NODE_RECORDS as u64]),
        ordinary: VectorBudget::new([MAX_ORDINARY_RECORDS as u64]),
    })
});

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<VectorBudget<1>>);
impl DomainBudget {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Self::try_new(capacity).expect("mandatory completion account allocation failed")
    }

    pub(crate) fn try_new(capacity: usize) -> Result<Arc<Self>, core::alloc::AllocError> {
        Arc::try_new(Self(Mutex::new(VectorBudget::new([capacity.min(MAX_DOMAIN_RECORDS) as u64]))))
    }

    pub(crate) fn used(&self) -> usize {
        self.0.lock().used()[0] as usize
    }
}

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
    count: u64,
}

impl Charge {
    pub(crate) fn platform(&self) -> bool {
        self.platform
    }
}

pub(crate) fn reserve(domain: &Arc<DomainBudget>, platform: bool) -> Result<Charge, ()> {
    let mut local = domain.0.lock();
    local.reserve([1]).map_err(|_| ())?;
    let mut node = NODE.lock();
    if node.total.reserve([1]).is_err() {
        local.release([1]).unwrap();
        return Err(());
    }
    if !platform && node.ordinary.reserve([1]).is_err() {
        node.total.release([1]).unwrap();
        local.release([1]).unwrap();
        return Err(());
    }
    Ok(Charge {
        domain: domain.clone(),
        platform,
        count: 1,
    })
}
impl Drop for Charge {
    fn drop(&mut self) {
        if self.count == 0 {
            return;
        }
        let mut local = self.domain.0.lock();
        let mut node = NODE.lock();
        local.release([self.count]).expect("completion record domain budget underflow");
        node.total.release([self.count]).expect("completion record node budget underflow");
        if !self.platform {
            node.ordinary
                .release([self.count])
                .expect("completion record ordinary budget underflow");
        }
    }
}

/// Ring publication has ended these records' retention, but their original
/// charges must survive the registry hold. One flush belongs to one captured
/// namespace; promotion can leave both ordinary and platform classes present.
/// Two inline owning charges suffice for accounting. Detached nodes travel in
/// the shared retirement list until post-guard deallocation, before refunds.
#[derive(Default)]
#[must_use = "retain flushed charges until the enclosing registry guard leaves"]
pub(super) struct Releases {
    charges: [Option<Charge>; 2],
    nodes: crate::klib::collections::retirement_list::RetirementList<(
        super::OperationId,
        super::DetachedOp,
    )>,
}

impl Releases {
    pub(super) fn push_node(
        &mut self,
        node: crate::klib::collections::retirement_list::RetiredEntry<(
            super::OperationId,
            super::DetachedOp,
        )>,
    ) {
        self.nodes.push(node);
    }

    pub(super) fn push(&mut self, mut charge: Charge) {
        let slot = &mut self.charges[usize::from(charge.platform)];
        if let Some(retained) = slot {
            assert!(
                Arc::ptr_eq(&retained.domain, &charge.domain),
                "mixed completion refund namespaces"
            );
            retained.count =
                retained.count.checked_add(charge.count).expect("completion refund count overflow");
            // The retained charge owns this exact account, so dropping the
            // redundant Arc cannot deallocate it beneath the registry. No
            // budget lock/refund occurs for this consumed charge.
            charge.count = 0;
        } else {
            *slot = Some(charge);
        }
    }
}
impl Drop for Releases {
    fn drop(&mut self) {
        // The caller has left its local registry hold. Retain both class
        // charges until every delivered node's actual allocation is freed.
        while let Some(node) = self.nodes.pop() {
            node.release();
        }
    }
}
pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used()[0] as usize, node.ordinary.used()[0] as usize)
}

mod tests;

pub(super) fn test_backlog_refunds() {
    tests::run();
}
