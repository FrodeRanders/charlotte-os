# Detached timer terminal disposal

Date: 2026-10-16. Baseline `6c87dc9b`, following CQ payload preparation.
Scoped R7-1/2 and C16/G1/G4 work. SEC-07/18 remain partial; the 38 findings
remain 29 corrected within scope, six partial and three open. Eighteen cleanup
owner families and seven gates are unchanged. Historical IPC exit, stack,
persistence and NIC failures remain unattributed by this batch.

## Observed boundary

`complete_detached` removed its record and published the effective result under
`COMPLETIONS`. Moving its record charge into `post_to_cq` left the remaining
record fields to implicit destruction before registry unlock. For a timer,
these include the concrete producer-observer Arc and `TimerCancellation`.
Cancellation can enter the available local timer queue, free its node and
release cancellation backing; this was more than a scalar counter update.

A direct ring result also destroyed its passed record charge inside
`post_to_cq`. A full ring instead retained that charge in the backlog. The
missing-CQ branch discarded the whole record under the registry. Afterwards,
the old numeric `signal_cq` helper resolved the namespace/CQ again; replacement
between publication and delivery could direct that wake at a successor source.
These are source observations, not historical failure attribution.

## Complete logical retirement owner

`retire_detached` still removes the exact globally nonreused operation and
publishes its effective result under one registry hold. `post_detached` returns
`RetiredDetached`, retaining the entire remaining `DetachedOp`, original
admission and exact published CQ source. The public adapter consumes that owner
only after the registry leaves, disposing the record before notification.
The record is already terminal and no longer addressable; repeated completion
or cancellation returns `UnknownCap` without a second CQ entry or wake.

Record admission stays in an optional field of the existing record until an
actual backlog entry needs it. `post_to_cq` invokes its ownership-transfer
closure only when retaining an entry; both callers use allocation-free scalar
marker/charge moves. A direct ring result keeps the original record charge
through post-guard destruction. A backlogged result consumes it exactly once
into the retained entry and keeps its live submission slot until delivery or
discard. A missing CQ leaves the charge with the retirement owner until its
post-guard disposal. Existing backlog flush accounting is unchanged.

The concrete timer observer and cancellation owner stay in that same record;
no dynamic observer replacement, reconstructed handles, separate metadata ticket,
registry or custody family is added. Field destruction order and existing local
versus busy/remote cancellation policy are preserved. A manual distant-timer
completion can cancel/remove its available local event before notifying the CQ.
If the timer itself is firing, cancellation sees its already-borrowed queue and
only flags the event. That callback still owns the event/node and producer Arc;
its separate timer charge survives CQ notification until queue processing ends.
Record retirement is not event backing release or physical quiescence.

Notification drains only the source captured at publication. Replacing the CQ
between retirement and disposal cancels its old registrations through normal
replacement; the old completion cannot consume a new successor waiter. The
now-unused numeric-only signaling helper is removed, with no compatibility
branch or public ABI change.

This is terminal logical disposal, not unstarted rollback, owner-preserving
physical retry or custody. No pending buffers are observed, no forced IRQ enable,
wait/deadline policy, signed format or wire protocol changes. Neither an absent
record nor a wake grants permission to replay accepted work.

## Context inventory and remaining work

| Entry | Qualified here | Remaining boundary |
| --- | --- | --- |
| Detached terminal hook, manual/cancel/firing producers | Exact record removal and CQ publication stay serialized; complete timer-bearing payload and direct-result charge leave the registry before disposal; notification uses the captured source. | Registry-map node removal/deallocation remains under the registry. Outer producer/IRQ contexts and the timer queue borrow are separate. |
| Backlogged detached result | The original charge moves only into actual retained storage; its slot survives terminal record disposal. | Existing backlog draining/discard and every containing metadata context are not newly qualified here. |
| Missing-CQ terminal branch | Whole remaining payload/charge retained until unlock, no fabricated source or wake. | This defensive branch is exercised with a private serialized fixture, not a claim of physical root quiescence. |
| Timer/record construction and capability-backed timer fields | Behavior unchanged. | Admission/allocation/partial errors under submission guards, capability timer installation/cancellation, authority/map metadata and broader outer-context qualification remain open. |

This updates the existing C16 row and local boundary table in
[cleanup/recovery](../../reference/cleanup-recovery.md). The owner is private
and consumed once; no general destructor-safety, physical-release or retry
proof follows from these local controls.

## Focused native controls

The existing callback-admission fixture exercises real admitted timer nodes,
concrete producer observers, original record/event accounts and CQ sources:

- Three payload splices cover direct delivery, full-ring backlog and missing
  CQ. With the heap and actual registry held, the production posting helper
  returns the complete remaining record. The timer observer stays alive, its
  event stays charged, and the direct/missing charge remains with that record;
  the backlog charge has moved only into its retained entry. Disposal runs
  afterwards, removes the available local event and releases only the
  appropriate charge. Backlog delivery then releases its retained record.
