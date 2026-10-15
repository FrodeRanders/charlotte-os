# Terminal completion observer disposal

Date: 2026-10-16. Baseline `5b1887b4`, following completion callback preparation.
Scoped R7-1/2 and C16/G1/G4 work. SEC-07/18 remain partial; the 38 findings
remain 29 corrected within scope, six partial and three open. Eighteen cleanup
owner families and seven gates are unchanged. Historical IPC exit, stack,
persistence and NIC failures remain unattributed by this work.

## Observed boundaries

`complete_registered` serialized terminal transition and CQ publication under
`COMPLETIONS`. Its `Completion::complete` helper detached event and worker
subscriptions under the inner mutex, then destroyed them after releasing only
that mutex. Their observer captures, independent-list registration allocations
and original watch charges therefore still disposed under the registry.

Event/worker setters also replaced existing fields under the inner mutex.
Rejected late registrations could leave that mutex before destruction but still
run beneath `EventSubmission::install_observation`'s registry read hold. These
are source observations, not explanations of a historical panic.

Moving destruction outside the registry also exposes a reentry boundary: an
observer destructor may replace its namespace or CQ. Resolving the numeric
ASID/CQ again after that destructor would direct the old completion's wake at
a successor source. This batch captures that source during publication.

## Transition and installation ownership

`TerminalTransition` retains the effective result and both detached producer
subscriptions. Under the same registry hold as before, `Completion::complete`
changes the exact operation to Completed, forcing Cancelled if cancellation
was pending. CQ insertion uses that effective result and captures the queue's
existing waiter-list owner. Duplicate terminal transitions remain no-ops.
No buffer is observed or returned during this transition.

After registry unlock, the complete transition owner disposes its event/worker
subscriptions. The producer then drains/notifies only its captured CQ source
and signals the original completion sources. An identity reference retains list
metadata, not ring backing or physical root authority. If a destructor replaces
the CQ, normal replacement discards old registrations; the subsequent old-source
wake cannot consume a newly registered successor waiter. Ordinary CQ signaling
uses the same source-delivery helper after its local registry capture.

`ObservationInstallation` retains any replaced or rejected event/worker
subscription. Setters swap owners under the inner mutex without disposal.
`EventSubmission` checks exact operation identity under the registry, receives
that result, releases the registry, then consumes it through `finish`. Stale
namespace rejection also releases incoming parameter owners after guard exit.
There is one current path; no compatibility allocator or numeric restoration.

Event installation into CancelPending returns cancellation-won and disposes
the late subscription. Worker installation into CancelPending keeps its exit
subscription: cancellation does not establish that the worker exited. Both
reject late terminal/observed subscriptions without retaining them. Replacement
cancels only the old subscription; it fabricates no completion. Registration
field order and the existing independent-list cancellation contract are
unchanged. Original buffer, record and watch charges stay with their owners;
weak completion captures continue retaining their original record allocation.

## Caller and cleanup inventory

| Entry | Local context qualified here | Remaining boundary |
| --- | --- | --- |
| Exact/numeric terminal completion; worker, timer, join and endpoint-close producers | Terminal state and effective CQ result remain serialized; detached event/worker subscriptions dispose beyond registry/inner guards; exact-source wake follows disposal. | Caller-owned outer guards/IRQ policy, other CQ/backlog field destruction and physical producer/root quiescence. |
| `EventSubmission` worker/watch installation | Exact-record validation; replacement and late/cancelled rejection carry unused owners past registry/inner unlock. | Worker-publication enclosing scheduler context, source registration preparation and arbitrary observer captures. |
| Direct setter fixtures | Explicit installation-result disposal after local guard exit; no old scalar/bool cleanup path. | Private namespaces are serialized controls, not real user-root retirement proof. |
| Timer-specific setter and record/CQ staging | Behavior unchanged. | Timer observer/cancellation replacement, anonymous timer enqueue/cancel/destruction, unified authority/map metadata, record construction, CQ staging and partial-error cleanup need separate qualification. |

This updates the existing C16 row and local boundary inventory in
[cleanup/recovery](../../reference/cleanup-recovery.md). It creates no new
registry, custody category, retry owner or physical cleanup receipt. Readiness,
disposal, result consumption and absence remain distinct from quiescence.
No forced IRQ enabling, timeout/admission policy, ABI or wire-format change.

## Focused verification

The native callback-admission fixture now includes real charged event/worker
owners whose destructors reenter `COMPLETIONS` and the original inner mutex:

- With the actual registry and heap guards held, the production terminal
  helper moves both subscriptions into its returned owner. Their two original
  watch charges remain held and no destructor runs there. A second terminal
  call returns None. Releasing the owner after unlock runs both destructors,
  cancels both registrations and leaves the buffer for explicit observation.
