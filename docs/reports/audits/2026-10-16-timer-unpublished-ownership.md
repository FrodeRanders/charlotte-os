# Complete unpublished timer-node ownership

Date: 2026-10-16. Baseline `87e0bff`, following post-queue timer delivery.
This continues R7-1/2 and C16/G1/G4/G7. SEC-07/18 remain partial;
numbered findings remain 29 corrected within scope, six partial and three open.
No new registry, cleanup family, physical proof or recovery custody.

## Source findings and scope

Queued, selected and retired timer chains already retained complete payloads
on abandonment. A successfully constructed `PreparedEvent` still held an
ordinary `OwnedNode` field: abandoning that preparation could destroy its Box,
callback weak reference, source and original charge in the caller's unknown
context. Capability/detached completion submission owners had inert containing
transactions, but their explicit finish paths relied on this implicit node
destruction. Standalone scheduler preparations had no such containing fallback.

Sleep also acquired its local park-publication mask before fallible event/node
preparation. Waiter rejection dropped the prepared node before releasing that
mask. Timed waits could implicitly destroy unused preparation on observer
allocation failure, budget expiry or waiter rejection, including beneath the
local setup mask. These are source findings; no historical panic, IPC exit,
stack, persistence or NIC incident is attributed to them.

This batch covers the **completed private node owner** and its ordinary unused
disposal callers. It does not make raw `TimerEvent` or partially constructed
allocation arguments inert, qualify final cancellation-reference destruction,
move waiter preparation out of every scheduler context, or add recovery.

## Consuming publication or unused disposal

`PreparedEvent` retains its complete `OwnedNode` behind `ManuallyDrop` and is
marked must-use. Abandonment performs no node/source/allocator/refund work and
retains every field and original admission. No callback or cancellation is
fabricated. The abandoned private event has no queue claim to clear and no
recovery selector; its retention is terminal, not Pending or retry ownership.

Enqueue consumes the same owner into its original queue node without allocation
or new admission. `cancel_unpublished(self)` consumes only still-private
preparation into the existing complete retirement owner, then explicitly
disposes it. This does not select a queued identity, signal an event, complete
an operation or discharge another owner's physical obligation. Original
admission still follows actual node and cancellation allocation lifetime,
including surviving strong and weak cancellation references.

| Caller | Ordinary unused preparation |
| --- | --- |
| Scheduler sleep | Prepare event admission/node before its local park mask; drain existing deferred delivery, then preserve one masked Blocked/enqueue transition. On rejection, release the local setup mask before consuming unused preparation and entering the existing bounded-duration runnable fallback. Successful sleep still rebases its interval after registration. |
| Checked timed wait | Observer allocation rejection, elapsed preparation budget, elapsed post-drain budget and waiter rejection explicitly consume the unused node. Local setup masks leave first where present. Accepted watchdogs still enqueue once and retain the original absolute operation deadline across unrelated wakes. |
| Capability-backed completion preparation | Existing explicit finish disposes unused nodes after local registry/authority guards leave; its containing owner still retains all fields on abandonment. |
| Detached completion preparation | Existing post-registry finish likewise consumes unused nodes explicitly; published work enqueues its original node once. |

No caller-owned mask is force-enabled. First-use waiter metadata, callback and
source finalization, partial construction, final cancellation handles and
unrelated enclosing guards remain separate qualification work. Explicit disposal
is not safe merely because it is named; callers must still qualify their
containing context. There is no ABI, wire grammar, public status or custody
policy change.

## Controls and deliberate retention

Synchronous pre-AP controls prepare private sponsors before event capture and
never replace live sponsors or reset shared counters.

- Explicitly dispose 64 real unused watchdog nodes with live callback and waiter
  references. One is flag-cancelled before publication; queue removal correctly
  returns false and its node remains charged until consuming disposal. Before
  each actual disposal, the shared finish helper checks availability of the
  actual timer queue and heap lock. No callback runs. Source entries release;
  retained cancellation owners remain charged through their actual lifetime.
- Keep one cancellation allocation weak-only after ordinary cleanup. Retiring
  its sponsor and releasing that weak reference refunds only the original
  account, without crediting a fresh independent successor.
- Explicitly dispose a noncancellable scheduler sleep node under an existing
  caller mask; the mask remains set until that caller releases it. Positive
  controls restore their original event/node baseline.
- Abandon one completed private node under held heap and real timer-queue
  guards. Its event/node/cancellation charge, callback weak reference, source
  and waiter remain intact. Later independent token cancellation releases only
  its waiter entry; the event, callback and empty source allocation remain.

The new negative control deliberately retains **one event charge and one empty
admitted source-list allocation** until shutdown, with zero persistent waiter
entry charges. It abandons no real queue claim or user root. Earlier delivery
controls retain two event/source pairs, queue disposal retains one, and
completion metadata retains its separate ten records, four timer events, four
CQ payload charges, two watch charges and three authority charges. The related
negative controls now retain eight original event charges in the shared timer
pool. This control runs after the full-pool baseline checks.

Existing node allocation and completion producer allocation rejection,
publication rejection/namespace reuse, pre-enqueue teardown, full-source sleep
rejection, successful sleep/watchdog cleanup, selected cancellation, final
observer destruction and scheduled exact IPC exit controls continue to run.
Their passing results qualify those bounded cases, not every implicit field
destructor or outer mask.

## Execution results

