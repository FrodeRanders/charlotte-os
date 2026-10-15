# Deferred thread retirement

A dying thread still owns its kernel stack after leaving the scheduler table.
Its owner must survive until its LP has switched off that stack. Slot return
uses [publication-prepared metadata](address-space-retirement.md); deferred
thread ownership now has separately prepared storage as well.

## Preparation and transfer

`Thread::try_new` fallibly prepares one `PreparedEntry<Thread>` before claiming
a generation, allocating stacks or publishing a thread. The stable context Box
is also admitted while still uninitialized, before generation/backing; only a
complete constructed context is written into it. Allocation rejection
returns `ThreadPreparationFailed`. Ordinary construction/publication rejection
explicitly releases the never-admitted stack pair after local serialization
leaves, then drops its metadata. Physical rejection retains its admission. There is no
physical cleanup in the empty node's destructor.

Staging consumes that node out of the live thread and fills it with the entire
thread owner. The contained thread has no second retirement node. Context
boxing retains the same stable-address contract, so the assembly's saved-stack pointer stays stable
when the thread moves from table to node. Staging links the node into its LP's
inline head without allocation. There are 256 heads, matching the validated
boot/scheduler LP limit; no first-use map entry or vector growth is required.

Owner-LP requested abort cleanup captures the current table slot ceiling and
checks one occupant at a time under the thread table. Its own pending request,
owner LP and generation determine extraction. Newly added slots beyond that
ceiling wait for a later boundary. There is no heap-backed request snapshot.

Self-exit retains its exact scheduler handle and master-table context while
marking an owner-side abort request. `next` excludes that requested lifetime,
then the architecture switch saves its context and clears Arm's `on_cpu`
flag before post-switch extraction. Removing the current handle before the
switch would skip that handshake and leave a permanently retained node.

Both architecture switch assemblies run `finish_context_switch` on the incoming
stack, after the outgoing save/CPU handshake and incoming address-space
installation, before restoring its register frame. This stages requested owners
and drains producer-LP deferred notifications with IRQs still masked and local
scheduler/table guards released. Fresh kernel/user trampolines share this boundary
with resumed continuations; delivery cannot depend on returning through an older
`cond_yield_lp` call. A no-switch pass runs the same helper directly. Physical
stack release remains on the pinned, IRQ-enabled reaper. Restore incoming FP/SIMD
state after callback work so initial userspace state contains no callback residue.
The x86 assembly preserves its exact saved-frame pointer and aligns only the
temporary helper call, supporting both initial user and saved kernel frames.
An exact-dispatch negative control and corrected native security/mixed-deployment
executions are recorded in the [first-entry evidence](../reports/audits/2026-10-15-first-dispatch-delivery.md).
They qualify this ordering boundary without attributing the historical IPC
exit-watch or persistence failures.

## Exit-watch storage and publication

Thread-exit observation captures and authorizes an exact thread generation and
optional existing source under a short table read guard. It then prepares the
charged list/entry outside the master thread table and source mutex. The final
write guard rechecks that captured generation and caller domain before linking
prepared storage without allocation or deallocation. Even TID-only observation
cannot attach to a reused slot while preparation runs. Retirement removes the
thread under that same write guard, so registration either precedes removal or
rejects the captured episode.

`PreparingExitWatch` retains its possible source and `PreparedRegistration`
entry through final admission. A competing first registration supplies the
winning source; unused list/entry/weak-reference/charge disposal happens after
the table guard leaves. Existing sources reuse their charged backing. The
original observing-domain watch account and captured classification remain;
there is no per-instance registry or new budget allowance. Entry publication
consumes its prepared node once. Source/entry admission rejection remains
`WaitRegistrationFailed`, distinct from `InvalidThread` and any terminal result.

