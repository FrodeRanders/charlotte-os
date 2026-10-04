//! Owning admission for mailbox capability records. These counts do not cover
//! mailbox queue backing, empty namespaces or the complete capability table.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::CountBudget;

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const DOMAIN_LIMIT: usize = 512;
pub(crate) const NODE_LIMIT: usize = 8_192;
pub(crate) const ORDINARY_LIMIT: usize = 6_144;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    ResourceLimit,
    Retired,
    AllocationFailed,
    IdentityExhausted,
}

#[derive(Debug)]
struct DomainState {
    count: CountBudget,
    retired: bool,
}

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<DomainState>);

impl DomainBudget {
    pub(crate) fn try_new() -> Result<Arc<Self>, Error> {
        Arc::try_new(Self(Mutex::new(DomainState {
            count: CountBudget::new(DOMAIN_LIMIT),
            retired: false,
        })))
        .map_err(|_| Error::AllocationFailed)
    }

    pub(crate) fn used(&self) -> usize {
        self.0.lock().count.used()
    }

    pub(crate) fn retire(&self) {
        self.0.lock().retired = true;
    }
}

struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
}

impl NodeBudget {
    const fn new() -> Self {
        Self {
            total: CountBudget::new(NODE_LIMIT),
            ordinary: CountBudget::new(ORDINARY_LIMIT),
        }
    }

    fn reserve(&mut self, platform: bool) -> Result<(), Error> {
        self.total.reserve().map_err(|_| Error::ResourceLimit)?;
        if !platform && self.ordinary.reserve().is_err() {
            self.total.release().unwrap();
            return Err(Error::ResourceLimit);
        }
        Ok(())
    }

    fn release(&mut self, platform: bool) {
        self.total.release().expect("mailbox node count underflow");
        if !platform {
            self.ordinary.release().expect("mailbox ordinary count underflow");
        }
    }
}

// Constant initialization avoids a first-use LazyLock spin dependency in an
// otherwise preemptible syscall. Reservations themselves use the IRQ-safe lock.
static NODE: Mutex<NodeBudget> = Mutex::new(NodeBudget::new());

#[must_use]
#[derive(Debug)]
pub(crate) struct Charge {
    domain: Arc<DomainBudget>,
    platform: bool,
}

pub(crate) fn reserve(domain: &Arc<DomainBudget>, platform: bool) -> Result<Charge, Error> {
    let mut local = domain.0.lock();
    if local.retired {
        return Err(Error::Retired);
    }
    local.count.reserve().map_err(|_| Error::ResourceLimit)?;
    if let Err(error) = NODE.lock().reserve(platform) {
        local.count.release().unwrap();
        return Err(error);
    }
    Ok(Charge {
        domain: domain.clone(),
        platform,
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut local = self.domain.0.lock();
        local.count.release().expect("mailbox domain count underflow");
        NODE.lock().release(self.platform);
    }
}

pub(crate) fn test_node_admission() {
    // Isolated counters, not thousands of live registry allocations or
    // mutation of the shared budget underneath other booting services.
    let mut node = NodeBudget::new();
    for _ in 0..ORDINARY_LIMIT {
        node.reserve(false).unwrap();
    }
    assert_eq!(node.reserve(false), Err(Error::ResourceLimit));
    assert_eq!(node.total.used(), ORDINARY_LIMIT);
    for _ in ORDINARY_LIMIT..NODE_LIMIT {
        node.reserve(true).unwrap();
    }
    assert_eq!(node.reserve(true), Err(Error::ResourceLimit));
    node.release(false);
    node.reserve(false).unwrap();
    for _ in ORDINARY_LIMIT..NODE_LIMIT {
        node.release(true);
    }
    for _ in 0..ORDINARY_LIMIT {
        node.release(false);
    }
    assert_eq!(node.total.used(), 0);
    assert_eq!(node.ordinary.used(), 0);
}
