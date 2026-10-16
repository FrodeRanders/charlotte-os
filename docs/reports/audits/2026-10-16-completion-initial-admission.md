# Initial capability-completion admission ownership

Date: 2026-10-16. Baseline `d486d06`, following complete record construction.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and qualified scope

`Completion::new` already retained its complete buffer/reservation through
holder/list/backing allocation. Its containing capability-submission owner had
earlier gaps: the buffer remained outside that owner until record construction,
and record admission was a local variable. Rejected watch admission could
therefore refund the local record reservation before disposing the submitted
buffer. Its record factory also received owning buffer/charge arguments, allowing
injected rejection to destroy them inside the callback.

This batch qualifies those initial captured-target, identity, record/watch and
record-factory phases. It is a source finding, not attribution of historical
panic, exact-worker-exit, stack, persistence or NIC failures. It does not qualify
every target-capture, timer/observer factory, published destructor or outer
IRQ/guard context.

## One containing owner

The existing `capability_preparation::Preparing` now captures its target and
original buffer before attempting operation-ID or quota admission. It records
the assigned identity and original record reservation in the same inert owner.
Watch rejection returns with that reservation and buffer intact. ID and record
rejection likewise leave all previously captured state owned; no charge is
invented for a buffer whose admission has not succeeded.

Record factories now borrow this complete transaction instead of taking owning
buffer/charge arguments. The ordinary factory directly transfers those fields
into the existing complete-record constructor before its first fallible phase.
That inner owner and the shared Arc helper retain the original payload and
admission through holder/list/backing rejection; the outer target and watch
charge remain live throughout. Successful construction preserves the same
buffer and assigned operation. There is one current factory contract, without
an owning compatibility branch.

Ordinary error calls the existing explicit finish after local guards and factory
return. Unused metadata/authority/event cleanup retains its existing ordering;
for an unconstructed record, finish then destroys the original buffer before
refunding its early record charge. This is unstarted rollback before publication,
not a public retry owner. A quota failure cannot prove completion or authorize
replay of accepted work elsewhere.

Abandonment in captured-buffer, record-admitted and watch-admitted phases retains
every field and original account, with no deallocation, budget refund, callbacks,
registry or logging work. This remains terminal retention, not Pending, physical
quiescence or recovery custody. Ordinary finish still needs qualified caller
guards/masks; existing active containing-field destruction is not made universally
safe by this early ownership correction.

Operation-ID ordering/exhaustion, quota limits, captured classification, outward
errors, allocation count/layout, hidden capability admission, exact namespace/
account/capacity revalidation, signed ABI and timer deadline behavior remain
unchanged. Rejected assigned IDs are never refunded or reset. Capture is no root
lease or physical cleanup proof; no new registry, owner family or wait source.

## Native controls and retained cost

Serialized pre-AP controls use bounded fixture namespaces and private operation-ID
counters, never resetting the live counter or saturating shared kernel sponsors:

- Ordinary identity exhaustion, record-quota rejection, watch-quota rejection
  and borrowed factory rejection return `WouldBlock`, leave live/CQ state empty
  and publish no authority. Quota pressure uses only that fixture's own original
  accounts. Admitted rejected identities remain consumed.
- The rejecting borrowed factory holds the actual heap and completion registry
  while checking its retained buffer, record charge and watch charge. Cleanup
  occurs only after that callback and its guards return.
- Actual watch rejection under held heap/registry guards retains the exact
  buffer pointer/content and original record charge. Explicit post-guard finish
  checks context and original charges before buffer and reservation disposal.
- Sixty-four borrowed factory transfers complete real capability publication,
  completion observation and close. Every step checks the same operation and
  original buffer pointer/content. All original charges return; the private
  counter reaches its permanent exhaustion boundary.
- Captured-only, record-admitted and watch-admitted owners are abandoned under
  held heap and actual completion registry guards. They retain their original
  charges across namespace replacement while the successor's accounts remain
  empty. The enclosing IRQ state is preserved.

The last control deliberately retains **three 16-byte buffers, two ordinary
record charges and one ordinary watch charge** per guest. Three captured target
values retain the original record/watch/timer account Arcs (one account in each
family), capacity/classification and the fixture identity. There is no retained
completion Arc, charge holder, waiter list/entry, CQ backing, timer charge,
capability reservation, user root or physical pin from this control. Earlier
fixture retention is itemized separately in the evidence. This is terminal
retention until guest shutdown, never recovery custody or a physical retry owner.

Existing capability metadata, hidden-authority replacement, close-before-enqueue,
backlog refund, record construction and shared allocation controls remain.
[Cleanup/recovery](../../reference/cleanup-recovery.md) updates the existing
C16/C18 families and containing publication boundary. Target capture, timer/event/
observer factories, published reference/source destruction, aggregate metadata
admission and wider IRQ/guard qualification remain separate work.

## Validation and evidence

The host runner passes **509 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting,
diff whitespace and relative documentation links pass.

Six reviewed Rust files were frozen before the five-profile matrix. Every
profile uses four LPs, fresh storage, a 180-second guest budget, denied Rust
warnings, assembly-section checks and automatic all-CPU failure capture.
All runners exit zero with diagnostic result `complete`, zero failed/pending:

| Profile | Deferred tests passed | Completion |
| --- | --- | --- |
| ARM / SMMUv3, no network | 17 | 11.577507 s |
| Intel VT-d, no network | 15 | 15.078593 s |
| AMD-Vi, no network | 15 | 21.159481 s |
| ARM / SMMUv3, security | 19 | 57.220404 s |
| Intel VT-d, security | 17 | 62.028162 s |

The new initial-admission controls pass on every profile: identity/record/watch/
factory rejection, actual guard-held watch rejection with retained buffer/charge,
320 borrowed factory publication/result cycles total, and all three guarded
early abandonment phases across namespace replacement. New retained costs are
itemized above and in the evidence. Existing record-construction, event/node
preparation, cancellation handback, callback/waiter delivery and observer-disposal
controls also pass. Each profile completes 64 scheduled IPC close/exit episodes with exact exit watches (320 total).
Both security profiles complete two scoped EL0 generations. ARM retains
certified runtime migration; x86 qualifies only never-dispatched migration.
These are bounded regression observations, not sustained fairness, hostile
pressure or physical hardware qualification. No failed native sample, panic,
deadline or persistence fail-stop was observed in this batch.

[Version-controlled evidence](evidence/2026-10-16-completion-initial-admission.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations, and new and prior fixture costs.
The five commands use `--smp 4 --fresh-storage --timeout 180`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance initial-admission-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance initial-admission-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance initial-admission-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance initial-admission-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance initial-admission-intel-security
```

All use `CATTEN_SKIP_EMBED_BUILD=1` and `CARGO_BUILD_RUSTFLAGS='-D warnings'`.
AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; the two no-network
kernel ELF hashes match. Security profiles add `CATTEN_ALLOW_SANDBOX_NETWORK=1`
and HTTP/deployment ports 19080/19081 on ARM, 19180/19181 on Intel.
All staged service hashes and run inventories match the prior baseline; no
service/runtime library was edited or rebuilt. Frozen Rust hashes are rechecked
after the matrix; documentation is finalized afterward. Published results do
not depend on temporary logs or ignored artifacts and are no full-tree or
platform attestation.

The earlier introduced temporary-guard failures remain separately preserved in
[unpublished ownership evidence](evidence/2026-10-16-timer-unpublished-failures.json).
Older unattributed samples remain in the
[backlog refund report](2026-10-16-completion-backlog-refunds.md).
These passes do not establish their causes or close the wider audit gaps.