The [exit-watch preparation review](../reports/audits/2026-10-15-thread-exit-watch-preparation.md)
records the scoped allocation inventory, heap-held publication/rejection,
preparation failure, TID reuse, competing source, authorization and scheduled
IRQ-preservation evidence. It does not qualify the containing completion
registry, all generic observer callers, stack growth's inner walker contexts,
aggregate heap limits or the historical intermittent failure causes.

## Reaping and publication fences

`ReapBatch` owns the detached list, deferred list and `RetirementGuard`. The
transition count/epoch cover table-to-stage transfer and local reaping, so
supervision cannot treat a thread temporarily absent from both registries as
reaped. Final explicit completion returns the transition marker only after
every node was released or reinserted. Abandonment quarantines the lists and
retains the marker; it cannot falsely publish domain quiescence.

Production reaping selects only the current LP's head and rejects masked
entry before claiming nodes. Both architectures use scheduled pinned reapers,
keeping physical invalidation out of IRQ tails. Arm post-switch extraction still
stages owners, but performs no physical reaping there. Both defer a node whose stack contains the executing SP;
Arm also defers while the assembly `on_cpu` ownership flag is nonzero. x86's
ownership predicate remains a stub. The migration gate rejects every x86
context whose `dispatch_count` is nonzero, so only never-dispatched contexts
may move there. The sampler itself still runs on both architectures. The
[selection audit](../reports/audits/2026-10-15-scheduler-selection-storage.md)
found that the earlier claim of disabled x86 runtime migration was not enforced
by the predicate and adds this conservative gate; it does not attribute any
historical failure to migration. LP assignment alone never authorizes releasing
the executing stack.

Idle workers now park on an owned, fallibly admitted waiter for their fixed LP
retirement head, with no timer/watchdog. After linking the complete node,
staging detaches admitted waiters into the shared producer-LP deferred
transport without callbacks or allocation. Only that producer LP dispatches
from yield, idle or post-switch boundaries after its local guards leave,
preserving IRQ state. This also prevents another LP from notifying while a
producer still owns serialization. The fixed sources/transport carry no thread
payload or recovery authority; existing retirement heads remain the sole custody.

Guard-free park setup also drains that producer LP's pending delivery before
publishing `Blocked`, under its IRQ-state-preserving publication mask. Otherwise
restoring IRQs can switch the producer away before `yield_lp` runs, leaving a
dependent worker's wake queued behind a parked producer. Timed readiness waits
recompute their remaining absolute budget after callbacks; event-only waits,
sleep and untimed completion waits share the delivery ordering. No subsystem
guard is held during dispatch and no physical cleanup proof is supplied.
See the [park-boundary correction](../reports/audits/2026-10-14-park-deferred-delivery.md).

The post-registration check reads the durable exact head and repeats the
off-stack/Arm ownership and unstarted-pair predicates. Publication before
registration or notification is therefore not lost; the owned waker captures
the worker generation. Readiness is a hint, never stack quiescence or cleanup
completion. Failed/started pairs are excluded, so a terminal retained node does
not keep a worker runnable or cause periodic rescans. Previously executing
production contexts are staged after their owner LP's context-switch handshake;
immediate removal qualifies a nonexecuting context under scheduler serialization.
Fixtures may synthesize active ownership but cannot authorize a production
remote reap.

Rejected waiter admission remains a distinct outcome. It increments
`REAPER_WAIT_ADMISSION_FAILURES` and backs off for 10 ms before retrying
admission. The existing void sleep API preserves that interval via cooperative
fallback if timer admission also rejects. This is pressure backoff, not the
ordinary readiness path or a physical retry; admission-capacity notifications
and integrated pressure/progress qualification remain separate work.

