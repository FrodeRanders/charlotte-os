# Complete waiter preparation ownership

Date: 2026-10-16. Baseline `e07435cf`, following thread-exit watch ownership.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and scope

`WaitSponsor::prepare` previously reserved a charge, constructed an entry and
only then captured the source in `PreparedWaitRegistration`. The completed
containing owner relied on ordinary entry/source field destruction. Abandonment
could release backing, weak callback references, original admission or the source
under an enclosing guard, even though normal IPC/CQ publication already borrowed
the prepared entry and disposed it after local registry unlock.

This batch qualifies that shared containing owner from exact source/sponsor
capture through admission, entry preparation, borrowed publication and explicit
unused disposal. Initial source acquisition, lazy source-list construction,
published registrations/source/batch destruction, scheduler waker construction
and every outer guard/IRQ context remain separate work. This is a source finding,
not attribution of historical exact-exit, stack, persistence or NIC failures.

## Complete owner and ordinary outcomes

`PreparedWaitRegistration` now retains the exact source, original sponsor and
weak observer behind `ManuallyDrop` before waiter admission. Its partial charge
and completed entry join that owner as preparation advances. Admission uses the
captured sponsor; it does not re-resolve an ASID or derive platform classification
from a role or source. Entry factories borrow the complete owner. The ordinary
constructor transfers observer/charge directly into the existing shared entry
constructor while retaining source/sponsor throughout its fallible allocation.

Quota, retired-sponsor and entry-preparation rejection explicitly finish after
the failing phase returns. `WaitSponsor::register`, IPC endpoint/reply and CQ
adapters record publication results, finish after local list/registry guards
leave, then return the original success/ready/error outcome. All private IPC/CQ
fixtures use that same consuming finish. Exact list identity still precedes
readiness under the registry, including absent, foreign and reused sources.
Readiness neither observes returned resources nor proves producer completion.

Finish releases an uninstalled weak observer, unused entry, uninstalled charge,
then captured source, with the original sponsor last. The shared completed
entry's backing/weak-before-charge policy remains. A published entry belongs to
its registration; finishing preparation cannot refund that entry. Source
allocation admission survives until actual source backing release. Remaining
fields stay inert between explicit finish steps too.

Captured-only, admitted and completed-entry abandonment retains every remaining
field without heap/budget/source/table guards, callbacks, cancellation, logging
or refund. This is terminal retention, not Pending, a retry owner or recovery
custody. An owning source/sponsor reference retains metadata, not a thread/root
lease, physical backing pin or quiescence proof. Active registration/source/batch
destructors retain their existing policies. Consuming finish still requires a
qualified containing context and does not remove an unrelated caller lock/mask.

Public source/registration interfaces and errors, 64-entry capacity, original
charge classification, heap allocation count/layout, notification and IRQ
policy remain unchanged. The private borrowed factory has one contract. No
compatibility branch, new registry, owner family, runtime ABI, wire/storage
grammar, timer/deadline behavior or operation replay is introduced.

## Controls and retained cost

New serialized pre-AP controls exercise:

- Ordinary domain-quota, retired-sponsor and borrowed entry rejection. The entry
  factory holds actual heap/thread-table guards while inspecting the captured
  source and original charge; cleanup follows its return. Admission failures
  never enter that factory and preserve their distinct error values.
- Explicit unused finish at all three phases with five ordered checkpoints,
  checking heap/table availability and weak/entry/charge release order. A
  complete owner holding the final source and final weak callback references
  releases entry/admission before source allocation, retaining its sponsor last.
- Sixty-four publications under held heap/thread-table guards, followed by
  post-guard finishes and actual token cancellations. Promotion after admission
  leaves all 64 original ordinary charges ordinary. No callback is invoked.
- Abandon all three phases under held heap/thread-table guards after callback
  strong references have gone. Closing the original empty source and retiring
  its sponsor leave original admission retained; a fresh independent sponsor
  remains empty. The original sponsor rejects fresh admission as Closed.
  Enclosing IRQ state is preserved; no live sponsor is replaced or reset.