- Two public producer paths cover ordinary and CancelPending completion with
  both event and worker subscriptions. Destructors inspect the already-published
  CQ entry and terminal state, checking effective-result parity and retained
  buffer bytes. CQ and completion callbacks reenter both guards only after
  disposal. Repeated completion does not notify again or alter the result.
- A third public producer path replaces its CQ during observer destruction,
  then registers a waiter on the successor source. That waiter remains owned
  and charged, receives zero old-result wakes, and is cancelled explicitly.
  The old CQ registration is discarded while the original completion callback
  still runs. Repeating the terminal call still cannot signal the successor.
- Ten installation controls cover event and worker replacement, CancelPending,
  Completed, Observed and stale namespace identity. Actual production
  installation disposes replaced/rejected observers outside both guards;
  worker cancellation alone retains its watch. Original unpublished-submission
  charges remain until that containing owner is dropped.
- A live callback's weak capture keeps its original record charge after all
  strong completion owners are released. Dropping the capture releases it.
  The fixture explicitly checks both sides of that lifetime.

Every probe preserves entry IRQ state. Sponsors and namespace budgets are
prepared privately; no live account is replaced or shared kernel sponsor
saturated for accounting claims. These are forced local orderings, not
exhaustive cross-LP exploration or physical allocator-failure tests.

Host validation passed **494 Rust tests, zero failures, four ignored**, plus
signing, boot/diagnostic/runtime-input/tooling/object-store checks and the
cluster-sign self-tests. Strict Clippy passed both custom targets after the fixture accounting
correction. Formatting, whitespace and 414 local Markdown link checks passed. The preliminary
ARM compile was deliberately stopped before guest launch to correct a reviewed
assertion that had omitted a still-live weak capture. No guest ran that version.

## Native execution and artifact evidence

| Profile | Terminal disposal probe at | Exact close/exit episodes at | Final passed / failed / pending at |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 1.939235 s | 64 at 9.495724 s | 17 / 0 / 0 at 11.302075 s |
| Intel VT-d | 1.265509 s | 64 at 17.260843 s | 15 / 0 / 0 at 18.334583 s |
| AMD-Vi | 1.340940 s | 64 at 15.533678 s | 15 / 0 / 0 at 16.422267 s |
| ARM security | 1.319826 s | 64 at 8.901588 s | 19 / 0 / 0 at 56.039680 s |
| Intel security | 1.241582 s | 64 at 13.696152 s | 17 / 0 / 0 at 61.409411 s |

All five runners exited **0** with diagnostic result `complete`. No failed
native sample, panic or Raft persistence fail-stop was observed in this matrix.
The new composite probe passed five times, including 50 installation controls
and five successor-CQ replacement controls. Existing fixtures completed **320
exact close/exit episodes, 20 first-entry controls, 20 forced park controls and
15 real EL0 growth modes**. Each profile reported 192 timer wakes preserving LP
affinity. ARM exercised its certified migration policy; x86 retained the
explicit never-dispatched-only gate and skipped started-context runtime moves.
These are bounded fixture observations, not whole-node progress or attribution
of earlier intermittent failures.


Runtime Rust and native probes were frozen before the completed matrix;
documentation/evidence was finalized afterward. Each run uses four LPs, fresh
storage, `-D warnings`, a 180-second guest budget and automatic all-LP failure
capture. No service source changes were made. The full staged service-bundle
inventories match the preceding batch byte for byte.

Run commands (common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`):

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance terminal-disposal-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance terminal-disposal-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance terminal-disposal-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance terminal-disposal-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance terminal-disposal-x86-security
```

AMD reuses the completed Intel no-network kernel with
`CATTEN_SKIP_KERNEL_BUILD=1`. Security runs use
`CATTEN_ALLOW_SANDBOX_NETWORK=1`, HTTP/deployment ports 19080/19081 for ARM
and 19180/19181 for x86. Kernel assembly-section checks remain enabled.

[Version-controlled evidence](evidence/2026-10-16-completion-terminal-disposal.json)
records each kernel/boot-image/initial-disk/final-UART hash, concise run markers,
reviewed changed-Rust hashes and full unchanged service inventories. These
inline observations and the checked-in JSON are the published evidence;
temporary UARTs and ignored diagnostic artifacts are not required to read it.
Per-run dirty-diff manifests exclude untracked new files; the reviewed file
hashes and resulting commit anchor those additions. This is not a full-tree
source manifest or a captured physical hardware state.

Remaining next work is completion record/CQ staging and partial-error cleanup,
timer-specific observer/cancellation disposal, authority/map metadata and outer
producer/IRQ qualification. SEC-07/18, aggregate principal/heap admission,
hostile pressure progress and historical failure investigations remain open.
