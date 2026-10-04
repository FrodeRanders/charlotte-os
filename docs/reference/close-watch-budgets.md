# Cancellable endpoint-close watches

`Connection::watch_closed()` returns an owned completion for the source
endpoint's death. Dropping that completion now cancels the registration and
finishes locally: the endpoint can remain open indefinitely. Cancellation does
not close the connection or endpoint, and ordinary message delivery does not
complete a close watch.

| Registration limit | Current policy |
| --- | ---: |
| Per endpoint, across callers | 128 |
| Per submitting completion namespace | Configured completion capacity, clamped to 1,024 |
| Per node | 8,192 |
| Ordinary-domain share | 6,144 |

These are independent of completion-record and submission-slot admission.
The submitting generation sponsors each entry; the watched endpoint's owner
does not grant access to the platform reserve. Kernel-designated platform
submitters share the remaining pool. Limits are kernel policy, not signed
deployment parameters or application-facing setters. The endpoint limit also
means one heavily watched endpoint can reject registration while node capacity
remains available; no per-caller fairness guarantee at that endpoint is claimed.

## Owning registration and notification

Endpoint lifecycle watches use a fallible one-shot `ObserverList`, separate
from the existing scheduler-readiness observer queues. Each registration is a
fixed-size, individually allocated list entry with a weak observer reference
and an owning charge. There is no growing vector or retained spare list capacity.
List and callback allocation use `Arc::try_new`; entry allocation uses
`Box::try_new`. The charge sits outside that Box so storage and its weak
reference are destroyed before admission is returned. The kernel enables the
nightly allocator API for these paths.
Registration-count admission bounds this fixed-layout entry storage, not the
entire allocator's metadata or every weak Arc allocation in the kernel.

The completion owns both its callback and registration token. Cancelling,
completing or destroying it unlinks the entry through the list's own IRQ-safe
lock. Token Drop never enters IPC or completion registries and never invokes a
callback. Removal is bounded by the endpoint's 128-entry ceiling. List and
notification-batch destruction are iterative, avoiding recursive list teardown
on kernel stacks.
Namespace teardown and replacement explicitly release registrations even if
a kernel waiter retains the old completion object; its separate record charge
still follows that retained strong reference.

Endpoint close marks the list closed and detaches a notification batch without
allocation. IPC releases its registry lock before invoking callbacks. A detached
entry stays charged until the batch actually releases it, even if its token is
cancelled first. Token removal cannot recover already-detached storage. The
entry and its weak reference are freed before its callback runs. Cancellation
can race an already captured callback: the completion's terminal transition
and exact-object identity check prevent a late callback from changing a
cancelled result, duplicating completion, or completing a reused capability.

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
parking. Blocking-lock waiters now share that admission too. Thread-exit
observers, watchdogs, timer and other observer paths
still use the old registration API. Its void
return cannot safely be replaced with silent bounded rejection: that would lose
a parked thread's wake source. Their adoption needs integrated admission, owned
cancellation and separate storage accounting. General
weak-only Arc/control-block storage and comprehensive kernel heap admission also
remain open. SEC-07 is still partially implemented.
