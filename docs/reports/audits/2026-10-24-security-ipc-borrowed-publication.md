# Security audit: borrowed authority publication

Date: 2026-10-24. Reviewed baseline: `0fe97d9d`.

Memory/IPC publication now borrows its existing reservation and transfer owners
instead of allocating a temporary authority vector. The capability gate still
validates the whole set before mutation under one hold. A complete prepared
scalar call can now publish authority, install admitted payloads and enqueue
with IPC and heap held.

The [tracked evidence](evidence/2026-10-24-security-ipc-borrowed-publication.json)
contains source/formal/artifact hashes and selected diagnostics. This advances
SEC-07 R7-2 and the existing C18 boundary. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial and production
trust remains disabled.

## Borrowed batches

Previously `commit_transfers_with_delivery` allocated and filled a
`Vec<Publication>` before entering the shared authority gate. A scalar call
needed that allocation for its one additional call reservation, even though
its record/payload storage and queue backing were already prepared.

`PublicationBatch` now exposes reborrowed iterators over existing owners.
Slice/array callers forward their current members; `TransferPublications`
chains the additional-reservation slice with the prepared-transfer slice.
The old temporary vector construction is removed. Auxiliary space stays constant
with batch length; there is no replacement heap buffer, batch-sized stack array,
new dependency or additional owning resource.

The provider contract requires the same destinations, sources and dispositions
on both passes, without membership mutation, allocation, callbacks or subsystem
entry. Current providers only reborrow disjoint fields. Source disposition is a
copied policy enum; reservations, escrow, roots, pins and charges are not copied.
The capability gate retains its single `CAPABILITIES` hold across complete
validation and publication. A retired or invalid member rejects before any
member becomes live. Moves retain their revoked source metadata in escrow;
loans restore their source authority through the existing action.

Memory validation still runs before that gate, under `MEMORY_OBJECTS`. The same
memory hold covers later payload updates; RPC callers also retain IPC. Scalar
calls with no memory transfers need no memory-registry hold. Checked combined
length arithmetic and packed request limits remain unchanged: up to 255 memory
entries plus the call and delegated-connection authorities. There is no ABI,
wire grammar, format or quota change.

## Ownership and limits

The containing operations retain their existing exact roots, pins, private
copied frames, source escrow and original charges. Borrowed batch views own no
cleanup and add no registry, custody, root mechanism or retry receipt. Ordinary
unstarted rejection still cancels through the complete prepared owners after
their local guards leave. Readiness, timeout, logical absence and metadata
removal grant no physical cleanup or replay authority.

Mixed memory payload installation still uses allocating `BTreeMap` updates after
authority publication, including copied-object records, target memory caps and
read-loan bookkeeping. Those updates remain under `MEMORY_OBJECTS` and enclosing
IPC. Removing the authority vector does not make memory publication fully
fallible or its complete commit allocation-free. The mixed test below locks
heap only around authority iteration/publication, then releases it before
payload installation. The scalar test does execute the complete prepared call
commit with heap held.

Direct-grant, receive and returned-connection factories retain their guarded
contexts. Other vector/control/list preparation, implicit destructors, aggregate
byte/principal accounting and wider IRQ/guard contexts also remain open.
Linear admitted-map traversal and repeated close/reference scans still need
hold-time and sustained-pressure qualification.

The [cleanup row](../../reference/cleanup-recovery.md),
[batch contract](../../reference/ipc-publication-batches.md),
[lock contract](../../reference/locking.md),
[API simplification record](../../reference/api-simplification.md) and
Markdown/LaTeX manual reflect this scope. Userspace code and signed service
bundles are unchanged.

## Verification

Serialized native controls exercise:

- An empty publication with heap held, without memory-registry work.
- A **257-member authority batch**: 255 real prepared memory destinations
  (85 moves, 85 copies and 85 read loans) plus two extra reservations. Heap is
  held while the actual borrowed provider enumerates and publishes the set;
  payload installation follows with heap available. Raw extra authorities are
  fixture entries with no IPC payload and are explicitly removed afterward.
- Successful destination authority, committed transfer state and hidden owning
  memory before delivery. After preparation drops and delivery occurs, every
  destination is usable; moves consume sources, copies preserve sources and
  loans retain their normal revocation contract. Explicit close/revocation
  restores original shared-count and memory-budget baselines. IRQ state is
  preserved.
- Two real late-rejection cases: retirement of the last memory destination's
  namespace, or of an additional reservation's namespace. Both run the actual
  gate with heap held. No destination becomes live, no transfer commits and
  admitted counts remain unchanged. Ordinary cancellation restores every source,
  then close returns shared counts and backing charges to baseline.
- **64 complete prepared scalar call commits** with IPC and heap held, followed
  by receive, reply, result observation and close. Enqueued values remain intact;
  original record/capability counts and IRQ state reconcile after each cycle.
- Existing root-retaining preparation, shared quota, copy/move/loan failure,
  returned memory/connection, claim/source readiness, guarded abandonment and
  scheduled endpoint/namespace retirement controls remain enabled.

No new abandoned fixtures, root/pin accounting, quota resets or cleanup authority
are introduced. These are serialized publication/rollback controls, without
arbitrary concurrent schedules or global allocator-exhaustion injection.

Both first native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**. Each completes
two scoped-security generations and all **14 host HTTP controls**. Healthy
responses beside stalled peers take **0.249–0.373 s** on ARM and
**0.263–0.376 s** on x86. Trickled deployment bodies reject in **30.13 s** and
**30.18 s**; cancellation traffic retires after **17,788** and **15,620** requests.

Each host capture records four `ConnectionResetError` attempts around EOF,
abortive-close and idle recovery. X86 also records an initial metrics-request
`TimeoutError`. Existing bounded retries succeed; failed attempts and adjacent
successful controls remain separate. Their causes are not isolated. No timeout
or count ceiling was widened, and these passes do not explain earlier failures.

All **16 shared collection host tests** pass. Strict base-kernel Clippy passes
on both architectures. Workspace formatting, whitespace, shell syntax and local
reference checks pass. Native assembly-section checks stay enabled. Embedded
services are reused, the tracked manual PDF is preserved and runtime sources
are unchanged after native builds; later edits are documentation/evidence.

## Formal qualification

All **20 focused cases** pass: three safe configurations and 17 expected
counterexamples. Required safe actions execute; unsafe controls violate their
named invariants through their intended actions.

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| `CharlotteIpcPublication` | 7,634,722 / 36,122,253 | 42 |
| `CharlotteIpcTransaction` | 26 / 71 | 11 |
| `CharlotteRootRetirement` | 2,978 / 3,733 | 33 |

No model/configuration changed. Existing atomic authority/delivery, rollback,
non-terminal timeout and exact-root projections remain. Iterator membership
stability, actual auxiliary storage, allocator/guard boundaries and 257-member
execution are native qualification. Memory payload-map allocation remains
outside the authority gate's atomic model projection. The prior full
**190-case** checkpoint remains separate; all **235 inputs** are unchanged.
These results provide no executable refinement, allocator/IRQ or fairness proof.
