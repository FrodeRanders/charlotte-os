# Detached submission preparation outside COMPLETIONS

Date: 2026-10-16. Baseline `f2ec7c7c`, following detached timer terminal disposal.
Scoped R7-1/2 and C16/G1/G4 work. SEC-07/18 remain partial; the 38 findings
remain 29 corrected within scope, six partial and three open. Eighteen cleanup
owner families and seven gates are unchanged. Historical IPC exit, stack,
persistence and NIC failures remain unattributed by this batch.

## Observed boundary and scope

Detached plain submission reserved record admission under `COMPLETIONS`.
Detached timer submission additionally reserved event admission, allocated
cancellation backing and the prepared node, allocated its producer observer,
and registered the callback there. Rejected partial timer preparation could
cancel/dispose those fields under the registry too. The preceding batch moved
terminal detached disposal, not this separate construction/error context.

This batch moves detached record/event admission and timer node/observer
preparation outside the registry. Capability-backed records/timers require
staged unified authority and remain separate work. These are source
observations, not attribution of a historical kernel panic.

## Capture, complete preparation and publication

Canonical `submit_detached` and `submit_detached_timer` use one private
`completion/detached_preparation.rs` path; old guarded allocation branches are
removed. The existing observer-allocation fixture hook uses the same path.

Capture takes the exact namespace root, original record/timer account Arcs,
submission capacity, trusted platform classification and selected CQ source.
Classification is captured before the registry hold and matched to its stored
generation; the permanent kernel designation remains explicit. Capture checks
retirement and preserves existing CQ backlog reconciliation and live-capacity
checks. It clones ownership without preparing new payload storage there.

One `Preparing` owner retains the capture, global 64-bit operation ID,
complete unpublished `DetachedOp` and optional prepared timer node. Outside
`COMPLETIONS`, it reserves original record admission and, for a timer, event
admission. Cancellation state, node construction and observer allocation are
fallible there. The cancellation handle joins that owner before node preparation;
the observer is allocated successfully before callback registration. All partial
fields remain owned through a later error, and ordinary unused disposal runs
outside the local registry guard. Unpublished preparations consume the same
original limits as live records/events; there is no uncharged staging pool.

Publication borrows the owner. Under the registry it rechecks exact root,
record/timer account identity, capacity and retirement, then exact CQ source
identity. A stale/retired namespace returns `UnknownAddressSpace`; a missing
or replaced CQ returns `NoCompletionQueue`, including a replacement retaining
the same numeric ID. A retained source reference prevents allocation-address
reuse from imitating the predecessor. Validation precedes further backlog
reconciliation and record movement. Capacity is checked again before insertion.
The complete already-prepared record moves once into the existing registry.

Rejected publication leaves the record/node and original charges in the
preparation through guard exit. Public adapters then dispose it and return the
typed error. This is unstarted rollback, not Pending, a public owner-preserving
retry interface, custody or permission to replay accepted work. No producer is
enqueued on preparation/publication rejection.

The prepared event keeps its original deadline; publication and enqueue never
restart it. After successful publication, consuming `enqueue` runs once outside
the registry.
If teardown wins first, it cancels the published record while the preparation
still owns its node. The original event charge remains until enqueue observes
the flag and discards that cancelled insertion. No accepted operation is
recreated or redirected to a successor root/CQ. Existing terminal cancellation,
backlog retention and exact-source notification remain unchanged.

No new registry, capability ABI, signed/storage/wire format, wait/poll policy,
forced IRQ enabling, physical pin or root lease is introduced. Rejected preparations consume an ID without returning it to a free list.
Captured identity and an empty registry are not physical quiescence or cleanup
authority. The existing global allocator still uses wrapping `AtomicU64::fetch_add`;
full counter exhaustion can violate its documented nonreuse intent. This batch
does not change that pre-existing edge. Checked exhaustion is a recorded
follow-up; finite native runs do not exercise a full-counter wrap.

## Context inventory and remaining work

| Entry | Qualified here | Remaining boundary |
| --- | --- | --- |
| Detached plain submission | Original record admission outside registry; one prepared record and captured root/accounts/capacity/CQ through borrowed publication and rejection. | Fresh registry-map nodes still allocate infallibly during insertion. |
| Detached timer submission and observer-injection adapter | Original event admission, cancellation state, prepared node and producer observer allocate outside registry; all partial errors dispose there. | Timer queue/comparator and arbitrary enclosing caller/IRQ contexts remain separate. |
| Publication | Exact namespace/source and retirement revalidation precedes moving the complete record; rejected owners survive until guard exit. | Existing capture/publication backlog reconciliation and map metadata remain under serialization. Successful insertion is not claimed universally allocation-free. |
| Accepted timer before enqueue | Consuming once-only post-guard insertion; teardown retains the node's original event charge through cancelled discard. | This is logical cancellation, not physical root release or a general recovery owner. |
| Capability-backed submit/timer | Unchanged. | Completion/control-block preparation, staged authority admission/publication, timer installation and their error/destructor contexts remain work. |

The existing C16 row and local entry table in
[cleanup/recovery](../../reference/cleanup-recovery.md) carry this boundary.
SEC-07/18, aggregate principal/heap admission, full outer-context qualification
and hostile-pressure progress remain open.

## Focused native controls

