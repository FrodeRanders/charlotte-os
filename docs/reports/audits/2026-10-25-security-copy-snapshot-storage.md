# Security audit: copy-pin snapshot preparation and retention

Date: 2026-10-25. Reviewed baseline: `75ba9b9a`.

Copy-pin frame-vector capacity now prepares fallibly outside `MEMORY_OBJECTS`.
Final source validation precedes allocation-free snapshot fill and pin publication.
Bare preparation/pin abandonment retains snapshot storage without allocator
entry; explicit unpin disposes it before decrementing its original backing fence.

This advances SEC-07 R7-2 and C18, preserving C05/C06's independent cleanup pins.
The ledger remains **29 corrected / 6 partial / 3 open**; SEC-07/18 remain partial
and production trust disabled. The [tracked evidence](evidence/2026-10-25-security-copy-snapshot-storage.json)
records source/model/artifact identities and selected execution diagnostics.

## Pin-free preparation and captured publication

The former `pin_for_copy` grew its snapshot vector under memory serialization.
The new path probes capability, read rights, source ownership, loan/retirement/
DMA fences, writable mappings and checked pin capacity under the same registry.
An owning preparation reserves capacity after that guard leaves. It carries no
frame identity, Rust borrow or source pin through its factory interval.

Final admission checks the captured full namespace handle, source object, size
and every original predicate under one hold. A removed capability, a new writable
mapping or DMA pin, or a reused ASID can reject the unused preparation. Snapshot
fill and pin increment share that hold, with no allocator entry. Object identities
remain nonreused. The exact returned pin retains backing and its original charge;
no later scalar lookup reconstructs a cleanup owner.

Ordinary failed/unused preparation explicitly disposes capacity after memory
unlock. Its inert fields retain partial storage on abandonment. Bare `CopyPin`
fallback now also retains its vector, alongside the existing registry pin that
already retained backing. Explicit `unpin_copy` frees snapshot storage before
pin decrement, outside the local memory guard. Interruption before decrement
retains the original fence; ordinary completion still permits existing deferred
backing destruction after registry unlock. The empty source-escrow pin uses this
same owner without allocating a snapshot.

These are the existing containing copy/IPC/fixture contexts. No root lease,
admission pool, recovery family, custody, physical operation or replay authority
is added. Last-copy-unpin still cannot bypass a mapping/loan retirement pin or
unfinished invalidation. `ScopedCopyPin`'s automatic unpin and source-transfer
destructors keep their separate outer-context qualification gap.

## Verification

Serialized native controls exercise:

- **128 preparation-prefix rejections**, 64 before and 64 after actual vector
  capacity preparation. The factory boundaries expose memory/heap/table/frame
  guards as available; pin count stays zero. A write remains allowed during
  preparation. Original frame/authority/backing counts return. These are stage
  adapters, not global allocator OOM. An actual capacity-overflow request and a
  per-call factory rejection return `FrameAllocFailed` without publishing a pin.
- Real source capability close during preparation rejects the unused snapshot.
  A writable CPU mapping or DMA pin admitted during preparation is rechecked and
  rejects copy admission without changing its pin count. The accepted competing
  owner is cleaned through its original operation, without replay.
- Real source-root retirement and ASID reuse during the pin-free interval rejects
  the captured generation. The successor's capability remains writable with no
  copied source pin, then closes normally.
- **64 heap-held snapshot fills/pin publications** use a 35-page source. Each
  snapshot matches its original frame vector. Explicit post-guard disposal
  observes the original pin still armed after snapshot free, with writes fenced;
  only then does unpin restore the count. Local guard availability and the
  original IRQ state are checked.
- Guarded abandonment retains one unused snapshot's 35-entry capacity request,
  with no frame identity/pin/root. A second control drops a real one-page copy pin
  under memory/table/heap/physical guards. Its snapshot and pin remain retained;
  authority close rejects. Source root retirement and real ASID reuse succeed,
  while the old pin/snapshot/object and original charge remain retained. A fresh
  successor allocates/pins/writes/closes independently; the fresh pin names a
  different object and cannot decrement the old one. A missing capability rejects
  before its factory, before the successor publishes new authority. Capability
  scalar reuse within that new namespace is valid. This adds **one retained data
  page**, complete object metadata and its original backing charge, with **no
  retained root/lease** from this control. No synthetic completion, counter reset
  or physical retry recovers it. These selected counts exclude other existing
  quarantine.