Detached lists reverse their prepend order to preserve prior insertion-order
notification. Filtering and reinsertion move existing nodes iteratively,
without allocation or recursive destruction. The node retains its complete thread/context while explicit retirement notifies
metadata once and releases the stack pair. Metadata notification precedes root
lease completion, preventing numeric-ASID usage accounting after reuse. Pair
release arms its one-shot fence before detachment/callbacks; any rejection or
interruption is terminal. It retains the original maximum reservation/root/slot
even when one range was released. A failed pair is reinserted in its existing
node and future scans skip physical work; it is never reported as reaped. Only
confirmed pair completion permits explicit metadata and node destruction.
`Thread` holds its complete inline payload in `ManuallyDrop`. Its fallback only
increments `ABANDONED_THREAD_PAYLOADS`; it retains the context Box, stack/root
owners, waiter/waker state, exit subscriptions, sponsors and unused retirement
node without callbacks, cancellation, usage refund, heap/physical cleanup,
guards or logging. The counter records no address or recovery authority.

Consuming `dispose_retired` requires both completed metadata notification and
confirmed release of this exact stack pair. It disarms its fallback before
destroying the payload. `RetiredEntry::release_with` keeps the node owned until
that typed disposal returns; interrupted disposal retains the node. Ordinary
never-admitted rejection uses the same consuming disposal after successful
pair release. There is no second payload destruction or physical retry after
disposal starts. This does not qualify every outer constructor/syscall context
or arbitrary callbacks during explicit notification.

A published `RetiredEntry` or list destructor retains its entire node/chain
without entering callbacks, allocator, logger or physical cleanup. Only explicit
release destroys a detached payload. There is no abandoned-node/marker recovery
or force-clear API.

## Whole-domain abort

`DomainAbortSweep` acquires an exact-root `AddressSpaceOperation` before
scheduler/publication serialization. Under the publication gate it closes the
root's inline `thread_admission_closed` flag. There is no map insertion or
per-abort allocation. The flag is terminal for that root lifetime; a fresh root
begins unfenced. Ordinary thread preparation rejects it before node/generation
mutation, and stack-slot reservation checks it again. A thread prepared before
the fence still fails publication and releases its owner after guards leave.

User publication takes lifecycle before the publication gate, qualifies its
captured root, and keeps abort/closing admission state stable through master-table
publication. An operation lease alone keeps a root alive but does not prevent a
staged close from installing its admission fence. Closing roots therefore also
reject prepared-thread publication and fresh preparation.

The sweep captures a finite thread-slot ceiling and selects only threads whose
captured address-space handle matches its retained root. It captures each thread
generation under the table and uses `abort_thread_generation` after that guard
leaves. Other domains may publish or recycle TIDs between capture and abort; a
reused generation rejects. The publication gate is not held across the sweep.
No target lifetime can be newly published after the fence.

The sweep captures its executing caller's exact TID/generation and defers that
lifetime while requesting peers. A short `LocalInterruptMask` then spans only
the final self-request and explicit root-operation completion. The local request
qualifies the current LP handle, thread generation and captured root under
LP/thread-table serialization; it neither scans peers nor stages a thread,
sends an IPI, allocates or releases backing. Guards leave before root completion,
and the mask preserves the enclosing IRQ state. No scheduler boundary can
retire that caller between this self-request and release of the sweep's lease.
The outgoing handle/context remains owned until the subsequent ordinary switch.

This qualifies the sweep's own self-request. Concurrent sweeps or unrelated
remote aborts can request its executor earlier; protecting every retained kernel
operation against those requests remains separate context/progress work.

Explicit sweep completion releases its own root lease. Pending contexts retain
their independent stack/root owners and the fence stays closed. Abandonment
retains the operation count/root/fence; no destructor reopens admission or
physically releases backing. Sweep completion means abort requests were issued,
not that threads, devices or roots have reached quiescence.

Forced node/deployment retirement publishes its force request after exact-root
lease admission/fencing and retains that lease through publication and sweep.
The request callback runs outside lifecycle, publication and table guards.
Deployment retirement retains a `Polling` registry claim while releasing the
registry guard before admission; competing retirement observes pending.
Success restores ordinary waiting with `force_requested`; rejection caches
`ThreadAbortRejected` without force-success counters or request publication for
an inadmissible root. Abandonment retains the registry claim and root operation.
Delayed callers retain handles; only the synchronous domain-abort ABI resolves
the current caller's numeric ASID at its entry boundary.