The existing callback-admission fixture now executes real original-account
and prepared-event controls:

- Prepared-node and producer-observer allocation boundaries reenter
  `COMPLETIONS`. Injected node rejection and later observer rejection release
  their original record/event charges without publishing or enqueuing work.
  A successful private preparation retains one of each until ordinary Drop.
  This is injected rejection, not physical allocator exhaustion.
- The public observer-injection adapter also reenters the registry during
  allocation, then successfully publishes a distant timer. Cancellation returns
  its exact operation/cookie/Cancelled CQ result and releases its local event.
  A plain detached operation then publishes and completes through the same
  record-preparation path.
- One timer preparation plus one plain preparation consumes a capacity-two
  record account without publishing either. Another public timer rejects with
  `WouldBlock`; no extra node appears, and ordinary disposal releases only
  each original preparation's charges.
- Eight heap-held rejected publications cover timer and plain preparations
  after namespace replacement, same-ID CQ replacement, actual user-root
  retirement and CQ removal. Both complete records and the prepared timer node
  stay owned/charged through rejection. The successor namespace/account and
  publication table remain unchanged. Only post-guard disposal releases them.
- A zero-deadline timer publishes without enqueue, then namespace close wins.
  Its record charge releases, while its unpublished prepared node still holds
  the original event charge. Consuming enqueue discards that cancelled node
  and releases admission; later completion rejects after close.

These raw namespaces are fixture-private; the retirement control uses an actual
user-root generation. No executing-thread sponsor is substituted or shared kernel
account saturated to manufacture isolated accounting. Entry IRQ state is preserved. Heap-held
controls cover rejection before map insertion, not fresh map-node allocation.
The tests are forced local orderings, not exhaustive cross-LP interleavings,
physical cleanup proof or production progress measurements. Existing partial
observer rollback, weak record lifetimes, shared capacity, timer cancellation,
CQ saturation and all prior terminal-disposal probes remain enabled.

Host validation passed **494 Rust tests, zero failures, four ignored**, plus
signing, boot/diagnostic/runtime-input/tooling/object-store checks and cluster-sign
self-tests. Strict Clippy passed both custom targets with
`--no-default-features --features acpi -- -D warnings`. Formatting, whitespace
and 442 local Markdown link checks passed. New local owner/guard controls
execute in native guests, not host mocks of kernel locks.

## Native matrix and evidence

| Profile | Detached preparation probe at | Exact close/exit episodes at | Final passed / failed / pending at |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 1.720148 s | 64 at 9.736702 s | 17 / 0 / 0 at 11.695832 s |
| Intel VT-d | 1.334152 s | 64 at 13.847326 s | 15 / 0 / 0 at 14.779145 s |
| AMD-Vi | 1.538481 s | 64 at 15.125905 s | 15 / 0 / 0 at 16.075967 s |
| ARM security | 1.329424 s | 64 at 10.044308 s | 19 / 0 / 0 at 56.933704 s |
| Intel security | 1.246658 s | 64 at 13.910183 s | 17 / 0 / 0 at 61.507523 s |

All five runners exited **0**, diagnostic result `complete`. No failed native
sample, panic or Raft persistence fail-stop was observed in this matrix. Five
composite detached-preparation probes passed, including **40 heap-held rejected
publications** across plain/timer namespace, CQ replacement/removal and actual
root-retirement controls. Prepared admission stayed bounded and close-before-
enqueue retained the original event charge through cancelled insertion. Existing fixtures
completed **320 exact close/exit episodes, 20 first-entry controls, 20 forced
park controls and 15 real EL0 growth modes**. Every profile reported 192 timer
wakes preserving established LP affinity. ARM exercised certified migration;
x86 kept its explicit never-dispatched gate and skipped started-context runtime
moves. These bounded observations do not establish whole-node progress or the
cause of historical intermittent failures.


Runtime Rust and native probes were frozen before the completed matrix;
documentation/evidence was finalized afterward. Every profile uses four LPs,
fresh storage, `-D warnings`, a 180-second guest budget and automatic all-LP
failure capture. No service sources changed; full staged bundle inventories
match the preceding batch byte for byte.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance detached-prepare-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance detached-prepare-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance detached-prepare-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance detached-prepare-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance detached-prepare-x86-security
```

AMD reuses the completed Intel no-network kernel with
`CATTEN_SKIP_KERNEL_BUILD=1`. Security profiles use
`CATTEN_ALLOW_SANDBOX_NETWORK=1`, HTTP/deployment ports 19080/19081 for ARM and
19180/19181 for x86. Assembly-section checks remain enabled.

[Version-controlled evidence](evidence/2026-10-16-detached-submission-preparation.json)
records reviewed changed-Rust hashes, each kernel/boot-image/initial-disk/final
UART hash, concise run observations and full unchanged service inventories.
These inline results and the checked-in JSON are the published evidence;
temporary UARTs and ignored diagnostic captures are not needed to read it.
Per-run dirty-diff manifests omit untracked additions; reviewed per-file hashes
and the resulting commit anchor the new module/probes. This is not a full-tree
source manifest or captured physical hardware state.

Next work includes checked operation-ID exhaustion, capability-backed
record/timer preparation and staged
unified authority, registry/map and backlog metadata, and outer producer/queue/
IRQ contexts. Historical intermittent failure attribution remains open.
