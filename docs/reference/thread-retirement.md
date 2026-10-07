# Deferred thread retirement

A dying thread still owns its kernel stack after leaving the scheduler table.
Its owner must survive until its LP has switched off that stack. Slot return
uses [publication-prepared metadata](address-space-retirement.md); deferred
thread ownership now has separately prepared storage as well.

## Preparation and transfer

`Thread::try_new` fallibly prepares one `PreparedEntry<Thread>` before claiming
a generation, allocating stacks or publishing a thread. Allocation rejection
returns `ThreadPreparationFailed`. Ordinary construction/publication rejection
drops the unused empty node with the unpublished thread owner. There is no
physical cleanup in the empty node's destructor.

Staging consumes that node out of the live thread and fills it with the entire
thread owner. The contained thread has no second retirement node. Context
boxing remains unchanged, so the assembly's saved-stack pointer stays stable
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

Production reaping selects only the current LP's head. x86 uses its scheduled
pinned reaper, keeping physical shootdown out of IRQ tails. Arm retains its
post-switch boundary. Both defer a node whose stack contains the executing SP;
Arm also defers while the assembly `on_cpu` ownership flag is nonzero. x86's
ownership predicate remains a stub and runtime migration remains disabled
there. LP assignment alone never authorizes releasing the executing stack.

Detached lists reverse their prepend order to preserve prior insertion-order
notification. Filtering and reinsertion move existing nodes iteratively,
without allocation or recursive destruction. Explicit node release invokes
thread callbacks and stack destruction after the staging/thread-table guards
are gone. Stack release still uses its original owning admission and retirement
contract; uncertain physical cleanup retains backing/charges as before.

A published `RetiredEntry` or list destructor retains its entire node/chain
without entering callbacks, allocator, logger or physical cleanup. Only explicit
release destroys a detached payload. There is no abandoned-node/marker recovery
or force-clear API.

## Evidence and limits

Four standalone host tests cover preparation rejection, unused preparation
release, ownership/identity across heads, explicit release and abandonment.
A current-test-thread allocation tracer observes zero allocations/reallocations
during staging, detachment, reversal, filtering, reinsertion and explicit release
of 1,024 prepared payloads. This excludes arbitrary callback allocation.

Guest fixtures reject preparation 64 times before generation/physical mutation,
then use actual never-scheduled kernel threads and warmed stack tables. They
check executing-stack deferral, independent head retention, exact generations,
transition visibility during callbacks, callback availability of thread/staging
guards and final physical frame recovery. Arm separately checks a synthetic
ownership flag. Private SP/head overrides are confined to these never-scheduled
fixtures; production cannot reap a remote active stack. Existing real remote
abort and scheduler-lifecycle verifiers remain enabled.
The scheduled exit-watch fixture also checks that self-exit retains its exact
handle/context and abort request before switching, never resumes it, and
ultimately completes the watched exit. Fast natural-return workers separately
exercise the normal trampoline's exit path.

The node is allocated from the global heap and adds one thread-sized allocation
per live/deferred owner. There is no independent node-byte admission pool.
General heap/metadata/principal accounting remains SEC-07 work. Whole-domain
abort admission/snapshots, scheduler run-queue allocation and other subsystem
metadata remain separate allocation paths. Callback internals and Arm's enclosing
interrupt state are unchanged; full SEC-18 lock/quiescence safety is not claimed.

Evidence: [thread retirement audit](../reports/audits/2026-10-07-security-thread-retirement.md).
