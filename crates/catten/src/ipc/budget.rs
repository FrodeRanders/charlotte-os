//! Endpoint records and preallocated message slots retain their original
//! namespace's sponsorship until their storage is actually released.

use alloc::{
    collections::VecDeque,
    sync::Arc,
};
use core::ops::{
    Deref,
    DerefMut,
};

use charlotte_lifecycle::resources::VectorBudget;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

// Count endpoint records and queue backing slots; payload bytes are outside this budget.
pub(crate) const DOMAIN_LIMIT: [u64; 2] = crate::resource_policy::POLICY.endpoints.domain;
pub(crate) const NODE_LIMIT: [u64; 2] = crate::resource_policy::POLICY.endpoints.pool.total;
pub(crate) const ORDINARY_LIMIT: [u64; 2] = crate::resource_policy::POLICY.endpoints.pool.ordinary;

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
    pub(crate) fn try_new() -> Result<Arc<Self>, core::alloc::AllocError> {
        Arc::try_new(Self(Mutex::new(VectorBudget::new(DOMAIN_LIMIT))))
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

impl Charge {
    pub(crate) fn domain(&self) -> &Arc<DomainBudget> {
        &self.domain
    }

    pub(crate) fn platform(&self) -> bool {
        self.platform
    }

    pub(crate) fn split(&mut self, amount: [u64; 2]) -> Self {
        for (remaining, part) in self.amount.iter_mut().zip(amount) {
            *remaining = remaining.checked_sub(part).expect("endpoint charge split underflow");
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
        local.release(self.amount).expect("endpoint domain budget underflow");
        node.total.release(self.amount).expect("endpoint node budget underflow");
        if !self.platform {
            node.ordinary.release(self.amount).expect("endpoint ordinary budget underflow");
        }
    }
}

pub(crate) fn node_used() -> ([u64; 2], [u64; 2]) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}

/// Field order frees queue backing before returning its admission charge.
#[derive(Debug)]
pub(crate) struct AdmittedQueue<T> {
    storage: VecDeque<T>,
    _charge: Option<Charge>,
}

impl<T> Default for AdmittedQueue<T> {
    fn default() -> Self {
        Self {
            storage: VecDeque::new(),
            _charge: None,
        }
    }
}

impl<T> AdmittedQueue<T> {
    pub(crate) fn required_slots(capacity: usize) -> Result<usize, ()> {
        capacity.checked_next_power_of_two().map(|slots| slots.max(4)).ok_or(())
    }

    /// Install already-admitted backing in its containing owner before allocation.
    pub(crate) fn reserved(charge: Charge) -> Self {
        assert_eq!(charge.amount[0], 0);
        Self {
            storage: VecDeque::new(),
            _charge: Some(charge),
        }
    }

    pub(crate) fn allocate_backing(&mut self, capacity: usize) -> Result<(), ()> {
        let slots = Self::required_slots(capacity)?;
        assert!(self.storage.is_empty() && self.storage.capacity() == 0);
        assert_eq!(self._charge.as_ref().unwrap().amount, [0, slots as u64]);
        self.storage.try_reserve_exact(slots).map_err(|_| ())?;
        // Fail closed if a future allocator grows beyond our reservation.
        if self.storage.capacity() > slots {
            return Err(());
        }
        Ok(())
    }
}

impl<T> Deref for AdmittedQueue<T> {
    type Target = VecDeque<T>;

    fn deref(&self) -> &Self::Target {
        &self.storage
    }
}

impl<T> DerefMut for AdmittedQueue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.storage
    }
}
