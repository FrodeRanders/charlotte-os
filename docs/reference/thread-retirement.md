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
ownership predicate remains a stub and runtime migration remains disabled
there. LP assignment alone never authorizes releasing the executing stack.

Detached lists reverse their prepend order to preserve prior insertion-order
notification. Filtering and reinsertion move existing nodes iteratively,
without allocation or recursive destruction. The node retains its complete thread/context while explicit retirement notifies
metadata once and releases the stack pair. Metadata notification precedes root
lease completion, preventing numeric-ASID usage accounting after reuse. Pair
release arms its one-shot fence before detachment/callbacks; any rejection or
interruption is terminal. It retains the original maximum reservation/root/slot
even when one range was released. A failed pair is reinserted in its existing
node and future scans skip physical work; it is never reported as reaped. Only
confirmed pair completion permits explicit node destruction. No physical work
occurs for stack backing in implicit thread/context/stack field destruction; general metadata
fallback in `Thread::drop` is still a separate context boundary.

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

Four standalone host tests cover preparation rejection, unused preparation
release, ownership/identity across heads, explicit release and abandonment.
A current-test-thread allocation tracer observes zero allocations/reallocations
during staging, detachment, reversal, filtering, reinsertion and explicit release
of 1,024 prepared payloads. This excludes arbitrary callback allocation.

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

The node is allocated from the global heap and adds one thread-sized allocation
per live/deferred owner. There is no independent node-byte admission pool.
General heap/metadata/principal accounting remains SEC-07 work. Scheduler
run-queue/migration allocation and other subsystem metadata remain separate
allocation paths. General callback/metadata fallback and outer construction/submission masks
remain unqualified; full SEC-18 lock/quiescence safety is not claimed. There is
no failed-pair retry owner, shared custody adapter or abandoned-owner recovery.

Evidence: [thread retirement audit](../reports/audits/2026-10-07-security-thread-retirement.md).
Whole-domain evidence: [domain thread abort audit](../reports/audits/2026-10-07-security-domain-thread-abort.md).

Published-pair evidence: [explicit stack retirement](../reports/audits/2026-10-09-security-published-stack-retirement.md).

Failed-node diagnostic snapshots copy only the exact thread's captured root, LP,
started phase and reported pair error. They allocate no snapshot collection and
are printed after the registry guard leaves. A detached in-flight batch may be
absent, so absence is not quiescence. Error classification and independent global
shootdown counters do not authorize physical retry or clear the earlier Intel
progress failure; see the [IOMMU follow-up](../reports/audits/2026-10-09-security-iommu-preparation-abandonment.md).
