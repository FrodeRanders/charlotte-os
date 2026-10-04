//! Hard admission for memory-object pages and backing-object metadata.
//!
//! The creating generation sponsors the allocation for its entire lifetime.
//! Move, lend and IPC rollback never erase or duplicate that charge. Copies
//! consume a new charge in the copying caller, not an unsuspecting receiver.
//! Retired sponsors remain in the ledger until their last charge is released.
//! No syscall permits a domain to enlarge its budget or claim platform reserve.

use alloc::collections::BTreeMap;

pub use charlotte_lifecycle::resources::Amount;
use charlotte_lifecycle::resources::Budget;

use super::{
    AddressSpaceHandle,
    AddressSpaceId,
    KERNEL_ASID,
    LazyLock,
    Mutex,
    PHYSICAL_FRAME_ALLOCATOR,
};

pub const MAX_DOMAIN_OBJECT_PAGES: u64 = 16_384; // 64 MiB, independent of heap.
pub const MAX_DOMAIN_OBJECTS: u64 = 1_024;
const MAX_NODE_OBJECTS: u64 = 8_192;
type Identity = (AddressSpaceId, usize);

#[derive(Debug)]
struct DomainAccount {
    budget: Budget,
    platform: bool,
    retired: bool,
    closed: bool,
    waiters: crate::klib::observer::WaitSponsor,
}

struct Ledger {
    total: Budget,
    ordinary: Budget,
    domains: BTreeMap<Identity, DomainAccount>,
}

impl Ledger {
    fn new() -> Self {
        // Memory objects can consume at most a quarter of usable RAM. Keep
        // one quarter of that pool for kernel/platform progress. This does
        // not yet reserve loader, heap, page-table, IPC or completion memory.
        let pages = (PHYSICAL_FRAME_ALLOCATOR.lock().usable_bytes() / 4096 / 4).max(1);
        Self {
            total: Budget::new(Amount {
                pages,
                objects: MAX_NODE_OBJECTS,
            }),
            ordinary: Budget::new(Amount {
                pages: pages * 3 / 4,
                objects: MAX_NODE_OBJECTS * 3 / 4,
            }),
            domains: BTreeMap::new(),
        }
    }

    fn default_limit(&self) -> Amount {
        Amount {
            pages: MAX_DOMAIN_OBJECT_PAGES.min(self.total.limit().pages),
            objects: MAX_DOMAIN_OBJECTS,
        }
    }

    fn account(&mut self, identity: Identity) -> &mut DomainAccount {
        let limit = self.default_limit();
        self.domains.entry(identity).or_insert(DomainAccount {
            budget: Budget::new(limit),
            platform: identity.0 == KERNEL_ASID,
            retired: false,
            closed: false,
            waiters: crate::klib::observer::WaitSponsor::new(identity.0 == KERNEL_ASID),
        })
    }
}

static LEDGER: LazyLock<Mutex<Ledger>> = LazyLock::new(|| Mutex::new(Ledger::new()));

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Error {
    StaleDomain,
    Limit,
}

/// Linear kernel-boundary reservation. Its Drop only touches this ledger,
/// never a subsystem or frame allocator, so subsystem -> ledger is acyclic.
#[derive(Debug)]
#[must_use]
pub struct Charge {
    identity: Identity,
    amount: Amount,
}

impl Charge {
    pub fn active(&self) -> bool {
        LEDGER.lock().domains.get(&self.identity).is_some_and(|account| !account.retired)
    }
}

pub(crate) fn accepting(handle: AddressSpaceHandle) -> bool {
    let table = super::ADDRESS_SPACE_TABLE.lock();
    table.generation(handle.id()).ok() == Some(handle.generation())
        && LEDGER
            .lock()
            .domains
            .get(&(handle.id(), handle.generation()))
            .is_none_or(|account| !account.retired)
}

