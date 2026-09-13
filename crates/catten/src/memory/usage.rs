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
    physical::PAddr,
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
    /// Physical frame of the domain's mutable status page, where `catten-rt`
    /// publishes standard heap accounting.
    status_frame: Option<PAddr>,
    /// Heap capacity chosen at load; faults beyond it are domain errors.
    heap_bytes: Option<usize>,
    /// Previous node-wide counter sample taken by this domain. Keeping this
    /// generation-scoped prevents another caller from shortening or otherwise
    /// perturbing the interval used for placement load.
    cpu_sample: Option<(u64, u128)>,
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

/// Highest heap peak observed for one service principal, across address-space
/// generations within this boot. Like the stack table it is deliberately
/// in-memory: a reboot cold-starts at the default capacity.
static PRINCIPAL_HEAP_PEAK: LazyLock<Mutex<BTreeMap<u64, u64>>> =
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
                status_frame: None,
                heap_bytes: None,
                cpu_sample: None,
            },
        ),
    );
    debug_assert!(previous.is_none(), "domain accounting survived ASID teardown");
}

/// Derive this domain's interval CPU occupancy from monotonic node counters.
/// The first query uses the boot-to-now interval; later queries use only time
/// since this same domain's previous query.
pub(crate) fn node_cpu_load_permille(
    asid: AddressSpaceId,
    now_ticks: u64,
    busy_ticks: u128,
    logical_processors: u64,
) -> u64 {
    let previous = DOMAIN_USAGE
        .lock()
        .get_mut(&asid)
        .and_then(|(_, usage)| usage.cpu_sample.replace((now_ticks, busy_ticks)));
    let (elapsed, busy) = previous
        .filter(|(previous_ticks, previous_busy)| {
            now_ticks > *previous_ticks && busy_ticks >= *previous_busy
        })
        .map_or((u128::from(now_ticks), busy_ticks), |(previous_ticks, previous_busy)| {
            (u128::from(now_ticks - previous_ticks), busy_ticks - previous_busy)
        });
    if elapsed == 0 {
        return 0;
    }
    busy.saturating_mul(1000)
        .checked_div(elapsed.saturating_mul(u128::from(logical_processors.max(1))))
        .unwrap_or(0)
        .min(1000) as u64
}

/// Record the domain's status frame once the loader maps it.
pub(crate) fn register_status_frame(asid: AddressSpaceId, frame: PAddr) {
    if let Some((_, usage)) = DOMAIN_USAGE.lock().get_mut(&asid) {
        usage.status_frame = Some(frame);
    }
}

/// Record the heap capacity the loader chose for this domain.
pub(crate) fn register_heap_capacity(asid: AddressSpaceId, bytes: usize) {
    if let Some((_, usage)) = DOMAIN_USAGE.lock().get_mut(&asid) {
        usage.heap_bytes = Some(bytes);
    }
}

/// Heap capacity chosen for `asid`, if the domain registered one.
pub(crate) fn domain_heap_capacity(asid: AddressSpaceId) -> Option<usize> {
    DOMAIN_USAGE.lock().get(&asid)?.1.heap_bytes
}

/// Remember a retired generation's heap peak for its principal.
pub(crate) fn remember_principal_heap_peak(principal: u64, peak_bytes: u64) {
    if peak_bytes == 0 {
        return;
    }
    let mut table = PRINCIPAL_HEAP_PEAK.lock();
    let entry = table.entry(principal).or_insert(0);
    *entry = (*entry).max(peak_bytes);
}

/// Highest heap peak recorded for `principal`, or zero when the principal has
/// not run yet in this boot.
pub(crate) fn principal_heap_peak(principal: u64) -> u64 {
    PRINCIPAL_HEAP_PEAK.lock().get(&principal).copied().unwrap_or(0)
}

/// One domain's published heap record.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HeapStatus {
    pub capacity_bytes: u64,
    pub allocated_bytes: u64,
    pub peak_bytes: u64,
    /// Cumulative successful allocations and bytes, for rate derivation.
    pub allocations: u64,
    pub total_allocated_bytes: u64,
    /// Cumulative arena-lock spin iterations; nonzero means shard contention.
    pub lock_spins: u64,
}

/// Read the domain's published heap accounting, or `None` when the status page
/// does not carry a valid record.
pub(crate) fn domain_heap_status(asid: AddressSpaceId) -> Option<HeapStatus> {
    let frame = {
        let table = DOMAIN_USAGE.lock();
        table.get(&asid)?.1.status_frame?
    };
    use charlotte_launch::heap_status;
    let base: *const u8 = frame.into();
    let read = |offset: usize| unsafe { (base.add(offset) as *const u64).read_volatile() };
    if read(heap_status::MAGIC_OFFSET) != heap_status::MAGIC
        || read(heap_status::VERSION_OFFSET) != heap_status::VERSION
    {
        return None;
    }
    Some(HeapStatus {
        capacity_bytes: read(heap_status::CAPACITY_OFFSET),
        allocated_bytes: read(heap_status::ALLOCATED_OFFSET),
        peak_bytes: read(heap_status::PEAK_OFFSET),
        allocations: read(heap_status::ALLOCATIONS_OFFSET),
        total_allocated_bytes: read(heap_status::TOTAL_ALLOCATED_OFFSET),
        lock_spins: read(heap_status::LOCK_SPINS_OFFSET),
    })
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
