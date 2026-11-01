//! Generation-owned admission for scheduler waiter entries. Kernel callback
//! observer implementations remain explicitly outside this budget.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::CountBudget;
use spin::LazyLock;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    klib::charged_allocator::ChargedAllocator,
};

pub(crate) type AccountRef = Arc<DomainBudget, ChargedAllocator<super::account_budget::Charge>>;

pub(crate) const DOMAIN_LIMIT: usize = crate::resource_policy::POLICY.waiters.domain[0] as usize;
pub(crate) const NODE_LIMIT: usize = crate::resource_policy::POLICY.waiters.pool.total[0] as usize;
pub(crate) const ORDINARY_LIMIT: usize =
    crate::resource_policy::POLICY.waiters.pool.ordinary[0] as usize;
pub(crate) const SOURCE_LIMIT: usize = crate::resource_policy::POLICY.local.waiters_per_source;

#[derive(Debug)]
struct Account {
    count: CountBudget,
    platform: bool,
    retired: bool,
}
#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<Account>);
impl DomainBudget {
    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn try_new(platform: bool) -> Result<AccountRef, core::alloc::AllocError> {
        Self::try_new_with_backing_class(platform, platform)
    }

    pub(super) fn try_new_with_backing_class(
        platform: bool,
        backing_platform: bool,
    ) -> Result<AccountRef, core::alloc::AllocError> {
        let charge = super::account_budget::reserve(backing_platform)?;
        let allocator = ChargedAllocator::try_new(charge)?;
        allocator.try_arc(Self(Mutex::new(Account {
            count: CountBudget::new(DOMAIN_LIMIT),
            platform,
            retired: false,
        })))
    }

    #[cfg(any(test, feature = "kernel_tests"))]
    pub(crate) fn used(&self) -> usize {
        self.0.lock().count.used()
    }

    pub(crate) fn list_platform(&self) -> Result<bool, super::registration::RegistrationError> {
        let local = self.0.lock();
        if local.retired {
            return Err(super::registration::RegistrationError::Closed);
        }
        Ok(local.platform)
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
    domain: AccountRef,
    platform: bool,
}
pub(crate) fn reserve(
    domain: &AccountRef,
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
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn node_used() -> (usize, usize) {
    let node = NODE.lock();
    (node.total.used(), node.ordinary.used())
}
