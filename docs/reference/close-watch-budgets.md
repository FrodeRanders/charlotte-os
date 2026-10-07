# Cancellable endpoint-close and thread-exit watches

`Connection::watch_closed()` returns an owned completion for the source
endpoint's death. Dropping that completion now cancels the registration and
finishes locally: the endpoint can remain open indefinitely. Cancellation does
not close the connection or endpoint, and ordinary message delivery does not
complete a close watch.

External thread-exit watches follow the same ownership contract: cancelling
or dropping their completion removes the subscription without terminating the
target or waiting for it to exit. A generation-bound join observes the captured
thread, not a replacement in the same TID slot. Registering against an absent or
stale target completes immediately; exhausted admission does **not** count as
an exit and returns submission failure.

| Registration limit | Current policy |
| --- | ---: |
| Per endpoint, across callers | 128 |
| Per target thread, across callers | 128 |
| Per completion, non-scheduler kernel callbacks | 128 |
| Per submitting completion namespace | Configured completion capacity, clamped to 1,024 |
| Per node | 8,192 |
| Ordinary-domain share | 6,144 |

Endpoint-close, thread-exit and kernel completion-callback entries share the namespace and node pools;
they are not additional allowances per event type. These are independent of
completion-record and submission-slot admission.
The submitting generation sponsors each entry; the watched endpoint's owner
does not grant access to the platform reserve. Kernel-designated platform
submitters share the remaining pool. Limits are kernel policy, not signed
deployment parameters or application-facing setters. The endpoint limit also
means one heavily watched endpoint or thread can reject registration while node
capacity remains available; no per-caller fairness guarantee at either source
is claimed.

## Owning registration and notification

Endpoint and thread lifecycle watches use a fallible one-shot `ObserverList`, separate
from the existing scheduler-readiness observer queues. Each registration is a
fixed-size, individually allocated list entry with a weak observer reference
and an owning charge. There is no growing vector or retained spare list capacity.
Lists use fallible charged allocation and callbacks use `Arc::try_new`; entry allocation uses
`Box::try_new`. The charge sits outside that Box so storage and its weak
reference are destroyed before admission is returned. The kernel enables the
nightly allocator API for these paths.
Registration-count admission bounds this fixed-layout entry storage, not the
entire allocator's metadata or every weak Arc allocation in the kernel.
Thread sources allocate the list lazily on their first subscription. An empty
list/control block can remain for that thread's lifetime; it now consumes separate
[observer-list allocation admission](observer-list-admission.md), through final
token/weak release. It is independent of these entry counters. Global thread/control-block admission remains
separate unfinished work.

The completion owns both its callback and registration token. Cancelling,
completing or destroying it unlinks the entry through the list's own IRQ-safe
lock. Token Drop never enters IPC or completion registries and never invokes a
callback. Removal is bounded by the source's 128-entry ceiling. List and
notification-batch destruction are iterative, avoiding recursive list teardown
on kernel stacks.
Namespace teardown and replacement explicitly release registrations even if
a kernel waiter retains the old completion object; its separate record charge
still follows its allocation through retained strong and weak references.

Endpoint close marks the list closed and detaches a notification batch without
allocation. IPC releases its registry lock before invoking callbacks. A detached
entry stays charged until the batch actually releases it, even if its token is
cancelled first. Token removal cannot recover already-detached storage. The
entry and its weak reference are freed before its callback runs. Cancellation
can race an already captured callback: the completion's terminal transition
and exact-object identity check prevent a late callback from changing a
cancelled result, duplicating completion, or completing a reused capability.

EL0 thread watches require the target's captured address-space identity to
match the caller's exact live generation, including when the expected thread
generation is zero. Foreign user/kernel threads reject before their source is
charged. A separate trusted kernel adapter preserves supervisor cross-domain
observation. Target authorization, generation checking and registration are
serialized with removal from the master thread table. The returned token owns unlinking, using only the
independent list lock. Reaping closes/detaches the list and invokes callbacks
outside source/list/table guards, before the thread's stack is deallocated.
Retaining a registration token after notification cannot retain its former
entry or charge.

