# Security audit: prepared memory-object records

Date: 2026-10-24. Reviewed baseline: `43f5132d`.

The global memory-object registry now relinks prepared records. Direct allocation
and copies retain fallible record storage with private backing before authority
publication. Object removal carries the complete detached node in the existing
backing receipt through physical release and actual node disposal before the
original charge returns.

The [tracked evidence](evidence/2026-10-24-security-memory-object-storage.json)
records source/formal/artifact hashes, selected diagnostics and earlier passing
images. This advances SEC-07 R7-2 and C05/C18. The ledger remains
**29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial and production
trust remains disabled.

## Preparation and publication

Previously direct allocation and copy publication inserted a global object
record into an allocating `BTreeMap` after shared authority became live. Object
removal extracted frames/charge and destroyed the record under the memory
registry, without a detached record owner.

`allocate_frames` now retains unused `ObjectStorage` alongside private frames
and their original page/object charge. That budget reservation precedes fallible
node allocation and physical frame allocation. Ordinary allocation rejection
returns the complete unused prefix through the same owner. The global registry
uses the shared `AdmittedMap`; object publication only relinks prepared storage.

Direct allocation claims a checked object identity before authority publication.
Copies claim theirs during preparation and carry it in `PreparedPayload`.
Cancelled preparation leaves gaps; identities never reset or recycle. The
isolated counter control rejects exhaustion before publication, without rewriting
the live global counter. These are internal kernel identities; capability ABI,
wire grammar and admission limits are unchanged.

This removes global object-record allocation after authority mutation. Target
memory-capability maps and namespace creation, read-loan bookkeeping and mapping
records remain allocating boundaries. Memory publication is not yet fully
fallible or wholly allocation-free. Factories and ordinary rollback inherit
existing caller contexts; no new unlocked interval or root mechanism is added.

## Complete backing receipt

Destruction detaches the complete object node into `ChargedFrames`, carried by
`RetiredMemory` or existing final-pin cleanup. The node retains the object, frame
vector, original backing charge and remaining metadata. Physical release and
node disposal leave the local memory registry. A successful release disposes
payload and node before dropping that original charge.

Physical release retains sixteen-frame batches. Attempted frame identities are
consumed before each allocator call, preserving the existing no-retry policy.
The backing owner records its started/failed fence before the first attempt;
its fields use `ManuallyDrop`. Failure or interruption cannot trigger implicit
metadata destruction or charge refund. Confirmed completion disposes unused
private storage, frame-vector backing or the complete published node, then
refunds and clears the fence. Ordinary rollback retains its existing release
path; these changes do not qualify all outer destructor contexts.

Rejected physical release retains the complete published node and original
charge even when its attempted-frame vector is empty. The receipt supplies no
retry or recovery custody. `RetiredMemory` abandonment retains its whole backing
owner. The existing detached-backing abandonment control now drops under memory
and heap guards after exact root retirement/slot reuse, retaining metadata and
the old generation's charge without touching the successor.

No new registry, owner family, root/pin, timeout or physical cleanup proof is
introduced. Existing mapping, loan, DMA/copy pin and quiescence requirements stay
in force. The [cleanup rows](../../reference/cleanup-recovery.md),
[object contract](../../reference/memory-object-storage.md),
[retirement contract](../../reference/memory-object-retirement.md),
[lock contract](../../reference/locking.md) and Markdown/LaTeX memory chapter
record this scope. Userspace code and runtime ABI are unchanged.

## Verification

Serialized native controls exercise:

- **128 charged node-preparation rejections**, after original page/object
  admission and before physical allocation. Heap, physical allocator and memory
  registry are available at the factory boundary; charge/frame baselines return.
- **64 copied-prefix rejections**, with a real source pin and unpublished
  destination reservation. Ordinary rejection restores shared authority counts,
  backing charges and frame counts; the source remains writable afterward.
- **64 heap-held object relinks/detaches** using the actual object type and
  prepared node. Original backing admission remains live after detachment;
  explicit physical release returns it only after node disposal. IRQ state is
  preserved. Shared collection tests separately verify actual node free before
  charge release.
