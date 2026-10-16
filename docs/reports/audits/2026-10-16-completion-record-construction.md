# Completion record preparation before shared Arc construction

Date: 2026-10-16. Baseline `95b0a74`, following shared Arc payload preparation.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap

The shared Arc helper already retained the complete record payload and original
allocator during backing allocation. `Completion::new` had earlier failure
windows: its owning charge-holder constructor could refund admission while the
submitted buffer remained a function argument, and waiter-list rejection could
destroy the local allocator before that buffer. No complete owner covered the
buffer, uninstalled reservation and partial holder/list storage together.
This is a source finding, not attribution of historical panic, IPC exact-exit,
stack, persistence or NIC failures.

## Complete partial-record ownership

`record_preparation::Preparing` captures the exact buffer, operation ID, original
reservation and its captured classification before any fallible record phase.
The shared holder factory borrows the reservation, retaining it in the parent
on rejection. Successful holder construction consumes that same charge; the
parent retains its allocator while allocating a waiter list. The list callback
takes only classification, with no owning buffer or record admission argument.

The holder/list factory returns before explicit cancellation consumes its owner.
Cancellation disposes the buffer, then any unused waiter list, before releasing
either installed or uninstalled original admission. Remaining fields stay inert
between explicit disposal steps. Abandonment in reservation-only, holder-installed
and list-installed phases retains all fields without allocator, budget, registry,
callback or logging work.

Completion constructs the same private `Completion` with its original operation
and buffer, then transfers it and its original allocator directly into the shared
Arc payload helper. That helper retains the complete record through backing
allocation and performs ordinary rejection cleanup after allocator return.
Record-backing rejection still invokes the existing completion destructor to
close source entries without notification, before the allocator refunds admission.
Successful construction preserves buffer pointer/content and operation identity.
No public operation ID is refunded or reset.

Outward `WouldBlock` errors, initial state, allocation layout/count, independent
list accounting, one-allocation allowance and strong/weak lifetime policy are
unchanged. Published completion/source destruction remains active. Capture is
not an exact-root lease, physical completion or quiescence proof. There is no
new registry, authority, retry/custody interface, deadline or IRQ policy change.
Ordinary allocation/disposal still requires qualified caller guards/masks.

## Controls and retention cost

Serialized pre-AP guest controls exercise the production implementation with
private record accounts and a private operation-ID counter:

- Complete 64 preparations, checking each exact buffer pointer/content and
  operation ID. A final weak reference retains original admission after strong
  destruction; its release leaves an independently reserved successor unchanged.
- Inject holder and list allocation rejection through the ordinary constructor,
  checking original admission and availability of the heap/completion registry
  at allocation boundaries. Both outward errors remain `WouldBlock`.
- Consume a private allocator's sole allowance to force real record-backing
  rejection, without physical allocator exhaustion or a global failure flag.
  Completion cleanup closes its actual source entry; an independent token retains
  only the list, and no callback or false completion is generated.
- Reject partial holder/list preparation under held heap and completion-registry
  guards, verifying the same buffer and original reservation remain. Explicit
  post-guard cancellation checks original admission before each payload/admission
  disposal step. Fully prepared unused cancellation uses that same path.
- Abandon all three phases beneath held heap and actual completion-registry
  guards, preserving enclosing IRQ state and all original admission.

The last control deliberately retains **three original ordinary record charges,
three 16-byte buffers, two record charge holders and one empty waiter list** per
guest. That list also retains its independent ordinary list charge, list Arc and
private list charge holder. One private record account remains through its
original charges. There are no retained waiter-entry/watch/authority charges,
user roots, physical pins or real scheduler claims. No live sponsor is replaced
or counter reset. This is terminal fixture retention until guest shutdown;
earlier fixture costs are recorded separately in the evidence. These controls
run after the existing full list-admission fixture.

This qualifies partial `Completion::new` construction, not outer submission
factories, entry preparation, published Arc/Weak/source/containing destructors,
authority/namespace rollback, aggregate pressure or every wider IRQ/guard context.
Existing C16/C18 and local-boundary rows are updated in
[cleanup/recovery](../../reference/cleanup-recovery.md); no owner family is added.

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
| ARM / SMMUv3, no network | 17 | 11.386293 s |
| Intel VT-d, no network | 15 | 14.507847 s |
| AMD-Vi, no network | 15 | 20.190355 s |
| ARM / SMMUv3, security | 19 | 57.326276 s |
| Intel VT-d, security | 17 | 62.409086 s |

The new record-construction controls pass on every profile: 320 exact original
buffer/operation transfers total, original-account weak retention, holder/list
and actual record-backing rejection, payload-first cancellation, and guarded
retention across all three construction phases. Each guest deliberately retains
three original record charges and buffers plus their partial metadata, as
itemized above and in the evidence. Existing event/node preparation, cancellation
handback, callback/waiter delivery and observer-disposal controls also pass. Each completes
64 scheduled IPC close/exit episodes with exact exit watches (320 total).
Both security profiles complete two scoped EL0 generations. ARM retains
certified runtime migration; x86 qualifies only never-dispatched migration.
These are bounded regression observations, not sustained fairness, hostile
pressure or physical hardware qualification. No failed native sample, panic,
deadline or persistence fail-stop was observed in this batch.

[Version-controlled evidence](evidence/2026-10-16-completion-record-construction.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations, and new and prior fixture costs.
The five commands use `--smp 4 --fresh-storage --timeout 180`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance record-construction-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance record-construction-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance record-construction-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance record-construction-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance record-construction-intel-security
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
