# SEC-18: inert memory-object backing abandonment

Date: 2026-10-26. Reviewed baseline: `98a2ffde`.

`ChargedFrames::Drop` previously invoked physical release, node/vector disposal
and original-charge refund, including from private or charge-only preparation.
Its caller's outer guards and IRQ context are unknown. This is a source-confirmed
C05/C18/G1 gap; no hostile cross-domain deadlock or early-frame-reuse exploit is
claimed here.

The backing owner's fallback now retains every field without allocator,
registry, accounting, callback or logging work. Ordinary preparation rejection,
last-pin cleanup and cancelled private-copy cleanup consume the same owner
explicitly. A native negative control restoring the prior Drop body fails the
new reservation-retention assertion before held-lock tests are entered.

The [evidence](evidence/2026-10-26-security-backing-abandonment.json) records
source/artifact/model hashes, the deliberate failing control, final executions
and relevant failure diagnostics. SEC-18 and SEC-07 remain partial; the ledger
stays **29 corrected / 6 partial / 3 open** and production trust stays disabled.

## Ownership and cleanup boundary

The existing `ManuallyDrop<FrameResources>` retains original frame identities,
charge, unused object storage or complete published object node, and the
started/failed release fence. Reservation-only abandonment also keeps its
original quota. Root retirement and successor reuse cannot supply a refund or
authority to adopt those resources. This is terminal retention, not implemented
recovery custody or a returned retry owner.

`try_prepare` borrows this complete owner for fallible private work. A returned
error leaves the callback's local guards before consuming explicit release;
the original preparation error remains the result. Failed physical cancellation
retains the owner and original charge. A successful callback returns the same
owner for consuming handoff. The constructor uses that path for object-node,
frame-vector and partial physical-frame preparation failure.

Direct allocation keeps memory serialization from final validation and authority
publication through object/payload handoff. Returning that same guard from
preparation adds no fallible work after authority publication. Copy preparation
explicitly cancels its private frames if its later identity allocation rejects.
No cleanup reconstructs ownership from an ASID, capability or physical address.

Last-copy/DMA-pin cleanup now calls consuming release explicitly. Cancelled copy
backing is explicitly released at the end of the existing containing transfer
destructor, after destination metadata/authority, matching its previous field
destruction order. **That containing destructor remains active and separately
unqualified.** Local guard exit does not qualify an unrelated enclosing IPC,
lifecycle or IRQ context; no mask is removed or force-enabled in this batch.

Existing mapping/loan/hardware proofs, sixteen-frame physical batches and
physical-before-node-before-charge disposal remain. The started/failed fence
still prevents replay after partial physical rejection. `RetiredMemory` keeps
its existing retaining fallback and consuming release. No root/pin, registry,
new custody category, release proof or physical retry authority is added. Limits,
wire/register ABI and userspace code/bundles are unchanged.

## Regression controls

The serialized kernel controls exercise the real owning path:

- Existing **128 charged construction failures and 64 copied-prefix failures**
  still restore original admission and data-frame baselines.
- **128 failures after one/two actual allocated frames** return from the
  allocator-held callback before explicit cancellation. Original frame and
  budget baselines return. This is prefix rejection, not global allocator OOM.
- **64 post-allocation rejections** return while local memory/table/heap/frame
  guards are held inside the callback. Those guards leave before cleanup; the
  original `AddressSpaceMissing` error and exact data-frame/budget baseline
  are preserved.
- **64 real copy cancellations and successful copy/public-close releases**
  preserve original source sponsorship and shared authority counts. A prepared
  copy rejected after target retirement and actual ASID reuse releases its old
  private backing without altering the successor's live capability.
- Existing heap-held publication/detachment and terminal physical-failure
  controls still pass. A failed release retains its complete node/charge and
  never replays consumed frame identities.
- Reservation-only abandonment retains its one-page/one-object admission.
  Private and published backing then abandon while memory/table/heap/frame/
  memory-ledger/shared-authority guards are held. No cleanup or refund occurs;
  IRQ state is preserved. This retains **two actual data pages and three
  original page/object reservations**, complete metadata and one detached
  authority record through real root retirement and successor reuse.

