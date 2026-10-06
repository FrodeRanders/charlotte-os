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
sponsorship, fairness guarantee or reserved platform subpool. Stack/kernel-heap
data has separate [stack admission](stack-admission.md); kernel-heap data and
IOMMU tables retain unresolved admission needs.

## Lifetime and rollback

For private tables, `PreparingTable` reserves against the exact account before allocating backing
and holds its exclusive Rust borrow through initialization, publication or
rollback. Production mapping retains the original address-space table guard
and generation. Preparation does not resolve a reusable ASID, allocate an
account block, or build a per-frame admission ledger. Borrowed current-root
snapshots cannot allocate private branches; shared higher-half preparation
continues to use its architecture-derived scope.

Allocation failure refunds the unused reservation. A rejected unpublished
physical release retains its original domain and node charge. Retention is
armed before invoking the deallocator, whose ownership is consumed before the
call; interruption cannot trigger a retry or premature refund. Unconfirmed
publication retains backing and a nonrefundable provisional count.

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

Two additional rejected/abandoned provisional fixtures intentionally retain
two physical frames and two table charges. They simulate rejection and
interrupted-owner state; they do not perform real panic unwinding or claim
hardware-race reproduction. QEMU results live in the
[audit record](../reports/audits/2026-10-06-security-table-admission.md).

Shared fixtures reject admission before an allocator callback, refund unused
and successfully released preparation, retain a real sparse higher-half prefix,
reject fresh branches repeatedly, reuse cached branches sixteen times at the
ceiling, and complete a mapping after pressure ends. Four user-root creation/
destruction rounds verify shared alias visibility and unchanged shared charges.
Two additional rejected/abandoned shared preparations retain two physical frames
and charges. Their interruption is simulated. Published empty fixture tables
remain owned and reusable, with exact physical/count deltas; they are not
quarantined. See the
[shared-table audit record](../reports/audits/2026-10-06-security-kernel-table-admission.md).

This closes the renewed audit's private sparse-table admission path. SEC-07
remains partial for inherited kernel tables/stacks, IOMMU tables, kernel heap
and general metadata/callback admission. These pools are not a complete
physical-memory ledger, live tree compactor, IOMMU-table budget, NUMA policy or worst-case
latency guarantee. Empty private tables remain linked until quiescent teardown.
