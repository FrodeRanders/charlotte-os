# Lazy waiter-source transaction ownership

Date: 2026-10-16. Baseline `80d88831`, following completed-entry ownership.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and scope

Lazy `WaiterSource::register` previously held its weak observer, candidate and
selected list in ordinary locals until handing them to the shared waiter owner.
Normal list preparation/publication already left the source-slot guard and
released losing candidates before waiter admission. The containing transaction
still lacked an inert owner across those steps and the child handoff.

This batch qualifies that lazy-source containing owner from original sponsor/
callback capture through candidate construction, selection, losing disposal,
entry handoff/publication and explicit finish. The source remains borrowed
through the ordinary register call as before. Its list metadata is not a lease
on an enclosing thread, timer or physical root. The inner list factory, source
destruction, active token/batch policies, waker construction and every outer
guard/IRQ context remain separate work. This is a source finding, not attribution
of historical exact-exit, stack, persistence or NIC failures.

## Complete transaction and first-winner policy

One `Preparing` owner now captures the original sponsor and weak callback behind
`ManuallyDrop` before list lookup or allocation. Existing list references,
candidate, selected winner and the shared `PreparedWaitRegistration` child join
that same owner as preparation advances. The list allocator receives only the
captured sponsor's checked classification. A sponsor already retired at that
check rejects before fresh allocation. Existing list reuse still reaches normal waiter admission and its
retirement check; no role, name or later ASID lookup supplies classification.

Selection borrows the candidate and retains the actual first published winner
after the source-slot guard leaves. Losing candidate disposal remains explicit
and still precedes entry admission, with the callback, winner and sponsor retained
throughout. The shared waiter constructor then captures that exact winner and
sponsor and consumes the weak observer before its own admission/allocation.
Entry publication borrows the retained child. Failed entry admission can leave
an initialized empty source, exactly as before.

Ordinary results are recorded before explicit finish. Finish disposes an
untransferred weak observer, consumes child finish, releases any unused candidate,
then selected-list metadata and the original sponsor last. Child preparation
retains its existing weak/entry/charge-before-source/sponsor order. Publication
transfers entry ownership to the returned token; finishing or abandoning the
emptied containing preparation does not cancel that token or refund its charge.
No callback is invoked by preparation cleanup.

All six handoff-stage fallbacks retain every remaining field without allocator/
budget/source guards, callback, cancellation, refund or logging. Fields remain
inert between explicit candidate release and finish checkpoints too. This is
terminal retention, not Pending, physical completion, recovery custody or public
replay authority. Consuming finish still requires a qualified containing context.
Active source/list/token/batch destruction is unchanged.

The old owning candidate-preparation helper is removed; tests and production
use the same borrowed complete transaction. First-winner classification,
allocation count/layout, 64-entry ceiling, admission errors, source draining,
readiness/notification and IRQ policy stay unchanged. No new registry, owner
family, runtime ABI, wire/storage grammar, timer/deadline behavior or sponsor
replacement is introduced.

## Controls and retained cost

Existing source fixtures retain competing ordinary/platform initializers,
promotion after the ordinary candidate was prepared, heap-held first-winner
selection, losing ordinary-list refund, retired existing/fresh-source rejection,
and the non-durable pre-publication drain hint. All use the complete owner and
explicit finish now.

New serialized pre-AP controls cover:

- Six ordered finish stages: captured only; candidate only; a losing candidate
  plus a competing winner; selected winner after candidate release; prepared
  child; and published child. Five checkpoints verify heap/source-slot
  availability, weak release and original charge ownership. Published tokens
  remain charged through containing finish until actual cancellation.
- List allocation rejection beneath actual heap/source-slot guards; retired
  fresh-source rejection before allocator entry; domain-quota rejection on an
  existing source without fresh list allocation; and closed-source rejection.
  The original empty source survives entry rejection and unused fields release
  after local guards.
- Sixty-four candidates selected under a held heap guard. Sixty-three losing
  allocations release afterward, all before waiter admission; exactly one list
  remains. Sixty-four child entries publish under held heap/source-slot guards,
  then containing finishes preserve their charges until actual token cancellation.
  The direct full-source adapter rejects and refunds its staged entry.
