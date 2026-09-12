//! # Per-domain resource accounting
//!
//! Phase 1 of the adaptive resource policy
//! ([`docs/architecture/adaptive-resource-policy.md`](../../../../docs/architecture/
//! adaptive-resource-policy.md)): observe and record what each protection domain consumes. Nothing
//! in this module changes allocation behavior; it only feeds the inspection paths
//! (thread statistics and the observe service) and future controllers.
//!
//! Counters are keyed by the generation-bearing [`AddressSpaceHandle`] so ASID
//! reuse cannot transfer one domain's accounting to its successor. Missing
//! entries are ignored rather than asserted: teardown races must not panic a
//! kernel merely because a late release arrived after the domain was reaped.

use alloc::{
    collections::BTreeMap,
    vec::Vec,
};

use crate::memory::{
    AddressSpaceHandle,
    AddressSpaceId,
    LazyLock,
    Mutex,
    address_space_handle_is_current,
};

/// One domain's accounted resource usage at snapshot time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DomainUsageSnapshot {
    /// Frames registered to the address space by the loader (ELF segments,
    /// runtime pages, domain heap, and CQ rings). User-stack frames are
    /// accounted separately.
    pub owned_frames: u64,
    /// Stack pages currently reserved by live threads of the domain.
    pub user_stack_pages: u64,
    /// High-water mark of [`Self::user_stack_pages`].
    pub user_stack_pages_high_water: u64,
    /// High-water mark of the pages a single thread actually touched,
    /// sampled from the saved stack pointer on context switches.
    pub stack_pages_used_high_water: u64,
    /// Live threads in the domain.
    pub threads: u64,
    /// High-water mark of [`Self::threads`].
    pub threads_high_water: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct DomainUsage {
    snapshot: DomainUsageSnapshot,
}

type DomainUsageTable = BTreeMap<AddressSpaceId, (AddressSpaceHandle, DomainUsage)>;

static DOMAIN_USAGE: LazyLock<Mutex<DomainUsageTable>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Highest touched stack high-water mark observed for one service principal,
/// across address-space generations within this boot. The table is what lets
/// the launch path size the next generation from the previous one; it is
/// deliberately in-memory and cold-starts at the default policy after reboot.
static PRINCIPAL_STACK_HIGH_WATER: LazyLock<Mutex<BTreeMap<u64, u64>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn with_usage(asid: AddressSpaceId, update: impl FnOnce(&mut DomainUsage)) {
    let mut table = DOMAIN_USAGE.lock();
    let Some((_, usage)) = table.get_mut(&asid) else {
        return;
    };
    update(usage);
}

/// Install a zeroed accounting entry for a freshly registered address space.
pub(crate) fn register_domain(handle: AddressSpaceHandle) {
    let previous = DOMAIN_USAGE.lock().insert(
        handle.id(),
        (
            handle,
            DomainUsage {
                snapshot: DomainUsageSnapshot::default(),
            },
        ),
    );
    debug_assert!(previous.is_none(), "domain accounting survived ASID teardown");
}

/// Drop a domain's accounting when its address space is torn down.
pub(crate) fn unregister_domain(asid: AddressSpaceId) {
    DOMAIN_USAGE.lock().remove(&asid);
}

/// Account one loader-owned frame registered to `asid`.
pub(crate) fn note_owned_frame(asid: AddressSpaceId) {
    with_usage(asid, |usage| {
        usage.snapshot.owned_frames = usage.snapshot.owned_frames.saturating_add(1);
    });
}

/// Account a thread created in `asid` with `stack_pages` reserved stack pages.
pub(crate) fn note_thread_created(asid: AddressSpaceId, stack_pages: usize) {
    with_usage(asid, |usage| {
        let snapshot = &mut usage.snapshot;
        snapshot.threads = snapshot.threads.saturating_add(1);
        snapshot.threads_high_water = snapshot.threads_high_water.max(snapshot.threads);
        snapshot.user_stack_pages = snapshot.user_stack_pages.saturating_add(stack_pages as u64);
        snapshot.user_stack_pages_high_water =
            snapshot.user_stack_pages_high_water.max(snapshot.user_stack_pages);
    });
}

/// Account a thread retired from `asid`, folding in the pages it touched.
pub(crate) fn note_thread_released(
    asid: AddressSpaceId,
    stack_pages: usize,
    stack_pages_used: usize,
) {
    with_usage(asid, |usage| {
        let snapshot = &mut usage.snapshot;
        snapshot.threads = snapshot.threads.saturating_sub(1);
        snapshot.user_stack_pages = snapshot.user_stack_pages.saturating_sub(stack_pages as u64);
        snapshot.stack_pages_used_high_water =
            snapshot.stack_pages_used_high_water.max(stack_pages_used as u64);
    });
}

/// Snapshot one domain's accounting, rejected when its handle is stale.
pub(crate) fn domain_usage(asid: AddressSpaceId) -> Option<DomainUsageSnapshot> {
    let (handle, usage) = {
        let table = DOMAIN_USAGE.lock();
        let (handle, usage) = table.get(&asid)?;
        (*handle, *usage)
    };
    address_space_handle_is_current(handle).then_some(usage.snapshot)
}

/// Snapshot every live domain's accounting.
pub(crate) fn all_domain_usage() -> Vec<(AddressSpaceId, DomainUsageSnapshot)> {
    DOMAIN_USAGE.lock().iter().map(|(asid, (_, usage))| (*asid, usage.snapshot)).collect()
}

/// Remember a retired generation's stack high-water mark for its principal.
pub(crate) fn remember_principal_stack_high_water(principal: u64, pages: u64) {
    if pages == 0 {
        return;
    }
    let mut table = PRINCIPAL_STACK_HIGH_WATER.lock();
    let entry = table.entry(principal).or_insert(0);
    *entry = (*entry).max(pages);
}

/// Highest stack high-water mark recorded for `principal`, or zero when the
/// principal has not run yet in this boot.
pub(crate) fn principal_stack_high_water(principal: u64) -> u64 {
    PRINCIPAL_STACK_HIGH_WATER.lock().get(&principal).copied().unwrap_or(0)
}
