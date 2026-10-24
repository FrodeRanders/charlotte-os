# Security audit: prepared memory mapping storage

Date: 2026-10-25. Reviewed baseline: `93469ac7`.

Mapping records and frame-snapshot capacity now prepare fallibly outside the
memory registry. Captured object/size and all admission predicates revalidate
before record publication and the existing pin. Detached mapping nodes remain
in the existing pin or whole-object receipt through maintenance and cleanup;
uncertain completion/abandonment retains complete nodes and exact peer leases.

This advances SEC-07 R7-2 and C05/C06/C18. The ledger remains **29 corrected /
6 partial / 3 open**; SEC-07/18 remain partial and production trust disabled.
The [tracked evidence](evidence/2026-10-25-security-memory-mapping-storage.json)
records source/model/artifact hashes, selected diagnostics and results. Earlier
reports preserve their own checkpoints and unresolved failure limitations.

## Fallible preparation and publication

Previously `map_locked_with_cleanup` reserved its frame vector and inserted a
`BTreeMap` mapping record under `MEMORY_OBJECTS`. Vector rejection was checked;
record insertion could allocate infallibly. Both factories now leave that guard.
Preflight keeps the existing capability/right/range/loan/pin/already-mapped checks.
One private preparation owns an empty admitted node and reserved snapshot capacity.
It captures no frame identity or data borrow and arms no backing pin during the
factory interval; the existing caller retains its exact root.

Final validation rechecks the captured object and size and repeats every admission
predicate. A removed source or competing mapping rejects unused preparation
without table mutation. Snapshot fill then uses reserved capacity; node relinking
and the existing retirement pin publish under one memory hold. Early rejection
still has no pin, skips invalidation and returns unused scratch through the same
completion path. Node/snapshot rejection disposes after the local guard.

The page-table walk and failed-prefix rollback keep their prior serialization.
This change adds no unlocked table-walk interval, replacement root, hardware
readiness contract, timeout, capacity change or replay permission. Snapshot
storage drops after local memory/table guards. Public ABI and wire formats are
unchanged. Mapping lookup/sorted insertion are now linear.

## Complete mapping owners

Successful leaf rollback carries the original detached node in the existing
`MappingRetirementPin`, including its exact published-prefix count. Cached
access has not yet been ruled out merely because leaves were removed. Confirmed
invalidation and required scratch completion permit explicit node disposal,
then pin decrement and any backing release. Maintenance failure or abandonment
retains the node, backing fence and original charge without a retry owner.
Failed leaf cleanup keeps the record live in the registry instead.

Explicit unmap now keeps its live record through leaf cleanup, invalidation and
scratch completion. It detaches the node into that same pin only after those
proofs. Confirmed loan revocation carries its detached mapping in its existing
pin too; unsuccessful revocation retains the original live record/fence.

Whole-object/namespace/serialized-fixture retirement moves complete admitted
nodes into `RetiredObjectMappings`, including each exact peer lease. Abandonment
no longer destroys mapping-node storage under unknown guards. Physical success
permits explicit node disposal; the captured peer lease remains owned through
actual node free and completes afterward. Failed physical release retains the
receipt's nodes/peer leases with its existing terminal backing policy. Ordinary
preparation cancellation still returns only unstarted interests and pins.

No new root, pin, registry, owner family, separately movable metadata ticket,
recovery custody or physical proof is introduced. Original roots, scratch,
backing/capability charges and uncertain-work fences retain their existing
policies. Containing IPC/IRQ/destructor contexts remain separate qualification.

## Verification

Serialized native controls exercise:

- **192 preparation-prefix rejections**: 64 before node storage, 64 after the
  node and 64 after snapshot capacity. Memory, table, heap and frame allocator
  are available at the factory boundary. The source remains writable and has
  no retirement pin; no invalidation runs. Scratch is immediately reusable and
  original frame/backing/authority counts return. These are stage-adapter
  rejections, not global allocator exhaustion.
- **Source-close revalidation** closes a real object after private preparation
  and before final admission. The unpublished node/vector dispose and the call
  rejects without a pin or new mapping; original backing charges return.
- **64 heap-held metadata publications/detachments** use the actual mapping
  node, preallocated 35-frame snapshot and original object. Snapshot fill and
  node relinking need no heap work under memory serialization. No hardware leaf
  is installed by this isolated metadata control; explicit disposal consumes
  its private zero-prefix node/pin outside guards. IRQ state is preserved.
