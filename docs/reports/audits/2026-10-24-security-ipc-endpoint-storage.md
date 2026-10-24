# Security audit: endpoint storage and queue growth

Date: 2026-10-24. Reviewed baseline: `4054bbfc`.

Endpoint creation now reserves its record and queue counts before preparing the
node, backing and observer lists fallibly outside IPC. Queue growth prepares
outside that guard too, retaining both backings until checked FIFO replacement
and post-guard disposal. Final connection removal carries closed endpoint nodes
through the existing cleanup owners; record admission returns after actual free.

The [tracked evidence](evidence/2026-10-24-security-ipc-endpoint-storage.json)
contains source, formal and artifact hashes with selected execution diagnostics.
This continues SEC-07 R7-2 and C08/C09/C10/C18. The ledger remains
**29 corrected / 6 partial / 3 open**. SEC-07/18 remain partial and production
trust remains disabled.

## Preparation and publication

Previously endpoint creation allocated queue backing, close/readiness lists and
an endpoint `BTreeMap` entry under IPC. Queue resize allocated its replacement
there and dropped the old backing before leaving the guard. Endpoint metadata
had no detached storage owner; final reference removal destroyed it under IPC.

Creation now captures the namespace identity under IPC, leaves before acquiring
its exact-root operation lease, then revalidates before atomically reserving
one endpoint record and all rounded queue slots. Its complete inert owner keeps
the original budget account and reservation classification, both split charges,
node, queue, close/readiness lists and authority preparation. Node and backing
factories run outside IPC and local lifecycle/table guards. The queue owns its
already-admitted charge before allocation starts.

Ordinary failure explicitly disposes the complete prefix after local guards,
root last. Abandonment retains every field without entering the allocator,
budget locks or callbacks. Publication revalidates the captured root and
namespace, then borrows its retained shared-authority reservation. Endpoint
installation relinks prepared admitted storage. Shared authority preparation
still enters its factory under IPC, and the per-capability payload `BTreeMap`
still allocates infallibly after authority publication. IPC publication is not
yet fully fallible.

## Growth and retirement

Growth retains the exact root, endpoint and original budget/classification before
reserving and allocating new backing outside IPC. Revalidation checks the
namespace, capability, endpoint identity, closing state and current backing
capacity. Only then does an allocation-free FIFO move replace the old queue.
Both backings remain charged until explicit post-IPC disposal, root last.

A competing larger resize can make the prepared queue unnecessary; it is then
freed outside IPC. Concurrent close rejects the operation without restoring
removed authority or replaying work. Ordinary preparation failure leaves the
old queue unchanged. Shrink still changes admission policy without reducing
backing or its charge. Caller promotion cannot reclassify an endpoint's
subsequent growth or its claim-source preparation.

The endpoint registry now uses the shared admitted-node machinery. Each node
keeps its original record charge independently of the payload. Final references
held by ordinary, queued or unobserved returned connections detach a closed,
empty, unclaimed endpoint into the existing cancellation, endpoint-close or
serialized cleanup owner. A temporary list reuses those nodes without a teardown
snapshot allocation or a new registry. Explicit payload/node disposal leaves IPC
and precedes refund; fallback retains the complete list. Original accounts
survive retirement and ASID reuse without crediting a successor generation.

Failed loans still retain their claims and prevent terminal cleanup. Readiness,
logical absence, detached metadata and these receipts grant no physical release
or replay authority. Existing root/pin order and notification contracts remain
in force. The [cleanup rows](../../reference/cleanup-recovery.md),
[endpoint policy](../../reference/endpoint-budgets.md),
[locking contract](../../reference/locking.md) and Markdown/LaTeX manual are
updated. Userspace code, protocols and runtime ABI are unchanged.

## Verification

Serialized native controls exercise:

- **384 preparation rejections** across six stages, including real node, backing
  and observer-list prefixes. Every injection observes IPC/lifecycle/table/heap
  availability and an exact-root close rejection of `OperationsInFlight`.
  Explicit ordinary finish restores endpoint/slot and authority baselines.
- **64 endpoint relinks/detaches** with IPC and heap held, using already-prepared
  payloads in a local registry. Post-guard disposal restores both count baselines.
  Retiring the authority namespace after full preparation rejects publication
  and cancels unused storage without publishing an endpoint.