The host runner passes **500 Rust tests, zero failed, four ignored**, across
57 harness summaries, plus signing/tooling/Python controls. Strict kernel Clippy
passes on both custom targets with `-- -D warnings`; workspace formatting and
diff whitespace checks pass. The host suite ran before the final native-kernel
guard correction; its packages are unchanged by that correction. Corrected
kernel behavior is covered by the Clippy checks and native matrix.

Corrected runtime Rust was frozen before this five-profile matrix. Every profile
uses four LPs, fresh storage, a 180-second guest budget, denied Rust warnings,
assembly-section checks and automatic all-CPU failure capture. All runners exit
zero with diagnostic result `complete`:

| Profile | Unpublished disposal fixture | Deferred summary |
| --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.273040 s | 17 passed, zero failed/pending at 11.904310 s |
| Intel VT-d, no network | Passed at 1.077932 s | 15 passed, zero failed/pending at 19.097919 s |
| AMD-Vi, no network | Passed at 1.073618 s | 15 passed, zero failed/pending at 16.599689 s |
| ARM / SMMUv3, security | Passed at 1.062578 s | 19 passed, zero failed/pending at 56.584710 s |
| Intel VT-d, security | Passed at 1.060674 s | 17 passed, zero failed/pending at 61.553562 s |

Together these explicitly dispose **320 complete unused watchdog nodes and five
noncancellable sleep preparations**, with original caller IRQ policy intact.
Each profile checks pre-publication flag cancellation, source cleanup, weak-only
original charges and guarded completed-owner abandonment. Existing post-queue
selection/delivery, nested-drain, observer destruction, exact cancellation and
node reclamation controls also pass.
Each profile also completes 64 scheduled IPC close/exit episodes with exact exit
watches, 320 total. Both security profiles complete two scoped EL0 generations. These
are bounded observations, not sustained fairness or hostile-pressure
qualification. ARM retains certified runtime migration; x86 qualifies only
never-dispatched migration. The two initial native samples panicked at the
enclosing-IRQ fixture; their separate failure evidence and the temporary-guard
correction are described below. No panic, deadline or persistence fail-stop was
observed in this corrected matrix. Historical failed samples remain separately
preserved in the [backlog refund report](2026-10-16-completion-backlog-refunds.md);
these passes do not establish their causes.

Commands share `CATTEN_SKIP_EMBED_BUILD=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance timer-preparation-final-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance timer-preparation-final-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance timer-preparation-final-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance timer-preparation-final-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance timer-preparation-final-intel-security
```

AMD adds `CATTEN_SKIP_KERNEL_BUILD=1` after Intel completes; their no-network
kernel ELF hashes match. Security profiles add
`CATTEN_ALLOW_SANDBOX_NETWORK=1` and HTTP/deployment port inputs 19080/19081 on
ARM and 19180/19181 on Intel; evidence records actual network forwards.
All staged service files and run inventories match the preceding baseline byte
for byte; service/runtime libraries were not edited or rebuilt.

[Version-controlled evidence](evidence/2026-10-16-timer-unpublished-ownership.json)
records reviewed hashes for all nine Rust files including the new
unpublished-owner fixture, commands, artifact/UART hashes, unchanged service
inventories and concise
observations. Frozen Rust hashes are rechecked after the matrix. Published
results do not depend on temporary logs or ignored build artifacts.
Documentation was finalized afterward; this is not a full-tree or physical
platform attestation.

## Failed intermediate samples and correction

The first AArch64/SMMUv3 and Intel VT-d no-network samples both exited one with
diagnostic result `kernel-panic`. The new synchronous unpublished-node controls
had passed at 1.237320 s and 1.104861 s respectively. Both later stopped at the
existing `deferred.rs:321` check in `test_park_delivery`: `wait changed the
enclosing IRQ policy`, actual false versus expected true. QMP, all four CPU
stacks/registers and kernel snapshots were captured successfully; LLDB reported
no command errors.

The intermediate patch had added `drop(setup)` and unused-node disposal inside
the error arm of a `match SYSTEM_SCHEDULER.read().block_thread_...` expression.
Rust retained that read-guard temporary into the arm. Releasing the local mask
there enabled IRQs beneath the still-live guard; its later IRQ-tracking unlock
restored the masked state, and node disposal also occurred beneath that guard.
This is an introduced ordering error in this batch. The correction binds the
registration result at a separate statement before matching, so the scheduler
guard leaves before either mask restoration or explicit disposal. Only that
scheduler source boundary changed between the failed and corrected source
freezes. Existing timed/event-only, nested-mask and forced-preemption precursor
controls exercise the corrected rejection path.

[Separate failure evidence](evidence/2026-10-16-timer-unpublished-failures.json)
preserves both samples, failed source hashes and exact rejection code, commands,
artifact/UART/capture hashes, panic excerpts and all-CPU backtraces. On both
architectures the panic stack resolves through `cq_driver` and
`test_park_delivery`; Intel's other active CPU was in a TLB shootdown, and the
remaining two CPUs were idle. ARM's other active CPU was in userspace and two
were idle. Those concurrent positions do not establish another failure cause.
Optimized local loop variables were not captured, so no exact Boolean fixture
combination is claimed. Passing corrected runs do not erase these samples or
attribute any older IPC, stack, persistence or NIC incident.

## Remaining boundary

Raw event and partial-construction cleanup, final strong/weak cancellation
reference destruction, first-use waiter preparation and broader timer callback/
disposal IRQ or enclosing caller guards remain open. Aggregate heap/principal
admission and integrated pressure/progress remain open. Historical IPC
exact-worker exit, stack, persistence and NIC causes remain unattributed.
Finding dispositions are unchanged.
