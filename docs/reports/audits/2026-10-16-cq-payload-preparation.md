# CQ payload preparation outside the completion registry

Date: 2026-10-16. Baseline `0243b641`, following terminal completion observer
disposal. Scoped R7-1/2 and C16/G1/G4 work. SEC-07/18 remain partial; the 38
findings remain 29 corrected within scope, six partial and three open. Eighteen
cleanup owner families and seven gates are unchanged. Historical IPC exit,
stack, persistence and NIC failures remain unattributed by this batch.

## Observed boundary

`stage_cq` ran under `COMPLETIONS` for both attachment and whole-namespace
creation/replacement. Besides reserving queue/backing admission, it allocated
a charged waiter list, reserved backlog storage and allocated a heap ring.
List/backlog/ring rejection also destroyed partial storage under that registry.
Successful old-payload detachment had already moved outside the guard in the
preceding source-disposal batch; preparation/error cleanup was a separate gap.

Physical-ring initialization was correctly delayed until admission and alias
checks. Moving preparation outside the registry must preserve that ordering,
recheck a potentially retired/replaced namespace, and keep old-plus-new charges
with their original accounts. These source observations do not establish the
cause of a historical kernel panic.

## Captured preparation and borrowed publication

The canonical CQ open APIs retain one current path, implemented privately in
`completion/queue_preparation.rs`. The old `stage_cq` path is removed and its
private detachment fixture uses the same heap preparation constructor.

Attachment captures the exact namespace root identity, CQ budget owner,
submission capacity and trusted platform classification under a short registry
borrow. The platform handle is captured before that borrow and must match the
stored generation. Outside the registry, `Prepared` validates ring capacity
and physical alignment, reserves the original count/byte charge, then prepares
the source list, rounded backlog and optional aligned heap ring. List and ring
allocation are fallible. All partial-error destruction happens in this outer
preparation scope. Backing remains charged until its fields are destroyed.

Publication borrows `Prepared`. It rechecks root/account identity and capacity,
then the existing retirement predicate, under registry serialization. Namespace
replacement, including a synthetic namespace with no physical root, cannot use
an old account to publish into its successor. Stale identity is
`UnknownAddressSpace`; an unchanged retired target is `RetiringAddressSpace`.
The constructor's errors retain their existing typed variants.

Physical preparation leaves the borrowed page untouched and keeps its ring
pointer private/uninitialized. Publication performs alias checks under the
same registry hold as initialization and replacement. Capacity/alignment were
preflighted; if the initializer rejects, the preparation still retains its state
for post-guard disposal. It has no rejecting branch after starting frame writes.
A competing queue claiming the frame after capture therefore rejects before
any write. Same-queue physical reuse still requires the caller's quiescent
consumer contract. A captured identity is not a physical pin or a root lease.

`PreparedNamespace` owns the complete private namespace and preparation. It
stages the default CQ's map node outside the registry too. Borrowed publication
rechecks retirement and aliases before physical initialization and installing
the complete namespace; current authority revocation remains serialized.
Rejected private namespaces leave every field owned until registry unlock.

Successful queue/namespace publication returns the complete old payload for
post-guard destruction, preserving the existing detached-slot and authority
accounting. No new registry, metadata ticket, physical receipt, custody category
or retry owner is introduced. No ABI, storage format, deadline, admission-wait
policy or IRQ enabling change. Pending operations/results are not observed by
preparation, and preparation rejection grants no permission to replay them.
These rejected preparations are unstarted rollback: public adapters dispose
them after unlock and return the typed error, without exposing a recovery or
retry owner. The private forced-alias control retains one still-unstarted owner
through its checks before its single successful publication.

## Local context inventory

| Entry | Qualified here | Remaining boundary |
| --- | --- | --- |
| Heap/physical CQ attachment and replacement | Exact root/account/capacity capture; admission, source/backlog/heap-ring allocation and partial rejection outside registry; borrowed identity/retirement/alias revalidation. | A new key can still allocate registry-map metadata. Existing source/consumer ownership and caller IRQ/outer guards remain required. |
| Default CQ with whole-namespace creation/replacement | Complete private namespace/CQ-map preparation outside registry, retained through rejected publication; physical writes after checks. | A fresh namespace can allocate the outer registry-map node. Old unified authority revocation/metadata stays under serialization; root/lifecycle quiescence remains separate. |
| Successful payload replacement | Existing whole old queue/namespace owner returned beyond registry unlock, with original charges. | Physical mapping/consumer release needs its existing proof. This batch adds none. |
| Completion record and timer construction/disposal | Unchanged. | Record/control-block preparation, capability/authority/map metadata, timer-specific cancellation and broader outer contexts remain work. |

This extends the existing C16 row and local boundary table in
[cleanup/recovery](../../reference/cleanup-recovery.md). It does not claim that
every registry publication is allocation-free or every outer destructor safe.
Private namespace/account/CQ-map metadata still uses its existing infallible
constructors outside the registry; metadata OOM is not a recoverable payload
rejection.

## Focused native controls

The existing callback-admission fixture now runs these real owner controls:

- List and heap-ring allocation boundaries can reenter `COMPLETIONS`.
  Injected list rejection refunds its original queue charge; injected ring
  rejection disposes a real admitted list and already-reserved backlog, also
  restoring the original account. These are injected preparation failures,
  not forced physical allocator exhaustion or direct backlog-allocation failure.