The last fixture deliberately retains **two original ordinary waiter charges,
one closed source list and its private charge holder, one completed entry Box,
three weak-only callback control blocks and the original sponsor/account** per
guest. Three captured owners share that source and sponsor. There is no retained
registration token, linked entry, active callback, event-watch charge, completion
record, timer charge, authority, target thread, stack, user root or physical pin
from this new control. Retention is terminal until guest shutdown. Earlier costs
remain separately itemized. Full list-pool saturation executes before these new
controls; no pool counter or admission policy is changed.

Existing IPC receive/reply full/ready/foreign/closed-source and CQ full/ready/
reused-source tests continue to publish under a held heap guard and now finish
explicitly after it leaves. Existing waiter-source competing initialization,
callback/exit-watch ownership, actual scheduler wakes and cleanup controls remain.
C16/C18 and the existing lazy waiter/IPC/CQ boundary row are updated in
[cleanup/recovery](../../reference/cleanup-recovery.md). Generic completed entry,
source/token/batch destruction, aggregate metadata/principal admission and broader
caller contexts remain separate qualification work.

## Validation and evidence

The host runner passes **509 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls including both real-
mtools image regressions. Strict kernel Clippy passes on both custom targets
with `-- -D warnings`. Workspace formatting, diff whitespace and relative
Markdown links pass.

Ten reviewed Rust files were frozen before native validation. Each profile
uses four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-permission checks and automatic all-CPU failure capture. All five
runs exit zero with diagnostic result `complete`, zero failed/pending:

| Profile | Deferred tests passed | Completion |
| --- | --- | --- |
| ARM / SMMUv3, no network | 17 | 11.693783 s |
| Intel VT-d, no network | 15 | 14.354563 s |
| AMD-Vi, no network | 15 | 17.359697 s |
| ARM / SMMUv3, security | 19 | 56.985660 s |
| Intel VT-d, security | 17 | 61.609297 s |

Every profile passes distinct quota/retired-sponsor/borrowed-entry rejection,
three ordered unused finish phases, final weak/source disposal, 64 heap/table-
held publications with post-guard finishes and actual token cancellations,
original ordinary classification through promotion, and three guarded abandoned
phases retaining original admission after source close and sponsor retirement.
Existing IPC/CQ full/ready/foreign/reused/closed-source publication controls pass
with explicit post-guard finish. New terminal fixture costs and all prior costs
remain separately itemized. Callback/exit-watch ownership, shared entry,
submission/record construction, timer preparation/cancellation/delivery and
broader scheduler/IPC controls pass too. Each profile completes 64 scheduled
close/exit episodes with exact exit watches (320 total). Both security profiles
complete two scoped EL0 generations. ARM retains certified runtime migration;
x86 qualifies only never-dispatched migration. These are bounded regression
observations, not sustained fairness, hostile pressure or physical hardware
qualification.

No guest panic, deadline or persistence fail-stop was observed in these five
runs. The earlier callback batch's two introduced fixture panics remain separately
preserved in
[their original evidence](evidence/2026-10-16-completion-callback-fixture-failures.json).
Passing samples do not replace those failures or attribute older incidents.

[Version-controlled evidence](evidence/2026-10-16-waiter-preparation-ownership.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations and new/prior fixture costs.
The commands are:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance wait-owner-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance wait-owner-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance wait-owner-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance wait-owner-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance wait-owner-intel-security
```

All use `CATTEN_SKIP_EMBED_BUILD=1` and `CARGO_BUILD_RUSTFLAGS='-D warnings'`.
AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security adds `CATTEN_ALLOW_SANDBOX_NETWORK=1` and
HTTP/deployment ports 19080/19081 on ARM, 19180/19181 on Intel. All boot images
use the existing 128 MiB default. Tooling and staged service hashes/run inventories
match the preceding baseline; no service or runtime library changed or was
rebuilt. Frozen Rust, tooling and bundle hashes are rechecked after the matrix;
documentation is finalized afterward. Published evidence is self-contained and
supplies no full-tree or platform attestation.

Earlier temporary-guard failures remain in the
[unpublished ownership evidence](evidence/2026-10-16-timer-unpublished-failures.json),
and older unattributed samples remain in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md).
Historical causality, broader caller contexts and numbered audit gaps remain open.