- Abandon all six stages under held heap/source-slot guards after callback
  strong references have gone. The published token is cancelled only after
  those guards leave. Empty source close and original sponsor retirement do not
  refund abandoned admission; a fresh independent sponsor remains empty.
  Enclosing IRQ state is preserved.

The last fixture deliberately retains **six list allocations and their six
private charge holders, one original ordinary waiter charge, one completed
entry Box, five weak-only callback control blocks and the original sponsor/
account** per guest. The list count is one candidate-only list, two lists in
the competing-selection case, and one selected list in each of the final three
stages. All six parent captures and two child preparations share the same
original sponsor/account; one child is empty after publication and real token
cancellation. No registration token, linked entry, active callback, event-watch
charge, completion record, timer charge, authority, target thread/stack, user
root or physical pin is retained by this control. Retention is terminal until
guest shutdown; no live account or counter is replaced/reset.

Earlier fixture costs remain separately itemized. Full list-pool saturation
runs before these new controls, using normal admission and its captured baseline.
C16/C18 and the existing lazy waiter/IPC/CQ boundary row are updated in
[cleanup/recovery](../../reference/cleanup-recovery.md). Inner list-factory and
source/token/batch destruction, aggregate metadata/principal admission and wider
caller contexts remain separate qualification work.

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
| ARM / SMMUv3, no network | 17 | 11.518216 s |
| Intel VT-d, no network | 15 | 14.665254 s |
| AMD-Vi, no network | 15 | 16.946790 s |
| ARM / SMMUv3, security | 19 | 56.207956 s |
| Intel VT-d, security | 17 | 62.648246 s |

Each profile passes six ordered finish stages, guarded list allocation rejection,
retired/quota/closed/full controls, 64 competing heap-held selections and all 63
losing releases before entry admission. Sixty-four child entries publish under
held heap/source-slot guards; post-guard containing finish retains their original
charges until actual token cancellation. All six guarded abandonment stages
retain exactly the documented metadata and one original entry charge after
source closure and sponsor retirement. The published-stage token is cancelled
explicitly after guards, with no fabricated readiness. Existing classification/
promotion and competing-source tests use this same owner. New fixture costs and
all earlier costs remain separately itemized.

Shared waiter/callback/exit-watch and completed-entry ownership, IPC/CQ source
publication, submission/record construction, timer preparation/cancellation/
delivery and broader scheduler/IPC controls pass too. Each profile completes
64 scheduled close/exit episodes with exact exit watches (320 total). Both
security profiles complete two scoped EL0 generations. ARM retains certified
runtime migration; x86 qualifies only never-dispatched migration. These are
bounded regression observations, not sustained fairness, hostile pressure or
physical hardware qualification.

No guest panic, deadline or persistence fail-stop was observed in these five
runs. The earlier callback batch's two introduced fixture panics remain separately
preserved in
[their original evidence](evidence/2026-10-16-completion-callback-fixture-failures.json).
Passing samples do not replace those failures or attribute older incidents.

[Version-controlled evidence](evidence/2026-10-16-lazy-waiter-source-ownership.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations and new/prior fixture costs.
The commands are:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance lazy-owner-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance lazy-owner-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance lazy-owner-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance lazy-owner-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance lazy-owner-intel-security
```

All use `CATTEN_SKIP_EMBED_BUILD=1` and `CARGO_BUILD_RUSTFLAGS='-D warnings'`.
AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security adds `CATTEN_ALLOW_SANDBOX_NETWORK=1` and
HTTP/deployment ports 19080/19081 on ARM, 19180/19181 on Intel. All boot images
use the existing 128 MiB default. Tooling and staged service hashes/run inventories
match the preceding baseline; no bundled service or runtime library changed or
was rebuilt. Frozen Rust, tooling and bundle hashes are rechecked after the matrix;
documentation is finalized afterward. Published evidence is self-contained and
supplies no full-tree or platform attestation.

Earlier temporary-guard failures remain in the
[unpublished ownership evidence](evidence/2026-10-16-timer-unpublished-failures.json),
and older unattributed samples remain in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md).
Historical causality, broader caller contexts and numbered audit gaps remain open.
