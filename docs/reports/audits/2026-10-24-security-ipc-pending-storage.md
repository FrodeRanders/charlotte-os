# Security audit: pending-call storage

Date: 2026-10-24. Reviewed baseline: `2936d70d`.

Pending-call nodes now allocate fallibly outside IPC, after retaining the exact
caller root and reserving the original call/reply counts. Publication relinks
that storage. Existing close/cancellation owners carry detached pending nodes
past IPC unlock, then free payload and node before refunding their count charge.

The [tracked evidence](evidence/2026-10-24-security-ipc-pending-storage.json)
records implementation/formal input hashes, native artifact identities and
selected diagnostics. This advances SEC-07 R7-2 and C08/C18 ownership/context
work. Counts remain **29 corrected / 6 partial / 3 open**; SEC-07/18 remain
partial and production trust remains disabled.

## Preparation and publication

Previously `PreparedCall` published joint IPC/memory authority and then inserted
its pending record into an allocating `BTreeMap` under IPC. Its field destructors
also handled preparation failure in several contexts, including IPC-held paths.

Public entry checks the connection type/rights and endpoint queue before count
admission or node allocation. Those inputs are rechecked before mutation.
The same containing owner now captures its caller namespace, leaves IPC before
acquiring an exact-root operation lease, revalidates that captured generation,
and reserves both record dimensions atomically. It releases IPC before fallible
pending-node allocation. The original reservations remain inside the owner
through allocation failure; no quota or authority is inferred from a reused ASID.
The immortal kernel root and explicitly prepared top-half synthetic fixtures
keep their existing special contexts, without granting platform classification
to synthetic callers.

`PreparedCall` now retains partial storage, reply reservation, pending waiter
metadata, capability reservation, optional delegated connection, a currently
attaching transfer, memory/loan vectors and the caller lease together. It lends
itself to staging and publication. All scalar, move, loan, copied and vector call
paths explicitly finish ordinary results after their local IPC guards leave;
unused fields finish before releasing the root. Abandonment retains every field
without allocator/registry cleanup. Publication is attempted at most once per
owner; this adds no retry entry point.

The pending registry uses shared `AdmittedMap` nodes. The original call charge
is attached to its already-allocated node before publication and follows that
node through successful publication, detachment and actual storage deallocation.
Payload destruction precedes node free, which precedes count refund. Quota still
precedes node allocation: the helper binding the charge adds no counter lookup,
allocation or classification change.

The old synthetic stale-preparation fixture allowed its caller root to retire
while a copied call was staged. The new owner makes that state unreachable:
close reports `OperationsInFlight` until explicit finish. The updated control
then retires the root, reuses its exact ASID/capability number, and verifies the
successor independently. Existing joint-authority retirement rejection controls
remain; no live lease is released to manufacture the older state.

## Close and cancellation

The existing serialized call-close and `PreparedCancellation` paths detach the
complete pending node only after their existing loan/claim checks permit it.
They leave IPC before disposing the payload/node and original charge. Failed
loan cleanup keeps the pending record, charge, uncertain backing and fences
live. Observation and CQ wake semantics are unchanged; they cannot refund a
retained call. Detached fallback retains the node and charge without cleanup.

This continues C08/C18 with existing root-operation and shared node machinery.
No new root, pin, registry, family, custody or physical replay authority is added.
The [cleanup rows](../../reference/cleanup-recovery.md),
[record contract](../../reference/ipc-record-budgets.md),
[lock order](../../reference/locking.md) and living Markdown/LaTeX manual are
updated.

## Verification

The serialized controls exercise:

- **64 invalid-handle rejections** prove the node allocator is never entered
  and no count/authority reservation changes.
- **64 injected preparation rejections** after exact-root/count admission and
  before node allocation. IPC/lifecycle/table guards are available at that
  boundary; the original count/authority reservations return to their baseline.
- **64 heap-held local publications and detaches** with no new allocation or
  premature refund. Explicit node release returns its original call count;
  finishing the containing owner restores the full baseline and preserves IRQ
  state.
- Three deliberately abandoned preparations retain charged storage, three
  call/reply reservations, five fixture roots and copied backing. Three caller leases remain retained;
  two server fixture roots also remain live. Dropping the owners
  under IPC/heap guards performs no field cleanup; admitted/copied phases also
  hold lifecycle/table guards.
- Existing quota, joint retirement, source rollback, copied/vector failure,
  returned-resource, queued/delivered cancellation, waiter/CQ and scheduled
  endpoint/namespace controls remain enabled. The updated exact-root control
  checks rejection before teardown and independent successor use after finish.
- **16 shared collection host tests** pass. The added test binds admission to
  prepared storage without allocation and observes actual payload/node
  deallocation before refund, for both unused and published storage.

Both first final-layout native runs pass: ARM **20 / 0 / 0**, x86 **18 / 0 / 0**. Each
completes two scoped-security generations and all **14 host HTTP controls**.
Healthy replies beside stalled peers take **0.220–0.385 s** on ARM and
**0.259–0.347 s** on x86. Trickled bodies reject in **30.10 s** and **30.20 s**;
concurrent cancellation traffic retires after **20,720** and **18,252** requests.

The host probes used their existing bounded transport retries. Both final
captures show four `ConnectionResetError` attempts around EOF, abortive-close
and idle recovery controls. The final ARM initial metrics probe also times out
once before its retry succeeds; an earlier x86 layout captured the same symptom.
The failed host attempts and adjacent successful controls are retained separately
in the evidence, alongside the two superseded passing images. Their cause was
not isolated. Each final layout passes its first guest run; no timeout or counter
was widened. Passing aggregate results do not erase these observations.

Strict base-kernel Clippy passes on both architectures. Workspace formatting,
whitespace and shell syntax checks pass. Userspace source and embedded service
bundles are unchanged; the checked-in manual PDF is preserved.

## Formal qualification

All **17 focused checks** pass: two safe configurations and 15 expected
counterexamples. IPC publication explores **7,634,722 distinct / 36,122,253
generated states**, depth 42; root retirement explores **2,978 / 3,733**, depth
33. All required safe actions execute; each unsafe case violates its named
invariant through the intended action.

No model/configuration changed. `CharlotteIpcPublication` projects joint shared
capability/memory publication, hidden delivery, rollback and charge retention;
its storage preparation is shared authority staging, not this family's node or
call/reply count dimensions. `CharlotteRootRetirement` projects the retained
caller lease and cleanup gate. Node allocation/disposal and the count's actual
allocation lifetime are native/host qualification. The preceding complete
**190-case** run remains the separate full-suite checkpoint. These results give
no executable refinement, allocator/IRQ or sustained-pressure fairness proof.

## Remaining work

Endpoint, reply-token and per-capability maps still allocate under IPC, including
other insertions after authority mutation. Pending storage alone does not make
IPC publication fully fallible. Waiter/control-block and vector factory contexts,
original authority/connection destructors, broader byte/principal admission,
allocator overhead and all enclosing IRQ/guard contexts remain separate work.

Pending-map lookup/insertion is now linear. Its existing record counts bound
size, but serialized controls do not qualify worst-case hold time, contention,
or sustained management/timer/teardown fairness. No quota or deadline was widened.
The earlier queued-loan mapping panic and root-close timeouts retain their
historical diagnostics and causal limitations; passing this batch does not
attribute them or grant replay of uncertain cleanup.
