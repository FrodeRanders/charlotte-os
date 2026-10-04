# Completion-record admission

All completion submissions reserve record admission: buffer-bearing operations,
timers, thread-exit and endpoint-close watches, and capability-free operations.
The existing submission-slot limit and this record budget measure different
lifetimes.

| Limit | Current kernel policy |
| --- | --- |
| Submission slots per namespace | Its configured completion capacity, clamped to 1,024 |
| Retained records per namespace | The smaller of that capacity and 1,024 |
| Retained records per node | 8,192 |
| Ordinary-domain share | 6,144; the remaining pool is available to kernel-designated platform domains |

The service loader currently configures 16 submission slots. These policies
are independent of endpoint counts and socket capacity, and have no signed
deployment override or application-facing setter. Platform classification is
captured against the namespace's exact ASID and generation; names, roles and
application descriptor fields cannot grant reserve access. The platform pool
is shared, not a guaranteed allowance for each essential service.

## Charges follow retained state

A capability-backed completion owns its charge until its last strong reference
is destroyed. Closing its capability returns its submission slot, but a kernel
waiter or captured callback retaining the object keeps the record charged.
Merely observing its result does not release either resource. Namespace teardown
can remove the public handle without destroying a retained object.

A detached operation moves its charge into an owning CQ backlog entry if its
result cannot enter the ring. Its operation ID is no longer addressable, but
both submission slot and record admission remain occupied. Ring publication
destroys that retained record and returns both resources. The fixed-size ring
entry remains in independently allocated CQ backing.

Cancelling non-timer work still waits for its producer to post a terminal
result, except [endpoint-close watches](close-watch-budgets.md), whose owning
registration can be cancelled locally. Timer cancellation can terminate the record immediately, but a cancelled
event on a busy or remote LP retains its separate
[timer-event charge](completion-timer-budgets.md) until queue reclamation.

Kernel-controlled CQ replacement discards the old queue's undelivered results.
Discarded detached results now return their submission slots as well as their
record charges. Capability results remain available through their registered
objects. This is teardown semantics, not a lossless queue-migration API; live
applications should not replace an active CQ expecting its old results to move.

## Admission, retirement and rollback

Record reservation precedes publishing a capability or detached operation.
Domain/node/ordinary reservations roll back on any later admission failure,
including timer-event exhaustion. The existing `SubmitError::WouldBlock` and
syscall submission-failure sentinel report backpressure; the runtime returns
`CompletionError::SubmissionFailed` without distinguishing the exhausted pool.

The completion registry checks the namespace's exact generation for retirement
before admission. Every namespace has a fresh reference-counted budget owner.
Old objects retain their original owner after ASID reuse, so late destruction
cannot credit a replacement. Completion close rechecks the captured object's
identity under the registry lock before removing a handle: a delayed close
cannot revoke a different object that reused its numeric capability.

Lock order is registry, then domain counter, then node counters. Budget guards
do not allocate or enter other subsystems. Lifecycle teardown marks retirement
without retaining the address-space table/budget ledger locks while entering
the completion registry.

## Verification and remaining work

Synchronous guest tests cover retained strong references after capability
close, mixed capability/detached admission, non-timer cancellation, retained
results and delivery, CQ replacement and teardown, submission rollback,
timer-event rejection, ordinary and total pool saturation, actual submission
progress for a kernel-designated platform domain, retirement rejection across
all four submission paths, and exact numeric ASID/capability reuse with a stale
captured close. Reservation-only pool exhaustion does not allocate the
equivalent maximum record footprint.

The scoped EL0 probe fills its record capacity with owned endpoint-close
watches, checks that timer submission is also refused while scalar IPC still
works, closes the watched endpoint, consumes every completion and waits for a
short timer after recovery. A further probe drops an entire watch batch while
its endpoint remains live, validating local cancellation rather than requiring
endpoint death for destructor progress.
The shared checked counter has host overflow/rejection/release tests.

These are retained-state counts, not byte accounting for the entire kernel
heap. Separate [CQ admission](completion-queue-budgets.md) bounds registered
queues and kernel-owned ring/backlog backing. Registry nodes, worker stacks, observer lists,
and weak-only Arc/control-block allocations are not separately charged here.
In particular, a weak reference can retain allocation storage after the strong
object's fields and charge have dropped. Observer cancellation/reclamation and
fallible metadata allocation still require hardening. Endpoint-close and
thread-exit registrations now share [owning lists and event-watch admission](close-watch-budgets.md).
Completion/CQ/IPC scheduler
waiters also have [separate owning admission](scheduler-waiter-budgets.md);
blocking-lock, timer and boot-status waiters share it too. Kernel completion
callbacks now have [owned event-watch admission](completion-callback-budgets.md).
General weak-only/control-block backing remains open.
[Sleep/watchdog event admission](scheduler-timer-budgets.md) is separate and
shares the completion-timer node pool.
Per-principal totals
across domains, typed deployment limits and userspace counters remain future
work. SEC-07 is partial; no hostile-workload containment guarantee is made.
