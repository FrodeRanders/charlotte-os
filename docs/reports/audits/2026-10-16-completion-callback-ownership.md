# Complete completion-callback preparation ownership

Date: 2026-10-16. Baseline `448c7f4`, following shared observer-entry construction.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and scope

The shared entry constructor already retained its original weak observer and
charge across empty allocation. The containing completion-callback preparation
still assembled partial locals before constructing its owner, and completed
preparation used ordinary field destruction. Abandoning it could free candidate
lists, entry backing, weak callback references, original watch admission or the
captured completion through implicit cleanup beneath an enclosing guard.

This batch qualifies that containing callback owner, from successful exact
capture through ordinary allocation/publication outcome and explicit disposal.
It is a source finding, not attribution of historical panic, exact-worker-exit,
stack, persistence or NIC failures. It does not qualify every capture caller,
published observation/token/source destructor or outer IRQ/guard context.

## Complete owner before admission

`callbacks::Preparing` now stores the exact captured record/account/source,
original weak observer and every partial charge/candidate/entry field behind
`ManuallyDrop` before watch admission. List allocation receives only the captured
classification. Entry factories borrow this complete transaction; the ordinary
factory transfers weak observer and charge directly into the existing complete
entry constructor before its first fallible phase. The parent retains its exact
record and independently charged candidate list throughout that transfer.

On quota, list or entry-preparation failure, the constructor explicitly finishes
the complete preparation after the failing phase returns, then returns rejection.
Borrowed publication preserves existing namespace/root retirement, original
watch account, exact record and source identity checks. It links an entry
without allocation; full, stale and ready outcomes retain unused fields with
the owner until local registry/inner/list guards have left.

Finish explicitly disposes the transient weak observer, candidate reference,
unused entry and uninstalled charge before releasing the captured target last.
Unused entry backing/weak release precedes its installed watch charge through
the shared entry policy. Candidate list and record backing keep their separate
original lifetime admission. Remaining fields stay inert between explicit
steps. A published entry belongs to its returned registration, so preparation
finish does not refund that entry. There is one private factory contract and
no owning compatibility branch.

`observe_registered` now records the publication result, finishes the complete
preparation, then propagates success or error. A ready callback is invoked only
after that unused disposal, preserving the existing once-only terminal path.
Readiness does not take the operation's returned buffer or prove physical
cleanup. Closing a subscription still cancels only that subscription.

Captured-only, charged, candidate-installed and entry-complete abandonment
retain every field without freeing, refunding, canceling, notifying, acquiring
guards or logging. This is terminal retention, not Pending, public retry or
recovery custody. Ordinary rejected preparation remains unstarted rollback.
Published observations and source/token destructors retain their existing active
policies. Explicit finish still requires a qualified containing context; it
cannot remove unrelated caller guards or grant a physical root lease.

Outward errors, quota limits, callback/list capacity, source selection, record
identity, original account classification, heap allocation count/layout, public types,
readiness and IRQ policy remain unchanged. No new registry, owner family,
physical proof, timer/deadline behavior or ABI change is added.

## Native controls and retained cost

Serialized pre-AP controls use fixture namespaces and original accounts:

- Ordinary quota, list and borrowed entry-factory rejection preserve the captured
  buffered operation and return all unused charges. Rejected factories hold the
  actual heap, completion registry and record-inner guards while inspecting
  retained fields; disposal occurs only after those callbacks return.
- Injected list allocation rejection beneath those guards leaves the original weak callback
  and watch charge with the same owner. Explicit unused finish at all four
  phases checks that heap/registry/inner guards are available and validates
  charge/weak release order before each disposal step.
- Sixty-four preparations publish under a held heap guard. Explicit post-guard
  finish releases losing candidate lists without refunding published entry
  charges. Actual token cancellation releases every entry, leaves the operation
  in flight with its original buffer, and invokes no callback.
- Abandon all four phases under simultaneously held heap, completion-registry
  and record-inner guards after callback strong references have gone. Close and
  replace the namespace; the complete original buffered record and old account
  charges survive while successor accounts remain empty. Preserve enclosing IRQ
  state. A normal temporary weak upgrade inspects retained record data; no raw
  handle reconstruction, ownership adoption or cleanup retry occurs.

The last control deliberately retains **one original ordinary record charge,
its 16-byte buffer, three ordinary watch charges, two candidate lists, one entry
Box and four weak-only callback control blocks** per guest. Four captured targets
hold the same original completion and watch account. Its record retains its
independent waiter source, so total new list admission is **three list charges**:
two candidates plus that record's waiter list. Record and three list allocations
also retain their four original private charge holders. There is no retained
registration token, active callback, timer charge, capability authority, real
user root or physical pin from this control. Earlier costs are itemized
separately. This is terminal fixture retention until guest shutdown; no live
sponsor is replaced or counter reset. The full list-admission fixture precedes
these controls.