## Evidence and limits

Standalone host tests cover preparation rejection, unused preparation
release, ownership/identity across heads, explicit release and abandonment.
A current-test-thread allocation tracer observes zero allocations/reallocations
during staging, detachment, reversal, filtering, reinsertion and explicit release
of 1,024 prepared payloads. This excludes arbitrary callback allocation.
Two additional tests verify payload disposal before node deallocation and node
retention when the typed disposal is interrupted; the latter uses host unwind
with a deliberately retaining fixture payload, not kernel panic recovery.

Guest fixtures reject both retirement-node and context-storage preparation
64 times each before generation/physical mutation,
then use actual never-scheduled kernel threads and warmed stack tables. They
check executing-stack deferral, independent head retention, exact generations,
transition visibility during callbacks, callback availability of thread/staging
guards and final physical frame recovery. Arm separately checks a synthetic
ownership flag. Private SP/head overrides are confined to these never-scheduled
fixtures; production cannot reap a remote active stack. Masked production entry is separately rejected before claim, and a rejected
real kernel stack pair remains in its original node through two scans without
physical retry. Existing real remote
abort and scheduler-lifecycle verifiers remain enabled.
The scheduled exit-watch fixture also checks that self-exit retains its exact
handle/context and abort request before switching, never resumes it, and
ultimately completes the watched exit. Fast natural-return workers separately
exercise the normal trampoline's exit path.

Guarded abandonment fixtures drop complete never-admitted kernel and user
threads while holding the master thread table, retirement table, address-space
table and physical allocator. Exit callbacks must remain uncalled, waiter and
watch charges remain live, usage and free-frame counts remain unchanged, and
the exact user root rejects close with `OperationsInFlight`. External source
and watch cancellation is checked separately; it cannot recover the abandoned
thread. A selected-generation disposal probe checks completed pair ownership
and availability of those four guards before ordinary metadata destruction.
These fixtures intentionally retain two complete payloads and the user root;
they do not measure all heap deallocations or prove arbitrary caller contexts.

Readiness fixtures additionally stage two owners while scheduler/master-table
guards are held, require deferred callbacks to find those guards and the
retirement guard available, check one coalesced wake with an unrelated LP's
subscription retained, and exercise publication before late registration.
Full-source rejection leaves a never-admitted target unblocked and restores its
original waiter charge. Explicit release recovers the original stack frames;
a failed pair remains nonready and fenced. Existing scheduled exit/remote-abort
tests exercise the production worker after the new event-only wait.

The node is allocated from the global heap and adds one thread-sized allocation
per live/deferred owner. There is no independent node-byte admission pool.
General heap/metadata/principal accounting remains SEC-07 work. Scheduler
run-queue backing no longer grows: [inline run queues](scheduler-run-queues.md)
use each complete thread's admitted payload and qualify exact linkage before
queue/state/affinity publication. The earlier
[selection audit](../reports/audits/2026-10-15-scheduler-selection-storage.md)
removed temporary load/candidate vectors. [Scheduler waiter preparation and
cancellation](scheduler-waiter-budgets.md) now run outside local LP/table guards,
with exact installed-wait qualification and remembered early hints. Lazy list
and IPC/CQ entry preparation also leave their local source guards. Other
subsystem metadata, observer teardown and quantum-timer storage remain
separate allocation paths. Outer construction/submission masks, growth's inner walker
contexts and arbitrary explicit callback contexts remain unqualified;
full SEC-18 lock/quiescence safety is not claimed. There is
no failed-pair retry owner, shared custody adapter or abandoned-owner recovery.

