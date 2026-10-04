# Memory-object admission budgets

Memory-object allocation has three admission checks: a per-request size bound,
a generation-scoped sponsorship budget, and node-wide limits. These are hard
checks, independent of telemetry smoothing or placement decisions.

| Limit | Current policy |
| --- | --- |
| One allocation | At most 16,384 pages (64 MiB) |
| One sponsoring generation | At most 16,384 pages, clamped to the node pool; at most 1,024 backing objects |
| Entire node | At most one quarter of usable RAM in backing pages; at most 8,192 backing objects |
| Ordinary-domain share | At most three quarters of each node-pool limit |
| Physical progress reserve | Allocation must leave at least one eighth of usable frames free |

All supported targets use 4 KiB pages. The frame check and allocation occur
under the same allocator lock. Kernel/supervisor-designated platform domains
can use the node pool's remaining quarter but still obey their own limits,
the total pool limits and the physical reserve. Scoped applications cannot
claim that designation through names or manifest entries. The supervisor
assigns it only on the ambient platform-launch path, before the domain runs.

The node policy is derived from usable RAM, not the nominal guest-memory size.
The per-domain ceilings are initial kernel policy. A kernel-only hook can
choose smaller limits; there is no userspace setter or signed-descriptor field
for these budgets yet. They are separate from the application heap window and
its signed stack/thread limits.

## Sponsorship follows the resource lifetime

Ownership determines who may use or destroy an object. Sponsorship determines
whose admission budget paid for it. They are intentionally separate:

| Operation | Charge |
| --- | --- |
| Allocate | Creating generation reserves pages and one backing object before allocating frames. |
| Copy | Copying caller reserves a new object and its pages, even when another domain receives ownership. |
| Move, including read-only launch transfer | Existing charge stays with the sponsor; no additional backing pages exist. |
| Borrow, map, DMA or copy pin | Existing backing charge remains; no duplicate page charge. |
| Failed submission or transactional move rollback | Original charge remains; staged allocation failure releases its reservation. |
| Sponsor exit with receiver-held or pinned frames | Charge remains against the retired generation and node pool until final release. |
| Final close or unpin | Release physical frames first, then return their charge. |

A sender cannot shed its budget simply by filling another service's queue.
Likewise, copying data into a privileged receiver does not charge that receiver
or consume its reserved share. The receiver can allocate its own copy if it
intentionally wants to sponsor retained data. Such a copy is a separate
allocation; there is no zero-copy sponsorship reassignment API yet.

Each charge is a linear kernel owner. Staged frames and copy pins also have
owners, so early failure rolls back without a manual cleanup ladder. Charges
survive mapping shootdown and delayed DMA/copy completion. Failed physical
deallocation quarantines the charge rather than creating quota headroom for
frames that may still be allocated.

Retirement marks a generation closed to new reservations before payload
teardown. Its tombstone stays until the address-space slot disappears. If
transferred resources remain, the generation's accounting entry persists until
their last release. Reused ASIDs start with a separate budget; a late release
cannot debit the replacement generation.

Retirement drains IPC before destroying memory attachments, so an in-flight
vector transfer can still roll back its earlier entries when a later entry is
denied. Mapped-loan revocation under the IPC guard does not re-enter the
lifecycle lock. A transient revoking state rejects new mappings and pins while
the registry is released for unmap/shootdown; failed unmap restores the prior
loan state. See [lock ordering](locking.md#5-lock-ordering-rules).

## Errors and verification

The kernel returns `MemoryObjectError::ResourceLimit` for quota or physical
reserve rejection. Status-bearing memory operations use `RESOURCE_LIMIT`
(16). The existing allocation ABI returns zero on failure, so
`OwnedMemory::allocate` reports `MemoryError::AllocationFailed` without
distinguishing quota from frame or metadata allocation failure. Copy/move IPC
submission also retains its existing failure ABI.

The synchronous memory-object tests exercise small aggregate ceilings,
failed-copy unpinning, failed close, transfer rollback, copy/lend charging,
retired sponsorship and delayed unpin after ASID reuse. A reservation-only
test fills the ordinary page pool without consuming real frames, proves that
platform admission remains available, and reconciles all counters on drop.
Pure checked-counter and physical-reserve decisions run in the host suite.
`--security-test` additionally exercises many small allocations from a real
scoped EL0 application, scalar IPC while allocations are denied, and grant
recovery after the owning allocation batch is dropped.

## Remaining isolation work

This limits memory-object backing storage, not all domain resources. It does
not budget loader/heap/stack/page-table frames, borrowed capabilities,
connections, endpoints, queue reservations, general completion records or all
kernel heap metadata. [Completion-backed timers](completion-timer-budgets.md)
have separate event-admission limits; [scheduler sleeps/watchdogs](scheduler-timer-budgets.md)
now share their node pool with separate generation-owned domain accounts.
Other resource families have their own admission rather than being charged to
the memory-object allowance. Comprehensive loader/heap/stack/page-table and
general metadata accounting still need aggregate admission and fallible
bookkeeping. The reserved share is a pool, not guaranteed capacity
for each essential service; a compromised platform service can consume it.
There is no budget telemetry record in the external observability ABI yet.
These limits partially remediate SEC-07; they do not close the audit.
