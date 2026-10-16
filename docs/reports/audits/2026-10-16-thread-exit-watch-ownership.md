# Complete thread-exit watch preparation ownership

Date: 2026-10-16. Baseline `9c921366`, following completion-callback ownership.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and scope

`PreparingExitWatch` previously assembled its list and entry as ordinary locals,
then used ordinary field destruction on abandonment or publication outcome.
The scheduler's injectable preparation factory also accepted the captured list,
weak observer and original watch charge by value, before any containing owner
held them. An interrupted partial preparation could destroy list/entry backing,
weak callback allocation or admission beneath an enclosing guard.

This batch qualifies that containing owner after successful source capture and
watch admission, through list/entry preparation, borrowed publication and explicit
unused disposal. It does not qualify every initial capture/admission caller,
published token/source/batch destructor, worker construction or outer IRQ/guard
context. This is a source finding, not attribution of historical exact-exit,
stack, persistence or NIC failures.

## Owner and explicit finish

`PreparingExitWatch::capture` now stores the captured candidate source, original
weak observer and already-admitted watch charge behind `ManuallyDrop` before
fallible list/entry preparation. The list allocator receives only the original
charge's classification. Entry factories borrow this owner; the ordinary entry
constructor consumes weak observer and charge directly into the existing shared
complete entry constructor. The candidate source remains with the containing
owner throughout that transfer.

The scheduler captures the same exact generation/source under its short table
borrow, constructs the owner, then lends it to one current preparation hook.
The former owning-hook contract is removed. Rejection explicitly finishes after
that hook and its local guards return. Successful preparation still revalidates
the captured generation and caller's exact root identity before allocation-free
publication under the table. Ordinary success, stale identity, authorization
rejection and closed/full source outcomes all finish after table unlock.
Direct source registration uses the same explicit finish after publication.

Finish disposes an uninstalled weak observer, unused entry, uninstalled charge,
then the captured candidate list last. The completed shared entry's existing
backing/weak-before-charge policy remains. Published entry admission belongs to
the returned registration, so preparation finish does not refund it. Keeping the
candidate source through transient disposal preserves its allocation admission;
first-source competitors still use the actual published winner and release
losing storage afterward.

Abandonment at captured, existing-source, allocated-candidate and completed-entry
phases retains every remaining field without allocator/budget/source/table
guards, callbacks, logging, cancellation or refund. Remaining fields also stay
inert between explicit finish steps. This is terminal retention, not Pending,
physical cleanup proof, recovery custody or public replay authority. A captured
list is metadata ownership, not a target-thread/root lease. Registered tokens,
notification batches and source teardown retain their existing active policies.

Public errors, watch/list capacities, exact-generation authorization, heap
allocation count/layout, worker producer lifetime, callback delivery and IRQ
policy are unchanged. No new registry, owner family, timer/deadline behavior,
runtime ABI or wire/storage grammar is introduced.

## Controls and deliberately retained cost

Existing tests now borrow the complete owner and finish unused preparations
explicitly. They retain stale-generation/recycled-TID rejection for both TID-only
and generation-qualified requests, competing first-source publication, foreign-
root authorization, heap-held publication/closed-source rejection and scheduled
table reentry under both caller IRQ policies. Two ordinary scheduler-hook
rejections hold the actual heap and thread-table guards, once before storage and
once after complete preparation, confirming cleanup only after callback return.

New serialized pre-AP controls cover:

- Injected list allocation and borrowed entry rejection beneath actual heap,
  thread-table and source-slot guards, retaining original weak observer/charge
  and any candidate list until explicit finish.
- Four unused phases, checking guard availability and weak/entry/charge-before-
  list disposal order at all four finish checkpoints. A final-weak-only callback
  is also explicitly released after its allocator/source contexts are available.
- Sixty-four first-source candidates published beneath held heap/thread-table
  guards, followed by post-guard finishes. Losing lists release without refunding
  published entry charges; actual token cancellation clears every entry without
  invoking a callback.
- Four abandoned phases beneath simultaneously held heap, thread-table and
  source-slot guards, after callback strong references have gone. Explicit close
  of the empty standalone fixture source leaves original charges retained and a
  fresh independent account/source empty. No scheduler-admitted target or exit
  callback is fabricated. The caller's IRQ state is preserved.

