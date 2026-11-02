# Memory-object, capability and read-borrower node admission

Memory-object records, capability payload records, namespace nodes and read-borrower nodes share
requested node-layout byte/count admission before allocation. This closes one SEC-07
gap: shared authority counts alone cannot bound unused payload/namespace candidates
or abandoned metadata after namespace authority has been retired.

| Dimension | Ordinary callers | Total |
| --- | --- | --- |
| Requested node-layout bytes | 3 MiB | 4 MiB |
| Prepared, published or retained nodes | 24576 | 32768 |

These are the defaults of `Policy.memory_capability_nodes`, selected through the
central [resource policy](resource-policy.md). They are compile-time adjustable with
`CATTEN_RESOURCE_POLICY`; see [Configuration](../guides/configuration.md). No
boot-time input or live resizing is introduced.

## Admission and ownership

The existing `cap_storage::Preparing` captures trusted classification once against
the exact destination namespace handle, before any of the three node factories.
Names, roles and caller-supplied principals grant no reserve. Promotion and ASID
reuse cannot change a node's original class or eventual refund.

Each node reserves its actual requested layout, including payload, link and typed
charge, plus one count before its fallible allocator. All four layouts share one
pool; the first applicable dimension rejects. Allocation/quota rejection returns
`ResourceLimit`. Failure on a later node explicitly disposes the unused prefix and
then cancels its hidden authority, before source escrow or pin mutation. No payload
capability or data backing is published by that rejected preparation.

Even a destination that already has a namespace prepares a candidate: that candidate
consumes quota until explicit post-guard disposal. Capability/namespace preparation needs two temporary
slots, or three with a read-borrower candidate. Allocation/copy additionally
prepare the object node described below. Established namespaces retain only the new payload
on ordinary success, and also the borrower node on read-loan success; write-loan
borrower state remains inline. Size the pool for overlapping preparations, retained
generations and cleanup uncertainty as well as live authority. Raising the unified
capability count does not automatically raise this independent metadata limit.

The same typed charge follows the node through `Caps`, namespace and borrower
storage, source-transfer retirement, read/write-loan cleanup and complete
object/backing receipts. Publication/detachment only relink storage and enter no
admission counter. Confirmed disposal destroys the payload and frees the node before
its original charge returns. Independent unified authority remains retained until
its own existing disposal boundary.

The capability preparation owner’s partial/full/published fallback retains all
fields without registry, allocator or counter entry. Namespace teardown may retire
hidden authority while abandoned payload/namespace metadata and its original
admission remain retained. Logical exit, disappearance of authority, promotion or
root reuse cannot refund that storage. Uncertain physical cleanup keeps its original
complete owner and backing charges; this metadata admission adds no physical proof
or retry owner.

## Object records and backing sponsorship

The object registry uses the same admitted node type for unused candidates,
published objects and complete detached retirement. `ChargedFrames` captures
trusted metadata class against its exact backing-sponsor generation before
allocation. Direct allocation uses that root; copies keep the existing source
backing sponsorship, separately from the destination capability/namespace class.
Promotion, moves, changing `MemoryObject.owner`, retirement and ASID reuse cannot
reclassify an existing node or redirect its eventual refund.

Object-node admission follows the original page/object reservation and precedes
frame-vector or physical-frame allocation. The charge includes the actual object,
link and 16-byte inline metadata owner. No holder allocation is added. Quota or
allocator rejection returns `ResourceLimit` through the existing complete
preparation cleanup. Direct allocation and copy need three candidate nodes:
capability, namespace and object. Two available slots can admit the capability
prefix but must reject the object before data allocation or publication; the
same prefix then cancels without source mutation or hidden authority remaining.
Moves and write loans still need two candidates, read loans three.

Private backing retains its unused charged object node. Publication relinks that
same node and retirement carries the full object, vector, pins, physical charge
and original metadata admission through the existing `ChargedFrames` receipt.
Physical completion precedes payload and node disposal; actual node deallocation
precedes its metadata refund. The physical sponsorship charge is released after
node disposal. Partial physical failure or abandonment keeps the whole node and
original physical and metadata charges without cleanup or replay. Existing guarded
private and published-owner fixtures also hold the metadata pool lock.

The four layouts share the existing 3/4 MiB byte and 24576/32768 node defaults.
The object layout is 248 bytes on supported 64-bit targets, including the
16-byte charge (128 KiB extra at the 8192-object physical count ceiling). Native
empty-layout controls admit about 12600 ordinary object candidates before byte
exhaustion, above the 6144 ordinary object planning ceiling; this does not promise
that mixed or independent ceilings can be occupied together. Empty object-layout
candidates exercise independent byte exhaustion, and actual
allocation/copy tests exercise the new third-node rejection. Shared capacity now
also bounds object records and makes them compete with capability metadata.
These controls do not require raising numerical defaults; mixed workload planning
may justify a reviewed increase. Frame catalogs share the byte dimension as
described below; allocator bookkeeping, aggregate principal metadata and general
heap admission remain separate.
The uncharged collection convenience factory is now test-only; production sites
use explicit charged construction. This does not establish that every kernel
allocation is admitted.