Existing copy bytes, move/loan escrow, hidden attachment, deferred last-pin,
mapping/loan invalidation and returned-capability controls remain enabled.
Strict base-kernel Clippy passes on AArch64 and x86_64. The copy snapshot path is
kernel-coupled; no alternate host implementation is introduced to mirror it.

Corrected native runs pass: **AArch64 20 passed / 0 failed / 0 pending** and
**x86_64 18 / 0 / 0**, with all **28 HTTP checks** and both
security-service generations per run. Healthy responses beside stalled peers
take **0.302–0.312 s / 0.330–0.401 s**, respectively;
trickle rejection completes in **30.15 s / 30.18 s**. Cancellation
pressure retires after **19,160 / 17,928 requests**.

Host retry diagnostics are recorded inline in the evidence, separately from guest
results. No test deadline, attempt bound or admission limit is widened.

Workspace formatting, whitespace and local references pass; both native runners
retain assembly-section checks. Bundled services are reused; userspace sources
and the tracked manual PDF remain unchanged. Earlier failed executions retain
their separate historical records; passing guests do not attribute their causes.

## Failed fixture executions

The initial AArch64 and x86 runs both panic at `copy_storage/tests.rs:189:5`:
the fixture expected `Err(OperationsInFlight)` from root close, but observed
`Ok(())`. Neither reaches HTTP checks or deferred self-test completion. Both
artifact identities, failure results and selected diagnostics are retained
separately from the passing repeats. ARM records raw return addresses; x86's
serial frame chain is unavailable in that stack page, while captured LLDB frames
identify `copy_storage::tests::retention` on the synchronous BSP path. AP frames
remain at their startup barrier.

The fixture incorrectly equated an unmapped backing pin with a root lease.
Existing namespace teardown can mark that object destroy-pending, detach its
authority and retire the root while its original copy pin retains backing and
the old generation's charge. The corrected control checks this actual contract:
successful root close and real successor reuse, retained old object/pin/charge,
absence before new authority publication and independent successor cleanup.
Only this test file changes
between failed and passing builds; production code, deadlines and bounds remain
identical. This explains these two fixture assertion failures, without attributing
unrelated historical root-close timeouts.

Both second executions stop at `tests.rs:206:43`, with `retired source reached
factory`. That control allocated a fresh successor capability before expecting
the old scalar to reject. Scalars can be reused within the new namespace, so it
named valid successor authority. The corrected control checks absence before
new publication, then pins the successor's actual capability and verifies its
different nonreused object identity without changing the old pin or charge.
The independent preparation-interleaving control checks captured-generation
rejection after real root reuse and successor capability publication. Both
second failure records and their source hashes are separate from the initial
assertion failures and final passes. These two corrections only change the test
file; no production code or validation limit changes between native attempts.

## Formal qualification and remaining work

Both focused safe configurations and all **seven expected counterexamples** pass
(the harness checks nine cases). IPC transaction explores **26 distinct / 71
generated states**, depth **11**; root retirement explores **2,978 / 3,733**,
depth **33**. Required safe actions execute; negative controls violate their
intended invariant through the named unsafe action.

No model inputs change. Existing loan/rollback/non-terminal-timeout and root
metadata/physical-once/original-charge projections remain relevant; neither
focused model implements copy snapshot preparation/revalidation, allocator
placement or field-destructor contexts. Those are native/source qualification,
without executable refinement, arbitrary-schedule or sustained-pressure proof.

The per-object bound limits one snapshot to its source's frame count. Concurrent
candidates and retained vectors still lack independent byte/count and aggregate
principal admission. DMA snapshot factories, automatic scoped/source-transfer
destructors, ordinary backing rollback and broader enclosing IPC/lifecycle/IRQ
contexts remain open. No factory change supplies readiness, physical completion,
cleanup custody or permission to replay accepted work. Historical mapping panics,
root-close timeouts and initial probe failures remain separately unattributed.