- A physical-release adapter deliberately rejects one frame without freeing it.
  The node and original charge remain retained; another cleanup call invokes no
  release callback. Dropping the terminal backing owner under memory/heap guards
  releases neither metadata nor frame. Confirmed root retirement and exact ASID
  reuse leave the old charge intact and the successor account empty.
- The expected `WARNING: failed to free memory-object frames; charge quarantined`
  belongs to that deliberate control. It is not a guest failure or a reproduced
  hardware allocator fault. The added terminal footprint is **one data page,
  one complete object node and its original page/object charge**, with no later
  re-adoption or release.
- Existing guarded detached-backing abandonment, mapping/shootdown failures,
  failed scratch completion, final DMA/copy pins, 257-member mixed authority,
  transfer rollback, hidden delivery, loan cancellation, returned resources and
  scheduled endpoint/namespace cleanup remain enabled.

Stage errors do not exhaust the global allocator. The local identity-exhaustion
control proves checked rejection, not exhaustion of the live global registry.
These serialized controls do not cover arbitrary interruptions or schedules.
Earlier intentional backing retentions now also keep their object/preparation
nodes; no budget reset conceals them.

Final native runs pass on both architectures: **AArch64 20 passed / 0 failed /
0 timed out**, and **x86_64 18 / 0 / 0**. Each completes all **14 HTTP checks**
and both security-service generations. Healthy probe times are 0.346–0.356 s
on AArch64 and 0.325–0.382 s on x86_64; trickle checks take 30.12 s and
30.21 s respectively. Cancellation pressure sends 19,912 and 18,348 requests.

Host retry diagnostics remain separate from guest results. Each run records
four connection-reset attempts; x86_64 also records an initial metrics-probe
`TimeoutError`. Retries subsequently pass without widening timeout or count
limits. These observations do not establish a cause for earlier intermittent
failures.

Both earlier first images also passed. Final review strengthened the backing
owner to fence release before allocator entry and make backing fields inert on
interruption. Both architectures were rerun for that final source. Earlier
passing images retain separate source/artifact hashes and results; they are not
the final-source validation claim. No native failure occurred in this batch.

All **16 shared collection host tests** pass. Strict base-kernel Clippy passes on
both architectures. Workspace formatting, whitespace, shell syntax and local
reference checks pass. Native assembly-section checks remain enabled. Bundled
services are reused and the tracked manual PDF is preserved; runtime sources
are unchanged after final native builds.

## Formal qualification

All **27 focused cases** pass: four safe configurations and 23 expected
counterexamples. Required safe actions execute and unsafe cases violate their
named invariants through the intended actions.

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| `CharlotteIpcPublication` | 7,634,722 / 36,122,253 | 42 |
| `CharlotteIpcTransaction` | 26 / 71 | 11 |
| `CharlotteRootRetirement` | 2,978 / 3,733 | 33 |
| `CharlottePrincipalBacking` | 72,344 / 290,961 | 14 |

No model/configuration changed. Existing authority/delivery, rollback,
non-terminal timeout, exact-root, physical-once and original-principal charge
projections remain. Object-node allocation/deallocation, the concrete start
fence, implicit field destructors, identity gaps and allocator/registry contexts
are native/host qualification. Root physical-once is a policy projection, not an
executable refinement of this backing owner. The prior full **190-case** run
remains separate; all **235 inputs** are unchanged. No allocator/IRQ or sustained
pressure fairness proof is implied.

## Remaining work

Memory namespace/capability, borrower and mapping maps still allocate, including
post-authority transfer updates. Ordinary private backing rollback and broader
caller/destructor contexts, metadata-byte/principal accounting and allocator
overhead remain open. Existing page/object limits are unchanged.

Global object lookup, sorted insertion and cursor traversal are now linear.
Count bounds do not qualify worst-case hold time, contention or sustained
management/teardown progress. Earlier queued-loan mapping panics, root-close
timeouts and initial HTTP probe failures retain their own checkpoints and causal
limitations; these passing runs do not retrospectively explain them.