- Two preparations capture one live namespace before publication. Actual
  replacement runs with the heap guard held; both complete old queue owners
  and their charges survive the splice. Their post-guard disposal cancels the
  original registered waiter without fabricating readiness. Peak count is three
  before disposal and returns to the one live queue afterward.
- Namespace replacement after capture rejects the old preparation with the
  heap held. Its original account remains charged until explicit disposal;
  the successor's account and live queue remain unchanged.
- An actual user-root retirement after attachment and private-namespace
  preparation rejects both heap-held publications. All old/new charges remain
  in their respective owners until guard exit; normal root close then releases
  the registered queue. No live sponsor or budget is substituted.
- A fixture-owned physical page stays filled with `0xa5` after preparation.
  A competing namespace registers it and receives a real result before the
  original attachment and private-namespace publication are attempted.
  Both reject aliasing under the heap guard; all 4,096 page bytes, including
  that pending result, remain identical. Charges remain held until disposal.
  Once the competing namespace leaves and no consumer remains, the original
  preparation publishes the frame once, retaining the replaced heap queue's
  charge through post-guard destruction. The frame is returned only after all
  referencing namespaces/preparations have left.
- A complete private namespace replaces an existing empty-authority namespace
  under the heap guard and retains the old complete payload/charge. Actual
  capability revocation is checked separately through the public adapter outside
  that guard; authority metadata is explicitly not qualified as heap-safe.

Every probe preserves entry IRQ state. Private fixture namespaces and captured
source identities establish local serialization/ownership only, not physical
root quiescence, exhaustive cross-LP interleavings or hostile-workload progress.
Existing domain/node count-and-byte exhaustion, ordinary/platform headroom,
partial loader rollback, retirement, ASID reuse, invalid ring, physical alias,
same-queue frame reuse and pending-result preservation controls remain enabled.

Host validation passed **494 Rust tests, zero failures, four ignored**, plus
signing, boot/diagnostic/runtime-input/tooling/object-store checks and cluster-sign
self-tests. Strict Clippy passed both custom kernel targets with
`--no-default-features --features acpi -- -D warnings`. Formatting, whitespace
and 437 local Markdown link checks passed. New ownership/guard controls run
in native guests, not host mocks of kernel locks.

## Native execution and evidence

| Profile | CQ payload probe at | Exact close/exit episodes at | Final passed / failed / pending at |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 2.012925 s | 64 at 9.937008 s | 17 / 0 / 0 at 11.854465 s |
| Intel VT-d | 2.988615 s | 64 at 18.888310 s | 15 / 0 / 0 at 20.042645 s |
| AMD-Vi | 1.304183 s | 64 at 14.486259 s | 15 / 0 / 0 at 15.308642 s |
| ARM security | 1.346774 s | 64 at 9.221228 s | 19 / 0 / 0 at 56.285511 s |
| Intel security | 1.241978 s | 64 at 13.981822 s | 17 / 0 / 0 at 61.796921 s |

All five runners exited **0**, diagnostic result `complete`. No failed native
sample, panic or Raft persistence fail-stop was observed in this matrix. Five
composite CQ probes preserved all alias-rejected physical-page bytes and
original charges across the forced publication boundaries. Existing fixtures
completed **320 exact close/exit episodes, 20 first-entry controls, 20 forced
park controls and 15 real EL0 growth modes**. Every profile reported 192 timer
wakes preserving established LP affinity. ARM exercised certified migration;
x86 kept its explicit never-dispatched gate and skipped started-context runtime
moves. These bounded observations do not establish whole-node progress or the
cause of historical intermittent failures.


Runtime Rust and native probes were frozen before the completed matrix;
documentation/evidence was finalized afterward. Each profile uses four LPs,
fresh storage, `-D warnings`, a 180-second guest budget and automatic all-LP
failure capture. No service sources changed; full staged bundle inventories
match the preceding batch byte for byte.

Run commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance cq-payload-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance cq-payload-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance cq-payload-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance cq-payload-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance cq-payload-x86-security
```

AMD reuses the completed Intel no-network kernel with
`CATTEN_SKIP_KERNEL_BUILD=1`. Security profiles use
`CATTEN_ALLOW_SANDBOX_NETWORK=1`, HTTP/deployment ports 19080/19081 for ARM and
19180/19181 for x86. Kernel assembly-section checks remain enabled.

[Version-controlled evidence](evidence/2026-10-16-cq-payload-preparation.json)
records reviewed changed-Rust hashes, each kernel/boot-image/initial-disk/final
UART hash, concise run observations and full unchanged service inventories.
These inline observations and the checked-in JSON are the published evidence;
temporary UARTs and ignored diagnostic directories are not required to read it.
Per-run dirty-diff manifests omit untracked new files; the reviewed file hashes
and resulting commit anchor those additions. This is not a full-tree source
manifest or captured physical hardware state.

Next work remains timer-specific observer/cancellation disposal, completion
record construction, unified authority/registry-map metadata and outer
producer/IRQ qualification. SEC-07/18, aggregate principal/heap admission,
hostile pressure progress and historical failure attribution remain open.
