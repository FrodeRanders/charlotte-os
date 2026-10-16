# Completed observer-entry preparation ownership

Date: 2026-10-16. Baseline `cefe7211`, following waiter preparation ownership.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and scope

Partial entry construction and its callback/exit-watch/waiter containing owners
already retained complete fields on abandonment. The completed shared
`PreparedRegistration<C>` still used ordinary field destruction: dropping a
standalone, fully initialized preparation could free its node, final weak callback
allocation and original charge beneath an enclosing guard.

This batch qualifies completed private preparation abandonment and explicit
ordinary disposal. It does not change active `Registration`, linked `Entry`,
`NotificationBatch` or source destruction, callback delivery, or every caller's
outer IRQ/guard context. This is a source finding, not attribution of historical
exact-exit, stack, persistence or NIC failures.

## Completed owner and consuming finish

`PreparedRegistration<C>` now retains its optional complete entry behind
`ManuallyDrop`, with a must-use annotation and consuming `finish`. Private entry
initialization still consumes the same empty Box exactly once; the complete
watch/waiter charge remains outside that Box. Abandonment retains node backing,
weak callback reference and original admission without allocator/budget/list
guards, callbacks, cancellation, refund or logging.

Publication still borrows the preparation. Closed/full/ID-exhausted rejection
leaves its complete entry intact. Successful publication takes the entry under
the existing list lock and transfers it once to the actual list/registration;
the empty preparation has no entry charge to release. No source identity,
capacity, checked registration ID or notification contract changes.

The direct list registration adapter records its publication result, explicitly
finishes after the local list guard leaves, then returns that result. Callback,
exit-watch and waiter containing-owner finishes now consume this child finish
instead of dropping the child. Their transient/source/sponsor/target disposal
orders stay unchanged. Existing construction fixtures likewise explicitly finish
both unused complete entries and emptied preparations after local guards.
There is one current private disposal contract; implicit preparation cleanup
is no longer available.

Explicit finish disposes the optional private entry in its qualified caller
context, preserving the existing weak/node-before-charge field order. The
shared `Entry` destructor itself is unchanged; its linked/batch/source and wider
enclosing contexts remain separate qualification work. No generalized interrupt-
safe destructor, partial physical cleanup receipt or recovery adapter is claimed.
An abandoned prepared entry has no list/token ownership and is not a registered
waiter, a Pending operation or a public retry owner.

Errors, original charge classification, quota/capacity, heap allocation count
and layout, callback/IRQ policy, public observation interfaces and timer/deadline
behavior are unchanged. No new registry, owner family, runtime ABI or wire/storage
grammar is introduced.

## Controls and retained cost

New serialized pre-AP controls use both actual production charge types:

- Prepare 64 watch and 64 waiter entries. Explicitly finish 32 unused entries
  of each type, checking that actual heap/list guards are available and original
  charges/weak references remain until disposal.
- Publish the other 32 of each type under a held heap guard, then explicitly
  finish emptied preparations after guard exit without refunding published
  entry charges. Real list publication still acquires its independent list lock.
- For each type, full and closed-source publication under a held heap guard
  preserves its unused entry/charge until post-guard finish. The direct adapter
  also rejects each full/closed case and refunds its staged entry explicitly.
- Detached batch discard retains charges until actual discard; retained tokens
  remain empty afterward. Final weak-only callback allocations are disposed by
  explicit finish outside heap/list guards. No callback is invoked.
- Abandon one completed watch entry and one completed waiter entry under
  simultaneously held heap and actual list-state guards after callback strong
  references have gone. Closing their empty sources and retiring the waiter
  sponsor leaves original admission retained, while both source allocations
  are released. Preserve enclosing IRQ state.

The last control deliberately retains **one original ordinary watch charge,
one original ordinary waiter charge, two completed entry Boxes and two weak-only
callback control blocks** per guest. Each charge retains its original account.
Unlike the containing-owner fixtures, this shared preparation owns no list or
separate sponsor reference. New retained list charges, private list charge
holders, registration tokens, linked entries, active callbacks, completion
records, timer charges, authority, target threads/stacks, user roots and physical
pins are all zero. Retention is terminal until guest shutdown. Earlier costs
remain separately itemized; no live sponsor or pool counter is replaced/reset.

Full list-pool saturation precedes these controls. Existing entry-construction,
waiter, callback and exit-watch owner controls retain their original charge/list
costs and now share the consuming completed-entry boundary. C16/C18 and the
existing lazy waiter/IPC/CQ boundary row are updated in
[cleanup/recovery](../../reference/cleanup-recovery.md). Source acquisition,
active linked/token/batch/source destruction, aggregate metadata/principal
admission and broader caller contexts remain separate work.

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
| ARM / SMMUv3, no network | 17 | 11.191776 s |
| Intel VT-d, no network | 15 | 20.194602 s |
| AMD-Vi, no network | 15 | 15.250004 s |
| ARM / SMMUv3, security | 19 | 55.811592 s |
| Intel VT-d, security | 17 | 67.535121 s |

Per profile, the new controls initially prepare 128 entries across both actual production
charge types, explicitly finish 64 unused entries, publish 64 beneath a held
heap guard and finish their emptied preparations afterward. Both full/closed
retaining rejections and both direct-adapter rejection paths pass for each type.
Detached discard releases 64 entries while their retained tokens remain empty;
explicit finish releases both final-weak-only callbacks. The two completed
abandonment controls retain their original charges/backing after source closure
and waiter-sponsor retirement without retaining either list. New fixture costs
and all earlier costs remain separately itemized.

Existing entry construction, containing waiter/callback/exit-watch ownership,
IPC/CQ source publication, submission/record construction, timer preparation/
cancellation/delivery and broader scheduler/IPC controls pass too. Each profile
completes 64 scheduled close/exit episodes with exact exit watches (320 total).
Both security profiles complete two scoped EL0 generations. ARM retains certified
runtime migration; x86 qualifies only never-dispatched migration. These are
bounded regression observations, not sustained fairness, hostile pressure or
physical hardware qualification.

No guest panic, deadline or persistence fail-stop was observed in these five
runs. The earlier callback batch's two introduced fixture panics remain separately
preserved in
[their original evidence](evidence/2026-10-16-completion-callback-fixture-failures.json).
Passing samples do not replace those failures or attribute older incidents.

[Version-controlled evidence](evidence/2026-10-16-completed-observer-entry-ownership.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations and new/prior fixture costs.
The commands are:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance entry-owner-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance entry-owner-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance entry-owner-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance entry-owner-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance entry-owner-intel-security
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