- Map removal can free a registry node, so the heap-held payload controls
  extract that node outside the heap guard. The fixture namespace is private;
  production removal and posting remain under one registry hold. The test
  explicitly does not qualify map removal as allocation/destruction-free.
- Three public producer paths cover manual terminal completion of a distant
  timer, explicit timer cancellation, and a real zero-deadline timer callback.
  CQ callbacks reenter the registry and inspect terminal absence, exact
  operation/cookie/result and zero record charge. The active firing callback
  observes its event charge and producer reference still alive; manual/cancel
  notification observes their completed local disposal. After queue processing,
  all event charges and producer strong references have left. Duplicate
  completion/cancellation rejects and does not notify again.
- A forced ordering replaces the CQ after real serialized publication but
  before record disposal/notification. The successor waiter remains registered
  and charged and receives zero predecessor wakes, including after duplicate
  completion. Original timer and record charges release with the retired owner.
  This is a forced phase ordering, not an arbitrary destructor-reentry test.

The zero-deadline setup uses a preserving local mask until its interest is
installed and explicitly processes the real timer queue. All probes restore
entry IRQ state. Sponsors/accounts are prepared privately, never substituted
on a live shared sponsor or saturated to imply exact per-thread accounting.
These are local ownership/order controls, not physical allocator exhaustion,
exhaustive cross-LP races or whole-node progress measurements. Existing timer
observer-allocation rollback/recovery, original weak-lifetime admission,
cancellation churn, CQ saturation and captured-object checks remain enabled.

Host validation passed **494 Rust tests, zero failures, four ignored**, plus
signing, boot/diagnostic/runtime-input/tooling/object-store checks and cluster-sign
self-tests. Strict Clippy passed both custom targets with
`--no-default-features --features acpi -- -D warnings`. Formatting, whitespace
and 436 local Markdown link checks passed. New local owner/guard controls
execute in native guests, not host mocks of kernel locks.

## Native matrix and evidence

| Profile | Detached disposal probe at | Exact close/exit episodes at | Final passed / failed / pending at |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 2.234560 s | 64 at 10.238474 s | 17 / 0 / 0 at 12.152534 s |
| Intel VT-d | 1.426258 s | 64 at 14.004037 s | 15 / 0 / 0 at 14.969421 s |
| AMD-Vi | 1.406193 s | 64 at 14.036764 s | 15 / 0 / 0 at 14.998103 s |
| ARM security | 1.320947 s | 64 at 8.746064 s | 19 / 0 / 0 at 55.584179 s |
| Intel security | 1.283358 s | 64 at 14.101140 s | 17 / 0 / 0 at 61.789383 s |

All five runners exited **0**, diagnostic result `complete`. No failed native
sample, panic or Raft persistence fail-stop was observed in this matrix. Five
composite detached probes completed **35 forced controls** covering direct,
backlogged and missing-CQ posting, manual/cancel/firing producers and exact
successor-source fencing. Active timer event charges survived their callback,
while record charges followed their respective record/backlog lifetimes. Existing fixtures
completed **320 exact close/exit episodes, 20 first-entry controls, 20 forced
park controls and 15 real EL0 growth modes**. Every profile reported 192 timer
wakes preserving established LP affinity. ARM exercised certified migration;
x86 kept its explicit never-dispatched gate and skipped started-context runtime
moves. These bounded observations do not establish whole-node progress or the
cause of historical intermittent failures.


Runtime Rust and native probes were frozen before the completed matrix;
documentation/evidence was finalized afterward. Each profile uses four LPs,
fresh storage, `-D warnings`, a 180-second guest budget and automatic all-LP
failure capture. No service source changed; full staged service inventories
match the preceding batch byte for byte.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance detached-disposal-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance detached-disposal-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance detached-disposal-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance detached-disposal-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance detached-disposal-x86-security
```

AMD reuses the completed Intel no-network kernel with
`CATTEN_SKIP_KERNEL_BUILD=1`. Security profiles use
`CATTEN_ALLOW_SANDBOX_NETWORK=1`, HTTP/deployment ports 19080/19081 for ARM and
19180/19181 for x86. Assembly-section checks remain enabled.

[Version-controlled evidence](evidence/2026-10-16-detached-timer-disposal.json)
records reviewed changed-Rust hashes, kernel/boot-image/initial-disk/final UART
hashes, concise profile observations and full unchanged service inventories.
These inline results and the checked-in JSON are the published evidence;
temporary UARTs and ignored diagnostic captures are not needed to read it.
Per-run dirty-diff manifests omit untracked additions; reviewed file hashes and
the resulting commit anchor those new probes. This is not a full-tree source
manifest, physical hardware snapshot or historical-failure reproduction.

Next work remains timer/record preparation, capability-backed timer
installation/cancellation, registry/authority metadata and outer producer/queue/
IRQ contexts. SEC-07/18, aggregate principal/heap admission, hostile pressure
progress and historical intermittent failure attribution remain open.