/// Kernel policy shared with other resource families; no caller-supplied
/// name, role or manifest field can claim access to the progress reserve.
pub(crate) fn platform_identity(owner: AddressSpaceId) -> Option<AddressSpaceHandle> {
    let table = super::ADDRESS_SPACE_TABLE.lock();
    let generation = table.generation(owner).ok()?;
    let platform = owner == KERNEL_ASID
        || LEDGER
            .lock()
            .domains
            .get(&(owner, generation))
            .is_some_and(|account| account.platform && !account.retired);
    platform.then_some(AddressSpaceHandle {
        id: owner,
        generation,
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut ledger = LEDGER.lock();
        let account = ledger.domains.get_mut(&self.identity).expect("memory sponsor missing");
        account.budget.release(self.amount).expect("memory sponsor underflow");
        let ordinary = !account.platform;
        let remove = account.closed && account.budget.used() == Amount::default();
        ledger.total.release(self.amount).expect("node memory budget underflow");
        if ordinary {
            ledger.ordinary.release(self.amount).expect("ordinary memory budget underflow");
        }
        if remove {
            ledger.domains.remove(&self.identity);
        }
    }
}

pub fn reserve(owner: AddressSpaceId, amount: Amount) -> Result<Charge, Error> {
    // Hold the table through reservation. Retirement installs a tombstone
    // before payload teardown and removes it only after this slot disappears.
    // No IPC -> lifecycle lock acquisition is introduced here.
    let table = super::ADDRESS_SPACE_TABLE.lock();
    let generation = table.generation(owner).map_err(|_| Error::StaleDomain)?;
    let identity = (owner, generation);
    let mut ledger = LEDGER.lock();
    let account = ledger.account(identity);
    if account.retired {
        return Err(Error::StaleDomain);
    }
    account.budget.reserve(amount).map_err(|_| Error::Limit)?;
    let ordinary = !account.platform;
    if ledger.total.reserve(amount).is_err() {
        ledger.domains.get_mut(&identity).unwrap().budget.release(amount).unwrap();
        return Err(Error::Limit);
    }
    if ordinary && ledger.ordinary.reserve(amount).is_err() {
        ledger.total.release(amount).unwrap();
        ledger.domains.get_mut(&identity).unwrap().budget.release(amount).unwrap();
        return Err(Error::Limit);
    }
    Ok(Charge {
        identity,
        amount,
    })
}

/// Supervisor-only: domains launched with ambient platform authority may
/// use the node's progress pool. Signed scoped applications never take this
/// path, regardless of their logical name or artifact class.
pub(crate) fn mark_platform(handle: AddressSpaceHandle) {
    let table = super::ADDRESS_SPACE_TABLE.lock();
    assert_eq!(table.generation(handle.id()).ok(), Some(handle.generation()));
    let mut ledger = LEDGER.lock();
    let account = ledger.account((handle.id(), handle.generation()));
    assert!(!account.retired);
    account.waiters.mark_platform();
    if !account.platform {
        account.platform = true;
        let used = account.budget.used();
        ledger.ordinary.release(used).expect("platform promotion budget underflow");
    }
}

pub(crate) fn retire(handle: AddressSpaceHandle) {
    let mut ledger = LEDGER.lock();
    let identity = (handle.id(), handle.generation());
    let account = ledger.account(identity);
    account.retired = true;
    account.waiters.retire();
}

/// Capture a generation-owned sponsor at thread construction, not while
/// parking under the master table. Reservation needs only counter locks.
pub(crate) fn waiter_sponsor(asid: AddressSpaceId) -> crate::klib::observer::WaitSponsor {
    let table = super::ADDRESS_SPACE_TABLE.lock();
    let generation = table.generation(asid).expect("waiter sponsor requires live ASID");
    LEDGER.lock().account((asid, generation)).waiters.clone()
}

pub(crate) fn forget(handle: AddressSpaceHandle) {
    let mut ledger = LEDGER.lock();
    let identity = (handle.id(), handle.generation());
    let account = ledger.domains.get_mut(&identity).expect("retirement tombstone missing");
    account.closed = true;
    if account.budget.used() == Amount::default() {
        ledger.domains.remove(&identity);
    }
}

/// Kernel launch policy/testing hook; callers retain the exact handle.
/// It can only choose a limit within the node's default per-domain ceiling.
pub(crate) fn set_limit(handle: AddressSpaceHandle, limit: Amount) -> Result<(), Error> {
    let table = super::ADDRESS_SPACE_TABLE.lock();
    if table.generation(handle.id()).ok() != Some(handle.generation()) {
        return Err(Error::StaleDomain);
    }
    let mut ledger = LEDGER.lock();
    let ceiling = ledger.default_limit();
    if limit.pages > ceiling.pages || limit.objects > ceiling.objects {
        return Err(Error::Limit);
    }
    let account = ledger.account((handle.id(), handle.generation()));
    if account.retired {
        return Err(Error::StaleDomain);
    }
    account.budget.set_limit(limit).map_err(|_| Error::Limit)
}

pub(crate) fn used(handle: AddressSpaceHandle) -> Amount {
    LEDGER
        .lock()
        .domains
        .get(&(handle.id(), handle.generation()))
        .map_or(Amount::default(), |account| account.budget.used())
}

pub(crate) fn node_snapshot() -> (Budget, Budget) {
    let ledger = LEDGER.lock();
    (ledger.total, ledger.ordinary)
}
