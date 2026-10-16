# Capability-backed completion preparation outside COMPLETIONS

Date: 2026-10-16. Baseline `92384c02`, following checked operation-ID exhaustion.
Scoped R7-1/2 and C16/G1/G4 work. SEC-07/18 remain partial; the numbered
findings remain 29 corrected within scope, six partial and three open. The
18 cleanup families and seven gates are unchanged. Historical IPC exit,
stack, persistence and NIC failures remain unattributed by this work.

## Observed boundary

Capability-backed plain/buffer/event-watch submission constructed charged
records, allocator/control-block backing, waiter lists and shared authority
metadata under `COMPLETIONS`. Capability timers additionally prepared event
cancellation backing, queue nodes and producer observers there. Partial
construction failure could destroy those resources under the registry too.
Detached preparation had already moved outside it; that did not qualify this
separate authority-bearing path.

Canonical public submissions now use `completion/capability_preparation.rs`.
The old guarded construction branches and their standalone record-reservation
adapter are removed. No public capability, register, wire or storage format
changes; existing observer-allocation and operation-ID fixtures use this path.

## Capture, preparation, hidden reservation and publication

Capture retains the exact namespace root, original record/timer/watch account
Arcs, submission capacity and kernel-derived platform classification. It checks
retirement and preserves existing CQ backlog reconciliation and live-capacity
checks. Capability-backed operations do not require a CQ or bind to one queue
instance; their existing default-CQ/individual observation semantics remain.

One `Preparing(ManuallyDrop<Resources>)` owns the complete operation. Outside
the registry it consumes a checked operation ID, reserves original record and
optional watch admission, constructs the charged record/list and prepares
shared authority storage with `PreparedReservation::try_new`.

A short registry hold revalidates root, original account identities, capacity
and retirement before hidden admission. The shared
`PreparedReservation::reserve_captured` only relinks preallocated authority
metadata; its caller must hold its payload registry and validate its captured
generation. It cannot acquire lifecycle or dispose unused storage there.
`Reservation` retains the actual authority namespace object after admission.
Hidden authority counts against its original shared budget but is not usable;
no completion payload or live submission slot has been published yet.

Timer admission, cancellation backing, prepared node and producer observer are
then constructed outside `COMPLETIONS`. Every partial field remains in the same
owner. Observer allocation succeeds before callback registration/installation.
The record remains private while its timer fields are initialized.

Final publication borrows this complete owner. Under the registry it repeats
exact root/account/capacity/retirement validation, then uses existing borrowed
`capability::publish_batch` before installing the record and incrementing live
capacity. Failure leaves hidden reservation, payload, optional watch and event
with their original owner through guard exit. A replaced/missing completion
namespace or retired root returns `UnknownAddressSpace`; shared authority
rejection returns the existing `WouldBlock`. A replaced authority namespace
cannot make an old token publish into a reused numeric capability.

Both public adapters explicitly finish every ordinary success/error after local
guards leave. Finish disposes unused authority storage, releases any remaining
hidden reservation against its captured namespace, and drops the complete
private payload. No producer is enqueued after rejection. Abandonment retains
all fields without registry access, cancellation, allocation, destruction or
logging. It is terminal retention, not a public retry/custody interface.
Ordinary rejected preparation is unstarted rollback, not Pending or proof of
physical cleanup. Capture carries no new physical-root lease or pin.

## Timer deadline and teardown ordering

Capability timers capture one saturating absolute deadline at entry, before
record/authority/event preparation. `TimerEvent::charged_at` carries it through
publication and once-only post-guard enqueue. Preparation delay does not restart
the budget, and an expired deadline remains expired. The existing relative
constructor shares its cancellation-allocation body while keeping its former
deadline-capture point; scheduler minimum-sleep rebasing is unchanged.

Before accepted enqueue the preparation drops its extra record reference
outside the registry. If namespace teardown won, the final record destruction
cancels the still-prepared event. The node keeps its original event charge
until enqueue observes cancellation and discards it. Retaining that extra
reference through enqueue would delay teardown's cancellation, so this boundary
is explicit. Old work cannot reach a replacement namespace's CQ.

## Native controls

The existing synchronous callback/admission fixture now runs these controls:

- Record-construction and authority-storage factories reenter `COMPLETIONS`.
  Injected record-construction rejection and the real shared authority-node
  one-shot rejection return original record/watch/shared charges. These are
  controlled allocation boundaries, not whole-heap exhaustion.
- Prepared node and timer-observer allocation reenter the registry; separate
  node and observer failures leave no payload, live capability or queued timer.
  A successful unused preparation finishes with the same original charges.
- Before hidden reservation, completion-namespace replacement/removal and real
  root retirement reject after allocating authority storage, disposing it only
  after registry exit. Old record/watch accounts return to zero.