## Object frame-vector capacity

Private and published objects use `backing::FrameVector` for their physical-address
catalog. It owns the exact-capacity vector and a byte-only reservation from
`memory_capability_nodes`. Buffers contribute zero to the node count: their
containing object node and original page/object reservation already bound their
quantity. This is separate from mapping/copy/DMA snapshot-buffer admission and
from the data pages themselves.

The backing owner passes the same captured generation-qualified class to object
storage and vector preparation. Copies retain source backing sponsorship, and
moves transfer the same vector/charge without reclassification. Admission covers
`pages * size_of(PAddr)` before allocation, checks exact capacity, and precedes
physical allocator entry. A vector allocation error returns `FrameAllocFailed`;
quota/size denial returns `ResourceLimit`. The existing complete preparation
cancels its object node, authority prefix and original page/object reservation
without publication. The operator profile rejects a maximum object catalog that
cannot fit in the ordinary metadata byte budget.

Only bounded push/pop and borrowed slice access are exposed; callers cannot grow
or reserve the vector after physical identities enter it. Its retaining fallback
keeps capacity and original admission, including while empty or after all release
identities have been popped. Empty length is not physical completion. Confirmed
backing release consumes the same catalog, frees its vector and only then refunds
its original byte charge. Partial physical failure retains the whole catalog,
object node, physical charge and terminal fence; no replay or early metadata
refund is permitted.

At the default maximum of 16384 pages, one catalog requests 128 KiB. Its inline
owner adds 24 bytes per object (192 KiB at the 8192-object count ceiling), included
in the 248-byte object-node layout. No extra holder allocation is needed. Shared
3/4 MiB metadata defaults, count defaults and the 22-pool/version-9 snapshot stay
unchanged. Frame catalogs now compete with node layouts for same-class bytes;
real buffer pressure and the native campaigns need no limit increase. Larger
object maxima or measured mixed workloads may warrant co-scaling the central
metadata and snapshot limits. These independent ceilings do not guarantee RAM
fit or sustained service fairness.

## Qualification and remaining scope

The [audit
follow-up](../reports/audits/2026-10-25-security-memory-capability-node-admission.md)
records allocator-adapter rejection for both layouts, actual ordinary/total
exhaustion, trusted reserve, two-node partial rejection, public allocation/
move/copy/loan rejection without persistent source/destination change, detached
lifetime and real cleanup/root reuse at exhaustion. Guarded abandonment retains four
unused nodes and their original charges through real root retirement and successor
reuse.

The [read-borrower
follow-up](../reports/audits/2026-10-26-security-borrower-node-admission.md) adds
the third layout to the same pool. Actual ordinary/total exhaustion leaves two
available slots: write-loan preparation can cancel normally, while read-loan
preparation rejects without source mutation. Trusted reserve admits a real read
loan; mapped revocation at exhaustion frees the exact borrower and capability nodes
with their original charges, including after promotion. A native control restoring
the old uncharged factory fails the third-node assertion. Unused borrower
cancellation still follows the active containing transfer policy; this does not
qualify arbitrary outer destructor/IRQ contexts.

This advances R7-2/R7-3 and C05/C18. It bounds these requested node layouts,
including their inline owning fields; nested authority/account allocations,
allocator overhead and general heap/principal admission remain separate. It
supplies no universal RAM-fit or essential-service fairness guarantee. Sustained
concurrent EL0 pressure and broader outer guard/IRQ/ destructor contexts remain
SEC-07/18 acceptance work.

The [object-node evidence](../reports/audits/evidence/2026-11-02-memory-object-node-admission.json)
records the fourth layout and native outcomes. Fresh four-CPU, 240-second
security campaigns passed ARM 19/19, Intel VT-d 17/17 and AMD-Vi 17/17, with
zero failures or pending tests. The full host suite and strict test/minimal
kernel checks passed. Controls cover 256 injected
allocator rejections across all four layouts, real empty object-node byte
pressure with count headroom, heap-held quota denial, trusted reserve, and exact
recovery. Two-slot public allocation/copy rejection preserves source state,
authority, buffers, physical budgets and frames. A private backing owner captures
ordinary class before promotion, survives root retirement and trusted successor
reuse, then refunds its original charge on confirmed release. Existing complete
backing fixtures retain nodes under physical failure and guarded abandonment.

The [frame-vector evidence](../reports/audits/evidence/2026-11-02-object-frame-vector-admission.json)
identifies production source and native artifacts. Host buffer-free tracing
checks deallocation before refund, popped identities with retained capacity,
constructor rejection, exact-size/bound rejection, trusted headroom and inert
abandonment under the metadata-pool guard. Native controls exhaust ordinary and
total byte admission using real empty buffers without changing node counts, and
inject 64 allocated-vector rejections after object-node preparation but before
physical allocation. Existing partial-frame, copy cancellation, publication,
root-reuse and guarded backing fixtures exercise the same new catalog owner.
Failed physical release retains empty but still allocated/charged capacity.