Existing competing/full/ready/stale/retired callback publication and public
terminal notification tests now use explicit preparation finish. Reentrant
namespace disposal, CQ detachment, shared entry and broader wait/IPC controls
remain. Existing C16/C18 and callback boundary rows are updated in
[cleanup/recovery](../../reference/cleanup-recovery.md). Capture entry contexts,
published observation/source/token disposal, aggregate metadata admission and
broader IRQ/guard qualification remain separate work.

## Validation and evidence

Both first no-network samples (ARM and Intel) panic at the new fixture's
`ordinary_rejection` teardown, line 90, with `NotComplete`. I had used terminal-only
`close` immediately after asserting that the buffered operation remained in
flight. The rejection was correct; the fixture cleanup was wrong. The same
cleanup mistake appeared at two later, unreached sites. All three now use
`abort_submission` for these serialized, never-externally-published fixture
submissions after ownership assertions and local guard exit. No terminal result
is fabricated and production close semantics are unchanged.

[Separate failed-run evidence](evidence/2026-10-16-completion-callback-fixture-failures.json)
preserves original source/artifact hashes, both panic excerpts and all four CPU
registers/symbolic backtraces. Intel identifies `ordinary_rejection` at line 90;
optimized ARM identifies its containing test runner. Both UARTs locate the same
line, with all three APs still at the boot barrier. These introduced fixture
failures remain separate from passing corrected repeats and historical
unattributed incidents; they provide no attribution for those older samples.

The host runner passes **509 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls including both real-
mtools image regressions. This host run precedes the final kernel-only fixture
teardown correction; host-tested sources did not change. Strict kernel Clippy
passes on both custom targets after that correction with `-- -D warnings`.
Workspace formatting, diff whitespace and relative documentation links pass.

Six reviewed Rust files were frozen after correction and before the repeat
matrix. Every profile uses four LPs, fresh storage, a 180-second guest budget,
denied Rust warnings, assembly-section checks and automatic all-CPU failure
capture. The five corrected runs exit zero with diagnostic result `complete`,
zero failed/pending:

| Profile | Deferred tests passed | Completion |
| --- | --- | --- |
| ARM / SMMUv3, no network | 17 | 11.456330 s |
| Intel VT-d, no network | 15 | 19.225252 s |
| AMD-Vi, no network | 15 | 15.748770 s |
| ARM / SMMUv3, security | 19 | 56.740976 s |
| Intel VT-d, security | 17 | 61.393632 s |

Every corrected profile passes three ordinary rejection cases, four explicit
unused finish phases, retaining list rejection under the actual heap/registry/
inner guards, 64 heap-held publications followed by post-guard finishes and
actual token cancellations, and four guarded abandonment phases with exact
old-account retention across namespace replacement. No callback is fabricated
and cancellation leaves the buffered operation in flight. The new terminal
fixture costs and all earlier costs are itemized separately in the evidence.
Existing callback full/ready/stale/retired controls, shared entry construction,
record/initial admission, timer preparation/cancellation/delivery, scheduler and
IPC controls pass too. Each profile completes 64 scheduled close/exit episodes
with exact exit watches (320 total). Both security profiles complete two scoped
EL0 generations. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration. These are bounded regression observations, not
sustained fairness, hostile pressure or physical hardware qualification.

No guest panic, deadline or persistence fail-stop was observed in the five
corrected runs. Both introduced fixture panics above remain separately published;
passing repeats do not erase them or attribute older incidents.

[Version-controlled evidence](evidence/2026-10-16-completion-callback-ownership.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations, the separate failed samples and
new/prior fixture costs. All commands use `--smp 4 --fresh-storage --timeout 180`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance callback-owner-final-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance callback-owner-final-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance callback-owner-final-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance callback-owner-final-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance callback-owner-final-intel-security
```

All use `CATTEN_SKIP_EMBED_BUILD=1` and `CARGO_BUILD_RUSTFLAGS='-D warnings'`.
AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security adds `CATTEN_ALLOW_SANDBOX_NETWORK=1` and
HTTP/deployment ports 19080/19081 on ARM, 19180/19181 on Intel. All five boot
images use the already-corrected 128 MiB default; tooling is unchanged. All
staged service hashes and run inventories match the prior baseline; no service
or runtime library changed or was rebuilt. Frozen Rust, tooling and bundle
hashes are rechecked after the matrix; documentation is finalized afterward.
Only the new fixture source differs between failed and corrected native samples.
Published evidence is self-contained; no full-tree or platform attestation follows.

Earlier introduced temporary-guard failures remain in the
[unpublished ownership evidence](evidence/2026-10-16-timer-unpublished-failures.json),
and older unattributed samples remain in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md).
These passes do not establish their causes or close the wider audit gaps.