- **32 real rejected-prefix rollbacks** collide with a foreign leaf. The node
  retains its exact one-page published prefix and original pin until confirmed
  maintenance, then normal cleanup returns the fence. The foreign leaf remains
  owned by its separate object.
- **One guarded uncertain rollback** performs that real prefix rollback, then
  rejects confirmation with a per-call adapter (`Busy` on x86, missing-root cause
  on ARM). `finish_mapping` drops its pin while memory and heap guards are held;
  it must retain the complete detached record without allocator entry. Foreign
  mapping cleanup still succeeds. The source cannot close and root close stays
  Pending. This deliberately retains **two data pages, one mapping node, its
  original object/capability metadata and charges, and its exact closing root**.
  It invokes no physical-release retry and is not a reproduced hardware fault.
- Existing scalar and owned namespace abandonment controls now hold memory/heap
  guards while dropping complete receipts. Their nodes, backing and exact peer
  counts remain retained; no existing quarantine or admission limit is reset.
- Existing actual mapping/invalidation/scratch failures, DMA/copy pins, loan
  revocation, returned resources, hidden delivery, cancellation and scheduled
  endpoint/namespace cleanup remain enabled. These tests do not qualify arbitrary
  interruption or schedules.

Final native runs pass on both architectures: **AArch64 20 passed / 0 failed /
0 pending**, and **x86_64 18 / 0 / 0**. Each completes all **14 HTTP checks**
and both security-service generations. Healthy responses beside stalled peers
take **0.303–0.381 s** on ARM and **0.222–0.382 s** on x86. Trickle checks
reject after **30.14 s / 30.21 s**; cancellation pressure retires after
**19,764 / 18,284 requests**, respectively.

Each final run records four `ConnectionResetError` retry diagnostics; x86 also
records an initial host metrics `TimeoutError`. ARM's initial metrics attempt
passes. These observations remain separate from guest results. Subsequent
attempts pass within existing retry/deadline limits. No limits were widened,
and these observations do not attribute earlier intermittent failures.

All **16 shared collection host tests** pass, including inert node/list
abandonment and explicit payload/node disposal ordering. Strict base-kernel
Clippy passes on both architectures. Workspace formatting, whitespace and local
reference checks pass. Native assembly-section checks remain enabled. Bundled
services are reused, userspace sources are unchanged and the tracked manual PDF
is preserved.

The first two images also passed both native suites and all 14 HTTP checks.
Final review added the guarded uncertain-rollback control above, with its explicit
two-page terminal footprint, and both architectures were rerun for that final
source. Earlier images/results remain separate from final-source qualification.
No native failure occurred in this batch. Historical mapping panics, root-close
timeouts and initial network/probe failures retain their own causal limitations.

## Formal qualification and remaining work

Both focused safe configurations and all **seven expected counterexamples**
pass their harness checks. `CharlotteIpcTransaction` explores **26 distinct /
71 generated states**, depth **11**; `CharlotteRootRetirement` explores
**2,978 / 3,733**, depth **33**. Required safe actions execute; negative cases
violate the named invariant through its intended unsafe action.

No model/configuration changes. As recorded in the earlier mapping-admission
qualification, these models do not implement the object-mapping or scratch
pipeline. Existing loan/rollback, non-terminal timeout and root/physical-once/
original-charge projections remain. Concrete factory placement, captured-source
revalidation, prefix descriptors, node lifetime, peer release and allocator/IRQ
contexts are native/host qualification. Earlier full-suite checkpoints remain
separate; no executable refinement or sustained-pressure fairness proof is implied.

Scratch namespace creation and other kernel factories retain their separate
allocating/guarded contexts. Preparation storage has no independent mapping-byte
or aggregate principal admission. Concurrent unused preparations may overlap;
original object/capability count limits and the existing per-object snapshot bound
do not qualify that peak heap pressure. General metadata bytes, allocator overhead,
private rollback and wider outer/destructor contexts remain open. Linear mapping
lookup/insertion and repeated retirement scans have no worst-case hold-time or
sustained management/timer/teardown progress qualification. No readiness, root
absence or error code supplies new cleanup/retry authority.
