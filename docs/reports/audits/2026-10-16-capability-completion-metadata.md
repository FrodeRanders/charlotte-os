# Capability-backed completion metadata

Date: 2026-10-16. Baseline `17807f05`, following detached completion metadata.
This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial; numbered
findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source gaps and scope

Capability-backed completion preparation already built its charged record,
shared authority preparation and timer fields outside `COMPLETIONS`. Final
publication still inserted a `BTreeMap` node after publishing authority.
Close, staging rollback and namespace teardown removed/freed payload metadata
and used the convenience authority-removal adapter under the enclosing
completion lock. Leaving `CAPABILITIES` alone did not leave that outer lock.

A separate source defect existed in `abort_submission`: it returned the live
slot without removing a completed capability's retained CQ backlog row. With
a full ring, repeated submission/completion/abort could accumulate stale rows
that no longer belonged to live slots, violating the backlog-capacity premise.
Staged-event rollback and completed close already removed those rows. This
batch gives all three paths the same retirement step. This is a source finding
and regression control, not attribution of a historical panic or an observed
hostile-workload exploit.

The shared `RetiredRecord::release` also destroyed its authority charge field
before the enclosing node allocation was deallocated. It now uses the existing
`RetiredEntry::release_with` handoff to retain that exact charge until actual
node deallocation returns. The callback only moves the typed charge into a
stack owner; no allocation, replacement reservation, scalar reconstruction or
new cleanup policy is introduced. This ordering improvement applies to existing
individual authority-node consumers; their enclosing contexts and completion
proofs remain independently qualified.

## Prepared publication and complete retirement

The capability payload map now uses shared `AdmittedMap` nodes. The existing
`Preparing` transaction fallibly allocates one empty `PreparedEntry` after
charged record construction and before shared authority preparation/reservation.
Metadata rejection therefore publishes no hidden or live authority. Ordinary
failure explicitly frees unused payload storage before its original charged
completion, after local guards. Inert preparation fallback retains every field.

Final publication still validates exact namespace root, original accounts,
capacity and retirement before the existing authority batch. It then relinks
the prepared node without allocation or a fallible payload-insertion step.
The record stores its charged `CompletionRef` and an initially empty detached
authority field. Sorted map scans remain bounded by the existing 1,024-record
namespace limit. No identity allocator, compatibility branch, ABI or wire
format changes.

Logical retirement validates the captured completion where required, detaches
live unified authority into its existing `RetiredRecord`, stores that owner in
the same payload node, removes only the exact operation/capability backlog row,
and returns the live slot. `RetiredCapability` retains that complete admitted
node: dropping it retains payload, detached authority and original charges;
there are no separately dropped sibling owners. Explicit finish leaves local
guards, keeps an existing charged completion Arc alive, frees payload and
authority nodes, then releases that Arc. Original completion admission still
follows final strong/weak backing through `ChargedAllocator`.

Close still rejects in-flight work and exact-object mismatch. Abort and
uncommitted event rollback retain their existing local staging semantics and
post-guard subscription disposal. Withdrawal removes only an unpublished
backlog row; an already visible ring entry is not rewritten or replayed. This
change supplies no permission to resubmit accepted work or release uncertain
physical backing. Subscription/timer field destructors and arbitrary enclosing
caller guards remain separate from local metadata disposal.

Whole namespace retirement stores each detached authority owner in its existing
payload node before releasing `COMPLETIONS`. Its complete namespace owner then
cancels the same public subscriptions and explicitly drains capability nodes
alongside the preceding batch's detached-node/queue disposal. Numeric namespace
and capability reuse cannot redirect deferred release: the retired owner frees
its original nodes/accounts without another authority lookup. The shared map's
new mutable iterator exposes immutable keys and mutable values, preserving
sorted node identity without allocation.

## Controls and retained fixture cost

All heap-held native probes run in the synchronous phase before AP schedulers
start; conversion of this second map does not qualify other allocating writers
or permit moving those probes into concurrent boot work.

- Reject payload-node allocation while reentering `COMPLETIONS`. The original
  record/watch charges remain live during preparation and refund on rejection;
  the authority allocator must not run. Successful preparation also reenters
  the registry.
- Prepare and publish 64 records, including 32 timers, into a fresh map with the
  heap locked. Complete them, then detach 64 payload/authority pairs under a
  second heap hold. Admission stays charged until explicit post-guard disposal.
  Captured strong handles preserve buffers; final strong destruction leaves
  64 charged weak-only records until final weak release. In-flight and duplicate
  close reject without consuming another owner.