Evidence: [thread retirement audit](../reports/audits/2026-10-07-security-thread-retirement.md).
Complete-payload fallback evidence: [thread metadata retention](../reports/audits/2026-10-12-thread-metadata-retention.md).
Readiness evidence: [event-driven LP reapers](../reports/audits/2026-10-12-reaper-readiness.md).
Shared producer-LP dispatch: [transfer-close readiness](../reports/audits/2026-10-12-memory-close-readiness.md).
Whole-domain evidence: [domain thread abort audit](../reports/audits/2026-10-07-security-domain-thread-abort.md).

Self-handoff evidence: [root-operation completion before caller retirement](../reports/audits/2026-10-09-security-abort-handoff.md).
Its scheduled EL0 fault fixture requests a spinning peer, checks stale-generation
and wrong-root local-request rejection, then yields eight times with the caller
still unrequested. Final masked self-request retains its current handle and Arm
CPU ownership; successful exact-root teardown afterward excludes a leaked sweep
lease in this episode. The original timeout captures remain unresolved evidence.

Published-pair evidence: [explicit stack retirement](../reports/audits/2026-10-09-security-published-stack-retirement.md).

Failed-node diagnostic snapshots copy only the exact thread's captured root, LP,
started phase and reported pair error. They allocate no snapshot collection and
are printed after the registry guard leaves. A detached in-flight batch may be
absent, so absence is not quiescence. Error classification and independent global
shootdown counters do not authorize physical retry or clear the earlier Intel
progress failure; see the [IOMMU follow-up](../reports/audits/2026-10-09-security-iommu-preparation-abandonment.md).

Concurrent-sweep evidence: [exact executor ownership](../reports/audits/2026-10-09-security-abort-executor.md).
`ThreadExecution` captures the current TID/generation and installs its inline LP
fence before the sweep can acquire a root operation. New admission rejects an
already requested lifetime or nested owner. Remote/mutual abort records the
request; `abort_ready` permits retirement only after explicit executor release.
Run-queue requeue/selection, timer wake and owner-LP reaping share that predicate.
Placement keeps the captured LP and migration rejects the owner. Immediate abort
revalidates executor/current-handle state under LP/thread serialization before
queue/table removal. Ordinary rejection completes root then executor; final
completion does the same under the IRQ-preserving handoff mask.

Dropping the executor does not clear its inline fence. Abandonment retains the
stack/root with pending requests, and explicit terminal self-abort yields forever
if ownership still prevents retirement; it cannot return to an unchecked
continuation. Kernel panic still stops the LP without unwinding. This is permanent
retention, not an admitted retry/custody owner or a general kernel cancellation
scheme. The two-fault fixture confirms nested rejection, mutual requests, production
timer sleeps, eight subsequent yields each and exact root teardown; ordinary
root/publication rejection is checked on a scheduled kernel executor.


The [stack-growth execution-owner batch](../reports/audits/2026-10-15-stack-growth-execution-owner.md)
shares that same inline execution fence with `CurrentGrowth`; scheduler/abort/
wake/migration checks use `operation_executor_lp`. The original complete thread/
context/stack/root/charge remains in the master table while one `Growth` owner
retains provisional backing and new-prefix progress outside its guard. Completed
handback revalidates the original episode and publishes progress before releasing
the execution fence. Actual EL0 growth, deferred abort, foreign-leaf rollback,
caller IRQ preservation and table reentry qualify this caller. General heap/
image and other kernel-operation retention remain separate; this is no blanket
cancellation/physical-recovery guarantee.

The [inline-queue follow-up](../reports/audits/2026-10-15-inline-scheduler-run-queues.md)
adds queue membership to the existing complete thread owner. Owner-LP abort
unlinks before table extraction/staging; stack/unstarted release rejects while
linked, without notification or physical work. Lost queue heads cannot authorize
adoption by a new queue. Original roots/charges and complete-payload fallback
remain; this adds no custody, force-clear, physical retry or historical failure
attribution.
