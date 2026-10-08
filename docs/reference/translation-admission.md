# Translation-table admission

Each owning user `AddressSpace` embeds a separate translation `Account`.
Admission counts actual 4 KiB roots and intermediate tables, independently of
data backing, heap, image or memory-object limits.

| Scope | Ceiling |
| --- | --- |
| One owning root lifetime | 1,024 table frames (4 MiB). |
| Node private-table pool | One sixteenth of usable RAM, rounded down to pages (minimum one). |
| Ordinary domains | Three quarters of that node pool. |

The remaining quarter is shared trusted-platform headroom, not a per-service
entitlement. There is no syscall, artifact-role, manifest or application-name
override. The trusted ambient loader classifies translation admission before
x86 root allocation; promotion after namespace publication would be too late
when ordinary root admission is exhausted. Arm classifies before lazy allocation.
Platform private tables may consume the physical progress floor, within their
table pool and domain ceiling. Ordinary private tables preserve that floor.
All paths remain subject to real allocator exhaustion.

## Shared runtime kernel tables

Fresh shared higher-half tables have a separate node pool of one sixty-fourth
of usable RAM, rounded down to pages (minimum one). `PreparingTable` retains
an owning shared charge, reserved before physical allocation. The short pool
guard is released before allocation, zeroing, publication or physical rollback.
The architecture derives scope from validated mapping context; applications
cannot request the shared policy. Shared requests can use the physical progress
floor, subject to their hard pool limit and real allocator exhaustion.

Successful publication retains the charge permanently for the kernel lifetime.
Copying a shared link into another root does not charge again; destroying that
root does not refund it. Empty branches and linked partial construction remain
charged and reusable at the ceiling. Only an unused reservation or confirmed
physical release of unpublished backing permits refund. Failed release,
interruption and abandonment retain backing and admission; Drop never refunds
an uncertain charge or retries a consumed release.

Bootloader-inherited tables predate `PreparingTable` and are outside this pool.
The policy counts fresh runtime table frames once; it is not a complete boot
table census. There is no shared-table compaction/reclamation path, per-domain
sponsorship, fairness guarantee or reserved platform subpool. Runtime stack data
has separate [stack admission](stack-admission.md).
IOMMU backing has [hardware-table admission](iommu-table-admission.md);
kernel-heap data and general metadata retain unresolved admission needs.

## Lifetime and rollback

For private tables, `PreparingTable` reserves against the exact account before allocating backing
and holds its exclusive Rust borrow through initialization, publication or
rollback. Production mapping retains the original address-space table guard
and generation. Preparation does not resolve a reusable ASID, allocate an
account block, or build a per-frame admission ledger. Borrowed current-root
snapshots cannot allocate private branches; shared higher-half preparation
continues to use its architecture-derived scope.

Ordinary allocation failure explicitly refunds the unused reservation. Ordinary
unused preparation uses consuming `cancel_unpublished`, which reports physical
release rejection. The Arm lazy-root hardware-tag rejection explicitly cancels
its unpublished table before returning; it does not rely on Drop. A rejected unpublished
physical release retains its original domain and node charge. Retention is
armed before invoking the deallocator, whose ownership is consumed before the
call; interruption cannot trigger a retry or premature refund. Unconfirmed
publication retains backing and a nonrefundable provisional count.

`PreparingTable::drop` retains backing and original admission without allocator,
accounting-pool or table locks, callbacks or logging. Private quarantine mutates
only the exclusively borrowed account. Shared charge Drop records quarantine
atomically, leaving the original pool reservation consumed. Reservation-only
abandonment also consumes admission, so retained charges can exceed physical
frames. Completed root destruction excludes those private charges; shared
charges survive later successful preparation. Ordinary rollback still holds the
captured table/account context; this change does not move physical rollback
outside that serialization. See [cleanup/recovery C17 and G1/G2](cleanup-recovery.md).

The raw `PreparingUserFrame` fallback likewise retains backing without cleanup.
Its consuming release is explicit and requires the adapter/fixture to establish
unpublished backing or completed detachment/quiescence. It is not an adoption
API or a substitute for an enclosing table, heap/image or stack transaction.

Linked partial trees and empty branches remain charged after mapping failure
or unmap. Cached branches can be reused at the exact ceiling. A failed new
branch is a fallible mapping error; existing mapping/loader/fault interfaces
keep their current error policy. Root teardown retires translation admission
alongside heap/image admission, and only a completely successful physical walk
authorizes aggregate refund. Any table/root/data release failure conservatively
retains the whole table charge. Successful teardown excludes prior quarantined
provisional pages from its refund. Promotion cannot reclassify quarantine.
Failed final invalidation and abandoned roots retain their existing account
owner; no charge is returned at logical retirement or to a successor ASID.

## Verification and remaining scope

Real architecture walkers exercise a five-table ceiling, a retained sparse
prefix, repeated rejection, sixteen cached unmap/remap rounds, retry after
raising a kernel-only test limit and independent roots sharing one data page.
An admission-pressure adapter exhausts ordinary table admission while keeping
real counters and physical memory intact; a trusted platform root/mapping
succeeds, then ordinary creation/mapping recovers after pressure ends. Existing
prefix/alias fixtures check exact table counts and successful teardown refunds.
Root-release failure fixtures now also check whole-account table retention.
The public memory-object path additionally checks repeated sparse-map rejection,
pin/lease completion, cached remapping and ordinary capability/root close.

Four private and four shared provisional fixtures cover physical rejection,
interrupted publication, unpublished abandonment and reservation-only
abandonment. Drop runs while both address-space guards, the physical allocator
and both original table admission pools are held. Each category retains three
physical frames and four charges; private charges survive original-root
teardown and shared charges survive explicit successful cancellation. Two raw
frame probes retain two more frames through guarded abandonment and terminal
release rejection. Interrupted states are simulated, not panic unwinding or
hardware-race reproduction. Arm's tag-rejection fixture verifies explicit
backing and original-account refund.

Shared-tree fixtures still retain a real sparse higher-half prefix, reject fresh
branches repeatedly, reuse cached branches sixteen times at the ceiling, and
complete a mapping after pressure ends. Four user-root creation/destruction
rounds verify shared alias visibility and unchanged shared charges. Published
empty fixture tables remain owned and reusable; they are not quarantined.
See the [table abandonment report](../reports/audits/2026-10-09-security-table-abandonment.md)
for current QEMU evidence, and the historical
[private](../reports/audits/2026-10-06-security-table-admission.md) and
[shared](../reports/audits/2026-10-06-security-kernel-table-admission.md) records
for original admission coverage.

This closes the renewed audit's private sparse-table admission path. SEC-07
remains partial for inherited kernel tables/stacks, kernel heap
and general metadata/callback admission. IOMMU tables use their own
[admission owner](iommu-table-admission.md). These pools are not a complete
physical-memory ledger, live tree compactor, NUMA policy or worst-case
latency guarantee. Empty private tables remain linked until quiescent teardown.