- Hold a one-entry ring full. Complete and abort 64 ordinary submissions, then
  64 uncommitted event submissions. Every iteration starts with exactly one
  retained candidate row and removes it before returning its slot. Record and
  authority usage return to the filler-only baseline; no stale row accumulates.
- Detach two namespace authorities with heap and registry held. After releasing
  the old authority namespace, force numeric capability reuse in a successor.
  Explicit old-owner disposal leaves the successor live and credits only the
  original accounts; original weak backing remains charged until final release.
- Abandon a complete timer-bearing preparation and a retired payload/authority
  pair with heap and registry held. Hidden authority remains unusable, detached
  authority stays revoked, and original payload/node/watch/timer charges remain.
  A numeric successor remains unchanged. The preceding detached-namespace
  abandonment control now also checks retention of its detached authority charge.

These private raw-namespace controls use no actual user root, live sponsor
substitution or production retry custody. This batch adds bounded retention of
**two record charges, one timer event, one watch charge and two authority
charges** until guest shutdown. The preceding namespace-abandonment control
additionally retains its one authority charge now that authority accompanies
the complete payload owner. Together the completion metadata controls retain
**nine records, four timer events, two CQ payload charges, two watch charges
and three authority charges**, plus their original nodes/list/buffer references.
These deliberate negative controls are separate from successful reclamation;
no counters are reset to hide them.

A new allocator-traced host control executes the shared `release_with` pattern:
the payload owner survives node deallocation and is destroyed afterward, with
no allocation during handoff. The existing admitted-map control now mutates
payloads through sorted immutable keys, then checks detached node identity and
zero allocator work. Existing native controls retain stale/retired publication,
hidden-authority rejection, absolute timer deadlines, close-before-enqueue,
CQ alias/mapping rejection, cancellation, returned-resource cleanup and
post-guard observer reentry. These are scoped controls, not aggregate heap or
hostile-pressure qualification.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus its signing/tooling/Python controls. Strict kernel
Clippy passes on both custom targets with `-- -D warnings`. Workspace formatting
and diff whitespace checks pass.

Runtime Rust was frozen before this five-profile matrix. Every profile uses
four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. Every runner
exits zero with diagnostic result `complete`:

| Profile | Capability metadata fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.865034 s | 17 passed, zero failed/pending at 11.395973 s |
| Intel VT-d, no network | Passed at 1.265561 s | 15 passed, zero failed/pending at 14.159979 s |
| AMD-Vi, no network | Passed at 1.379512 s | 15 passed, zero failed/pending at 18.549013 s |
| ARM / SMMUv3, security | Passed at 1.348640 s | 19 passed, zero failed/pending at 56.144250 s |
| Intel VT-d, security | Passed at 1.250531 s | 17 passed, zero failed/pending at 61.129707 s |

Together these execute **320 fresh heap-held publications, 320 paired
payload/authority removals and 640 full-ring abort cycles**, plus guarded
retention, namespace reuse and original weak-only account controls. Each also
completes 64 scheduled IPC close/exit episodes, 320 total, with exact exit
watches. Both security profiles complete two scoped EL0 generations; concurrent
cancellation traffic retires after 14,852 requests on ARM and 13,932 on Intel.
These are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration.

No failed native sample, panic, deadline or persistence fail-stop was observed
in these runs. That does not erase prior failed samples or establish historical
failure causes. Shared authority release also runs through existing multi-kind
admission/disposal and device/IPC/loader controls; passing them does not qualify
every outer caller or physical platform.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance capability-map-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance capability-map-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance capability-map-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance capability-map-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance capability-map-intel-security
```

AMD additionally sets `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes,
using its identical no-network ELF. Security runs set
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 for
ARM and 19180/19181 for Intel; actual network forwards appear in evidence.
All staged service files and each run's inventory match the preceding baseline
byte for byte. Services and userspace runtime libraries were not edited or
rebuilt.

[Version-controlled evidence](evidence/2026-10-16-capability-completion-metadata.json)
records reviewed source hashes including new fixtures and the shared release
helper, commands, unchanged service inventories, artifact/UART identities and
concise observations. Historical failed samples remain in the preceding
[backlog refund report](2026-10-16-completion-backlog-refunds.md). Published
results do not depend on ignored build artifacts or temporary logs.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Remaining boundary

CQ and global namespace maps still allocate/free metadata under existing
guards. Completion timer/observer disposal, other authority consumers' enclosing
guards, IRQ/active queue contexts, aggregate heap/principal admission and
integrated pressure/progress remain open. Both per-operation maps now use
admitted nodes; the entire completion registry is not allocation-free.
Historical IPC exact-worker exit, stack, persistence and NIC incidents remain
unattributed. No polling, forced IRQ enabling, readiness source, root pin,
physical retry or custody policy changes.