- After hidden reservation, two complete owners (buffer/watch and timer) lose
  to same-capacity namespace replacement, namespace removal, real root
  retirement, or authority-namespace replacement with the exact old numeric
  capability reused. All **eight borrowed publication rejections** run with
  the actual heap lock held. Payload, reservation, event and original charges
  remain owned and no authority becomes live. Post-guard finish cannot alter
  the replacement's staged token; that token still publishes successfully.
- An accepted timer loses its namespace before enqueue, retaining original
  record/event admission until explicit handback and cancelled insertion. A
  replacement CQ receives no predecessor result.
- A separately accepted timer preserves absolute deadline zero through event
  preparation/publication, then actually completes when local events run.
  Public allocation-rejection/recovery, strong/weak retention, cancellation,
  watch/worker and scoped EL0 controls continue through the same new path.

The fixture checks its enclosing IRQ state is unchanged. It does not enable
IRQs, add polling or introduce a new readiness source. Heap-held rejection
checks unused-owner retention; it does not claim successful map insertion or
every backlog path is allocation/destruction-free.

## Execution results

The host runner passes **499 Rust tests, zero failed, four ignored**, plus its
signing/tooling/Python controls. Strict kernel Clippy passes on both custom
AArch64 and x86 targets with `-- -D warnings`. Workspace
`cargo fmt --all -- --check` and `git diff --check` pass.

Runtime Rust and native fixtures were frozen before this completed five-profile
matrix. Every profile uses four LPs, fresh storage, a 180-second guest budget,
denied Rust warnings, assembly-section checks and automatic all-LP failure
capture. Each completed with runner exit zero and diagnostic result `complete`.

| Profile | New preparation controls | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.607762 s | 17 passed, zero failed/pending at 11.051620 s |
| Intel VT-d, no network | Passed at 1.260088 s | 15 passed, zero failed/pending at 14.885385 s |
| AMD-Vi, no network | Passed at 1.787711 s | 15 passed, zero failed/pending at 16.151978 s |
| ARM / SMMUv3, security | Passed at 1.289093 s | 19 passed, zero failed/pending at 56.079341 s |
| Intel VT-d, security | Passed at 1.227330 s | 17 passed, zero failed/pending at 61.677529 s |

Together the new fixtures exercise 40 heap-held late publication rejections,
15 identity rejections before hidden reservation and 20 controlled construction
failures, plus successful unused preparation and deadline/teardown controls.
Each profile also completes 64 scheduled IPC close/exit episodes with exact
exit watches. Both security profiles pass two independent scoped EL0 probe
generations through the new completion path. No failed native sample, panic or
Raft persistence fail-stop was observed in these five runs; historical incidents
remain separate. ARM retains certified runtime migration; x86 still qualifies
only never-dispatched migration.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance capability-prepare-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance capability-prepare-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance capability-prepare-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance capability-prepare-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance capability-prepare-x86-security
```

AMD reuses the completed Intel no-network kernel with
`CATTEN_SKIP_KERNEL_BUILD=1`. Security profiles use
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment ports 19080/19081 for ARM,
19180/19181 for x86. Full staged service inventories match the preceding batch;
no service sources or bundles changed.

[Version-controlled evidence](evidence/2026-10-16-capability-completion-preparation.json)
records reviewed changed-Rust hashes, commands, kernel/boot-image/initial-disk/
final-UART hashes, concise observations and unchanged service inventories. These
inline results and the JSON are the published evidence; temporary logs and
ignored diagnostic directories are not required to read it. Reviewed per-file
hashes include new untracked modules that dirty-diff manifests omit.
Documentation/evidence was finalized afterward. This is not a full-tree or
physical-platform attestation, nor an integrated hostile-pressure proof.

## Remaining inventory and qualification

Fresh completion registry/payload `BTreeMap` nodes remain infallibly allocated
under serialization. Existing backlog reconciliation, authority close/removal,
terminal timer cleanup, timer-queue/comparator and arbitrary enclosing caller
guard/IRQ contexts remain separate. General heap/principal admission and
integrated hostile-pressure progress remain R7/R18 acceptance work.

Source review also records that anonymous timer cancellation IDs still use
wrapping `NEXT_TIMER_EVENT_ID.fetch_add`; checked operation IDs do not cover
that independent queue-cancellation identity. Its exhaustion behavior needs a
separate bounded correction and controls. No finite native matrix exercises
64-bit exhaustion. This observation attributes no historical panic.

The shared capability-admission, completion record/timer contracts and LaTeX
completion chapter record this boundary. C16/G1/G4 is extended without adding
a family, separate metadata ticket, physical completion proof or replay owner.