- **96 growth rejections** across three stages, preserving a queued message,
  capacity, high-water depth and the original record/slot charges.
- Actual FIFO replacement while heap is held, followed by inspection proving
  both old and new backing remain charged until explicit post-guard finish.
  The source is promoted before growth; the original ordinary share still pays.
  IRQ state is preserved and the original message arrives unchanged.
- A larger resize during preparation and close before revalidation. The first
  uses retained larger backing and disposes unused private storage; the second
  rejects and cancels only unused preparation. These are controlled interleavings,
  not a sustained concurrent stress run.
- Final-reference cleanup through queued delegated connection cancellation and
  unobserved returned-connection close. Closing the source owner alone leaves
  its record charged; removing the hidden final reference disposes it.
- Guarded abandonment of complete creation, unpublished growth and a detached
  private endpoint node. The deliberate retained footprint is **two roots/leases,
  three endpoint records and twenty queue slots**, plus their complete empty
  observer-list storage. The detached private node has an empty queue with four
  backing slots; confirmed root retirement and exact slot reuse leave its
  original account charged. These controls register no waiter entries and carry
  no loans or hardware authority.

Stage injections return errors around real admitted allocations. They do not
exhaust the global allocator. Existing quota, failed transfer/loan cleanup,
returned memory/connection, source/epoch readiness, scheduled endpoint/namespace
retirement and real scoped-security controls remain enabled.

Both first native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**. Each completes
two scoped-security generations and all **14 host HTTP controls**. Healthy replies
beside stalled peers take **0.283–0.347 s** on ARM and **0.298–0.403 s** on x86.
Trickled deployment bodies reject in **30.17 s** and **30.22 s**; cancellation
traffic retires after **20,080** and **18,200** requests.

Each host capture includes four `ConnectionResetError` attempts around EOF,
abortive-close and idle recovery controls. Their existing bounded retries pass;
failed attempts and adjacent successful controls remain separate in the evidence.
The cause is not isolated. No timeout/counter was widened. Earlier mapping
panics, root-close timeouts and initial HTTP-probe timeouts remain at their
historical checkpoints, without retrospective attribution from these passes.

All **16 shared collection host tests** pass. The existing charged-map test now
also checks allocation-free mutable traversal before its payload/node lifetime
and refund checks. Strict base-kernel Clippy passes on both architectures.
Workspace formatting, whitespace and shell syntax checks pass. Native assembly
section checks remain enabled. Embedded service bundles are reused; the tracked
manual PDF is preserved. Native builds precede only the later host-test-only
extension and documentation/evidence edits; runtime sources are unchanged.

## Formal qualification

All **20 focused cases** pass: three safe configurations and 17 expected
counterexamples. Required safe actions execute and unsafe controls violate their
named invariants through the intended actions.

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| `CharlotteIpcPublication` | 7,634,722 / 36,122,253 | 42 |
| `CharlotteIpcTransaction` | 26 / 71 | 11 |
| `CharlotteRootRetirement` | 2,978 / 3,733 | 33 |

No model/configuration changed. These models retain their shared
authority/delivery, rollback, non-terminal timeout and exact-root projections.
Endpoint allocation, atomic record/slot admission, old-backing lifetime,
post-guard disposal, original classification and linear scans are native/host
qualification, rather than explicit model variables. The preceding full
**190-case** run remains a separate checkpoint; all its **235 inputs** are
unchanged. These results provide no executable refinement, allocator/IRQ or
sustained-pressure fairness proof.

## Remaining work

Per-capability payload insertion and shared-authority factory entry under IPC
remain substantial gaps. Other waiter/vector/control-block factories, weak-only
retention, payload destructors, namespace/account byte/principal admission and
comprehensive heap/allocator overhead remain open. The existing endpoint and
queue count limits are unchanged.

Endpoint and call/token lookup and insertion are linear. Reference checks and
serialized close can repeat those traversals. Count bounds alone do not qualify
worst-case guard/IRQ hold time, contention or sustained management/timer/teardown
progress. Broader caller contexts and the earlier unresolved failures still
need their own evidence. This batch adds neither recovery custody nor permission
to retry uncertain cleanup.
