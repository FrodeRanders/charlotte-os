//! Independent mailbox record and word-ring backing admission. Namespace/account
//! metadata and the complete capability table remain outside these ceilings.

use alloc::sync::Arc;

use charlotte_lifecycle::resources::{
    CountBudget,
    VectorBudget,
};

use crate::cpu::multiprocessor::spin::mutex::Mutex;

pub(crate) const DOMAIN_LIMIT: usize = 512;
pub(crate) const NODE_LIMIT: usize = 8_192;
pub(crate) const ORDINARY_LIMIT: usize = 6_144;
// Dimensions: prepared/live queue sets and requested word-ring backing bytes.
// Namespace nodes/account control blocks remain separate metadata admission.
pub(crate) const QUEUE_DOMAIN_LIMIT: [u64; 2] = [2, 1024 * 1024];
pub(crate) const QUEUE_NODE_LIMIT: [u64; 2] = [1024, 8 * 1024 * 1024];
pub(crate) const QUEUE_ORDINARY_LIMIT: [u64; 2] = [768, 6 * 1024 * 1024];

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
    queues: VectorBudget<2>,
    retired: bool,
}

#[derive(Debug)]
pub(crate) struct DomainBudget(Mutex<DomainState>);

impl DomainBudget {
    pub(crate) fn try_new() -> Result<Arc<Self>, Error> {
        Arc::try_new(Self(Mutex::new(DomainState {
            count: CountBudget::new(DOMAIN_LIMIT),
            queues: VectorBudget::new(QUEUE_DOMAIN_LIMIT),
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

    pub(crate) fn queue_used(&self) -> [u64; 2] {
        self.0.lock().queues.used()
    }
}

struct NodeBudget {
    total: CountBudget,
    ordinary: CountBudget,
    queue_total: VectorBudget<2>,
    queue_ordinary: VectorBudget<2>,
}

impl NodeBudget {
    const fn new() -> Self {
        Self {
            total: CountBudget::new(NODE_LIMIT),
            ordinary: CountBudget::new(ORDINARY_LIMIT),
            queue_total: VectorBudget::new(QUEUE_NODE_LIMIT),
            queue_ordinary: VectorBudget::new(QUEUE_ORDINARY_LIMIT),
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

    fn reserve_queue(&mut self, platform: bool, amount: [u64; 2]) -> Result<(), Error> {
        self.queue_total.reserve(amount).map_err(|_| Error::ResourceLimit)?;
        if !platform && self.queue_ordinary.reserve(amount).is_err() {
            self.queue_total.release(amount).unwrap();
            return Err(Error::ResourceLimit);
        }
        Ok(())
    }

    fn release_queue(&mut self, platform: bool, amount: [u64; 2]) {
        self.queue_total.release(amount).expect("mailbox queue total underflow");
        if !platform {
            self.queue_ordinary.release(amount).expect("mailbox queue ordinary underflow");
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

#[must_use]
pub(crate) struct QueueCharge {
    domain: Arc<DomainBudget>,
    platform: bool,
    amount: [u64; 2],
}
pub(crate) fn reserve_queue(
    domain: &Arc<DomainBudget>,
    platform: bool,
    bytes: usize,
) -> Result<QueueCharge, Error> {
    let amount = [1, u64::try_from(bytes).map_err(|_| Error::ResourceLimit)?];
    let mut local = domain.0.lock();
    if local.retired {
        return Err(Error::Retired);
    }
    local.queues.reserve(amount).map_err(|_| Error::ResourceLimit)?;
    if let Err(error) = NODE.lock().reserve_queue(platform, amount) {
        local.queues.release(amount).unwrap();
        return Err(error);
    }
    Ok(QueueCharge {
        domain: domain.clone(),
        platform,
        amount,
    })
}
impl Drop for QueueCharge {
    fn drop(&mut self) {
        let mut local = self.domain.0.lock();
        local.queues.release(self.amount).expect("mailbox queue domain underflow");
        NODE.lock().release_queue(self.platform, self.amount);
    }
}
pub(crate) fn queue_node_used() -> ([u64; 2], [u64; 2]) {
    let node = NODE.lock();
    (node.queue_total.used(), node.queue_ordinary.used())
}

pub(crate) fn test_queue_admission() {
    // Isolated production counters distinguish byte and set ceilings without
    // allocating megabytes or exhausting the live shared node pool.
    let mut node = NodeBudget::new();
    node.reserve_queue(false, [1, QUEUE_ORDINARY_LIMIT[1]]).unwrap();
    assert_eq!(node.reserve_queue(false, [1, 1]), Err(Error::ResourceLimit));
    assert_eq!(node.queue_total.used(), [1, QUEUE_ORDINARY_LIMIT[1]]);
    node.reserve_queue(true, [1, QUEUE_NODE_LIMIT[1] - QUEUE_ORDINARY_LIMIT[1]]).unwrap();
    assert_eq!(node.reserve_queue(true, [1, 1]), Err(Error::ResourceLimit));
    node.release_queue(false, [1, QUEUE_ORDINARY_LIMIT[1]]);
    node.release_queue(true, [1, QUEUE_NODE_LIMIT[1] - QUEUE_ORDINARY_LIMIT[1]]);
    assert_eq!(node.queue_total.used(), [0, 0]);
    assert_eq!(node.queue_ordinary.used(), [0, 0]);
    for _ in 0..QUEUE_ORDINARY_LIMIT[0] {
        node.reserve_queue(false, [1, 0]).unwrap();
    }
    assert_eq!(node.reserve_queue(false, [1, 0]), Err(Error::ResourceLimit));
    for _ in QUEUE_ORDINARY_LIMIT[0]..QUEUE_NODE_LIMIT[0] {
        node.reserve_queue(true, [1, 0]).unwrap();
    }
    assert_eq!(node.reserve_queue(true, [1, 0]), Err(Error::ResourceLimit));
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