## Transactional submission

One kernel owner stages the completion, captured object and registration charge.
Registration validates the connection again after staging, and the endpoint
closed-state check and list insertion share the IPC lock. Missing or revoked
connections, list rejection and allocation failures abort the unpublished
completion and return its slot and charges. Rollback checks the captured
object, rather than revoking whichever capability currently has its number.
Watching an already closed endpoint completes immediately without retaining
a list entry. A callback that wins before its registration owner is installed
does not leave that owner on a terminal completion.
Installation also checks the exact completion object under its namespace guard:
teardown or replacement before a late installation releases the staged token
even if another kernel owner retains the old completion. This fence applies to
endpoint-close and thread-exit watches alike.

## Worker operations are different from joins

`submit_worker` binds its exit callback and owning registration to a newly
constructed kernel thread **before** publication and scheduler admission. A
fast worker cannot outrun a post-spawn registration, and TID recycling cannot
redirect it. Callback/list/entry rejection rolls back the unpublished completion
without running the worker. Kernel stack/thread-table construction retains the
ordinary spawn API's infallible allocation contract.

Cancelling a worker operation retains its registration until actual producer
exit, at which point the terminal result is `Cancelled`. An external join is
only a subscription and can finish locally; a worker operation must preserve
producer-lifetime semantics. Both release their tokens on terminal completion
or namespace teardown. Namespace teardown revokes notification ownership; it
does not assert that a running worker has stopped.

The ordinary completion submission-failure sentinel still reports exhaustion
without identifying its pool; the Rust helper returns
`CompletionError::SubmissionFailed`. Per-endpoint rejection and allocation
errors also use that sentinel. Existing completion-record/capability allocation
is not yet fully fallible: this change makes registration storage fallible,
not the entire submission path immune to allocator panic.

## Verification and remaining work

Synchronous guest tests cover endpoint/domain/node ceilings, staged rollback,
ordinary and platform pools, 512 cancel/rearm cycles on a live endpoint,
ordinary IPC after cancellation, notification and late watches, detached-batch
retention/discard, callback reentrancy, cancellation before a late captured
callback, retirement, repeated client teardown against a persistent endpoint,
and exact numeric ASID/capability reuse during unpublished-owner rollback.
Pool saturation uses reservations, not the maximum corresponding allocation.

The fifteenth scoped EL0 probe bit fills a watch batch, drops it while the
endpoint stays live, churns 128 more owned watches within a five-second budget,
checks that a fresh watch remains pending, then closes the endpoint and observes
its normal result. A short timer verifies recovery. The security mask is now
`0x7fff`. No allocator-failure injection or exhaustive cross-LP interleaving
test is claimed.

Completion/CQ and endpoint-readiness/pending-call scheduler waiters now use
[fallible owned registration](scheduler-waiter-budgets.md) with admission before
parking. Blocking-lock and timer waiters now share that admission too.
Thread-exit fixtures add a full 128-entry source with transactional rejection,
512 cancel/rearm cycles, live target checks, normal exit, stale TID generation,
reentrant exit callbacks, retained tokens/completions, cancellation before
owner installation, worker deferred cancellation, shared-account exhaustion,
and exact ASID/capability reuse during late-install rejection. Scheduled kernel
tests cancel/rearm 128 watches against a live target, observe its eventual exit,
run 32 immediate-return workers and cancel a held worker before release. These
new forced conditions are kernel fixtures; no new real-EL0 probe bit or physical
allocator-OOM injection is claimed.

Non-scheduler timer callbacks use a single embedded slot. Kernel completion
callbacks now use [owned, fallible registration](completion-callback-budgets.md)
with shared event-watch admission, exact-object checking and immediate late
notification outside locks. All scheduler `Observable` sources require owning
registration; no weak-only default remains. Sleep/watchdog events now have
[separate admission](scheduler-timer-budgets.md). General
weak-only Arc/control-block storage and comprehensive kernel heap admission also
remain open. SEC-07 is still partially implemented.
