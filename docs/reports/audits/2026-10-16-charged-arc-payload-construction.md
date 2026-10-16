# Shared charged Arc payload construction

Date: 2026-10-16. Baseline `0f27dbf`, following event/shared-holder preparation.
This continues R7-1/2 and C16/C18/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.

## Source gap and ownership

The shared charge-holder constructor already kept its original reservation
outside allocation. `ChargedAllocator::try_arc<T>` still passed an owning `T`
into `Arc::try_new_in`. Its outer allocator reference preserved admission until
payload destruction, but allocation rejection could destroy resource-bearing
payload fields inside the allocator callback before returning to the helper.
Completion records, observer lists and timer cancellation state use this common
path. This is a source finding, not attribution of historical panics or IPC,
stack, persistence or NIC incidents.

`PreparingArc<T, C>` now retains the complete payload, original allocator and
any empty `Arc<MaybeUninit<T>, ChargedAllocator<C>>` behind `ManuallyDrop`.
Allocation receives only an allocator clone; no owning payload crosses that
boundary. A rejected allocation returns before explicit cancellation destroys
the payload, then any empty backing, then the preparation's original allocator.
Original admission therefore remains through payload and backing disposal.

Successful private completion checks unique, unexposed storage, initializes it
once and consumes the same backing with `assume_init`. It releases only the
extra preparation allocator reference: the initialized Arc still retains the
same admission. Initialization performs no heap allocation/deallocation or
payload destruction. No uninitialized strong/Weak alias escapes.

Abandonment before or after empty allocation retains all fields, with no
allocation, deallocation, refund, callback, lock or logging. This is terminal
retention, not Pending, an owner-preserving retry interface, physical cleanup or
recovery custody. Ordinary construction/cancellation still requires qualified
containing guards and IRQ policy; this helper cannot remove its caller's guards.

Payload layout, one-allocation allowance across clones, failed Global-allocation
reset, outward errors and final strong/weak accounting remain unchanged.
Published Arcs and allocator references retain their existing active final-release
policy. There is no new registry, reservation, timer queue publication, readiness
contract, deadline change, ABI or physical/replay authority.

## Controls and scope limits

Four new host tests exercise the production generic implementation:

- Rejection returns before a resource-bearing payload and its fields drop; the
  callback releases its guard first and admission remains throughout cleanup.
- Both preparation phases are abandoned beneath a held guard. Heap tracing
  records zero allocation/deallocation; payload and original charge remain.
- Explicit unused-storage cancellation checks payload disposal before backing
  release and original admission refund.
- Successful private initialization beneath a guard performs zero heap calls
  and no payload destruction, then final strong/weak release refunds once.

The existing actual rejecting-allocator control now allocates empty storage.
The public helper's resource-bearing success/128 Weak-alias control, concurrent
one-backing admission and concurrent final-reference controls remain. The two
new host-only negative preparations retain two synthetic charge counters, two
payloads, two private holders and one empty payload Arc until process shutdown;
they add no kernel reservation. Earlier holder controls retain two additional
synthetic charges and one empty holder Arc. Guest fixture retention is unchanged
and recorded in the evidence.

Native runs exercise the shared production helper through completion records,
observer lists and cancellation states, with the existing guarded timer/event
and completion controls. No new guest fixture or retention charge is introduced.
This qualifies the shared helper, not payload assembly before entry, outer list
factories, published Arc/Weak finalization, source/containing destructors,
aggregate pressure or every wider guard/IRQ context. Existing owner-family and
local-boundary rows are updated in [cleanup/recovery](../../reference/cleanup-recovery.md).

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
| ARM / SMMUv3, no network | 17 | 11.388174 s |
| Intel VT-d, no network | 15 | 14.224678 s |
| AMD-Vi, no network | 15 | 18.649711 s |
| ARM / SMMUv3, security | 19 | 57.762432 s |
| Intel VT-d, security | 17 | 64.687501 s |

All profiles pass the existing event/node preparation, cancellation handback,
callback/waiter delivery and observer-disposal controls. Each also completes
64 scheduled IPC close/exit episodes with exact exit watches (320 total).
Both security profiles complete two scoped EL0 generations. ARM retains
certified runtime migration; x86 qualifies only never-dispatched migration.
These are bounded regression observations, not sustained fairness, hostile
pressure or physical hardware qualification. No failed native sample, panic,
deadline or persistence fail-stop was observed in this batch.

[Version-controlled evidence](evidence/2026-10-16-charged-arc-payload-construction.json)
records exact commands/environments, baseline and reviewed source hashes,
artifact/UART hashes, concise observations and unchanged guest fixture costs.
The five commands use `--smp 4 --fresh-storage --timeout 180`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance arc-payload-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance arc-payload-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance arc-payload-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance arc-payload-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance arc-payload-intel-security
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