The final control deliberately retains **four ordinary watch charges, three
list allocations and their three private charge holders, one completed entry
Box and four weak-only callback control blocks** per guest. One list is the
captured original empty source, now closed; two are unlinked candidates. All
four watch charges retain their original account. No token, linked entry,
active callback, completion record, timer charge, authority, thread stack,
user root or physical pin is retained by this new control. These costs are
terminal until guest shutdown and separate from earlier fixture costs. No live
sponsor is replaced and no counter is reset. Existing pool saturation fixtures
use their captured baselines and normal admission around these retained slots.

Existing C16/C18 and the shared callback/exit-watch boundary row are updated in
[cleanup/recovery](../../reference/cleanup-recovery.md). Generic completed
`PreparedRegistration`, containing waiter/source/token destruction, aggregate
metadata/principal admission, pre-capture callers and wider IRQ/guard contexts
remain separate work.

## Validation and evidence

The host runner passes **509 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls including both real-
mtools image regressions. Strict kernel Clippy passes on both custom targets
with `-- -D warnings`. Workspace formatting, diff whitespace and relative
Markdown links pass.

Eight reviewed Rust files were frozen before native validation. Each profile
uses four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-permission checks and automatic all-CPU failure capture. All five
runs exit zero with diagnostic result `complete`, zero failed/pending:

| Profile | Deferred tests passed | Completion |
| --- | --- | --- |
| ARM / SMMUv3, no network | 17 | 11.890310 s |
| Intel VT-d, no network | 15 | 14.699128 s |
| AMD-Vi, no network | 15 | 14.812700 s |
| ARM / SMMUv3, security | 19 | 55.835968 s |
| Intel VT-d, security | 17 | 61.434251 s |

Each profile passes both ordinary scheduler-hook rejections, retaining guarded
list/borrowed-entry rejection, four ordered unused finish phases, explicit
final-weak disposal, 64 heap/table-held publications followed by post-guard
finishes and actual token cancellations, and four guarded abandonment phases.
Existing exact-generation reuse, competing-source, authorization, closed-source
and scheduled IRQ-preservation controls pass. New terminal fixture costs and
all prior costs remain separately itemized in the evidence. Shared entry and
completion-callback ownership, submission/record construction, timer preparation/
cancellation/delivery and broader scheduler/IPC controls pass too. Each profile
completes 64 scheduled close/exit episodes with exact exit watches (320 total).
Both security profiles complete two scoped EL0 generations. ARM retains certified
runtime migration; x86 qualifies only never-dispatched migration. These are
bounded regression observations, not sustained fairness, hostile pressure or
physical hardware qualification.

No guest panic, deadline or persistence fail-stop was observed in these five
runs. The preceding callback batch's two introduced fixture panics remain
separately preserved in
[their original evidence](evidence/2026-10-16-completion-callback-fixture-failures.json).
These passing samples do not replace those failures or attribute older incidents.

[Version-controlled evidence](evidence/2026-10-16-thread-exit-watch-ownership.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations and new/prior fixture costs.
The five commands are:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance exit-owner-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance exit-owner-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance exit-owner-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance exit-owner-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance exit-owner-intel-security
```

All use `CATTEN_SKIP_EMBED_BUILD=1` and `CARGO_BUILD_RUSTFLAGS='-D warnings'`.
AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security adds `CATTEN_ALLOW_SANDBOX_NETWORK=1` and
HTTP/deployment ports 19080/19081 on ARM, 19180/19181 on Intel. All boot images
use the existing 128 MiB default. Tooling and all staged service hashes/run
inventories match the preceding baseline; no service or runtime library changed
or was rebuilt. Frozen Rust, tooling and bundle hashes are rechecked after the
matrix; documentation is finalized afterward. Published evidence is self-contained
and supplies no full-tree or platform attestation.

Earlier temporary-guard failures remain in the
[unpublished ownership evidence](evidence/2026-10-16-timer-unpublished-failures.json),
and older unattributed samples remain in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md).
Historical causality, broader caller contexts and numbered audit gaps remain open.