The new retention footprint is intentional fixture quarantine, separate from
the existing one-page failed-release fixture. Production ordinary cancellation
continues through explicit consuming paths; abandonment remains terminal
retention. These observations supply no aggregate physical-RAM ledger or
recovery owner.

## Native negative control

Only `ChargedFrames::Drop` was temporarily restored to `self.free()`; the rest of
the final source and new tests stayed in place. The isolated four-LP AArch64
no-network run stopped with `reason: kernel-panic` at the new assertion:

```text
reservation-only backing abandonment refunded charge
left:  Amount { pages: 0, objects: 0 }
right: Amount { pages: 1, objects: 1 }
```

This unguarded check precedes data-owner abandonment under held guards. The
failure therefore establishes the old refund behavior without a deliberate
deadlock. It is an expected regression-control failure, not a passing guest or
an unexplained incident. Artifact hashes and the exact diagnostic are preserved
separately. The final source was restored byte for byte before passing runs.

## Formal scope and tool limitation

The unchanged `CharlotteRootRetirement` and `CharlottePrincipalBacking`
projections pass **two safe configurations and eleven expected counterexamples**.
Required actions execute; unsafe cases violate their named invariants.

| Model | Distinct / generated states | Depth |
| --- | --- | --- |
| Root retirement | 2978 / 3733 | 33 |
| Principal backing | 72344 / 290961 | 14 |

Both initial sandbox attempts stopped before exploration because TLC's RMI
listener was denied (`java.rmi.server.ExportException`, port 0,
`java.net.SocketException: Operation not permitted`). They are not model
counterexamples. Permitted reruns use two workers and complete successfully;
the tool failures remain recorded separately from the semantic controls.

No model input changed. These are existing physical-once/original-charge policy
projections. Concrete construction prefixes, implicit field destruction, local
guard exit and arbitrary outer/IRQ contexts are native/source qualification,
not executable refinement. No general allocator, quiescence, recovery or
essential-service fairness proof follows.

## Final native validation and remaining scope

Strict base-kernel Clippy passes for both custom targets with signed bundle
inputs and `--no-default-features --features acpi -- -D warnings`. The final
four-file Rust hash set matches the source restored after the deliberate
negative control. Both existing signed userspace bundles are reused; no service,
runtime, signature input or wire format changes are required.

Final fresh-storage, four-LP security/HTTP runs execute sequentially. Each first
final-source guest attempt finishes with `reason: complete`, with both security
publication generations (1 and 2) passing:

| Backend | Deferred passed / failed / pending | HTTP checks passed |
| --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 |
| x86_64 / AMD-Vi | 18 / 0 / 0 | 14 |

Existing mixed memory/authority publication, receive rollback, loan/cancellation,
returned-resource, DMA, root-retirement and readiness controls stay enabled.
The expected native old-Drop panic is recorded separately and is not counted as
a final-source failure or passing repeat. No unexpected final guest failed.

Each host HTTP run records four `ConnectionResetError` retry diagnostics: one
after EOF, one after abortive peer closes and two during the idle-client probe.
Intel also records one initial-metrics `TimeoutError`. All affected probes
subsequently pass. Evidence keeps all thirteen diagnostics with their eventual
results. No timeout, attempt bound or quota is widened.
Passing results do not establish the cause of historical first-entry/scheduler,
IPC, root-close, metrics or NIC failures.

The manual rebuild completes two passes; affected/adjacent PDF pages 51–53 are
rendered and visually checked. The full 249-page build still emits 189 overfull-
box warnings, with no undefined references; the remaining pages are not
re-audited. Workspace formatting, whitespace, source hashes and local Markdown
targets pass.

This qualifies the backing owner's fallback and its local preparation/error
boundaries. Active containing transfer/scoped-pin destructors, unrelated outer
IPC/lifecycle/IRQ holds, allocator interruption and sustained concurrent pressure
remain C05/C18/G1/G7 work. Recovery needs a complete eligible operation owner and
authorized policy; retained allocations or an error code provide neither.
SEC-18/07 stay partial and the other finding dispositions remain unchanged.
