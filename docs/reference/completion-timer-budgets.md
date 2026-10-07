# Completion-timer admission and cancellation

Completion-backed timers have two independent lifetimes: the operation record
and the actual event in an LP's timer queue. Admission accounts for both.
Closing a record cannot return timer-event capacity while its queue node or
retained cancellation backing still exists.

| Limit | Current kernel policy |
| --- | --- |
| Operation submission slots in a domain | Its configured completion capacity; capability and detached operations share it. |
| Completion timer events in a domain | The smaller of completion capacity and 1,024 events. The service loader currently uses capacity 16. |
| Anonymous timer events on a node | 8,192 events shared with sleeps/watchdogs, including cancelled events awaiting physical queue removal. |
| Ordinary-domain share of node events | 6,144 events; kernel/supervisor-designated platform domains may use the remaining 2,048. |

Platform access comes from kernel launch policy. The generation-qualified
designation must match the completion namespace, so ASID reuse cannot inherit
a predecessor's reserved-pool access. The share is not a guaranteed allowance
for every essential service. No application-facing setter or signed deployment
field controls these initial limits.

## Charges follow the event

Timer submission reserves domain and node admission before publishing either
the capability or detached record. Rejection returns `SubmitError::WouldBlock`
without leaking an operation slot or reservation. The existing syscall ABI
returns its submission-failure sentinel; `catten_rt::owned::Completion::timer`
reports `CompletionError::SubmissionFailed` rather than a distinct quota reason.
Cancellation state and its fixed-size queue node are prepared fallibly before
record publication. Insertion allocates nothing. The queue's inline quantum
slot is outside anonymous admission; see [scheduler timer budgets](scheduler-timer-budgets.md).

Relative timeout conversion saturates at the largest representable deadline.
Oversized tick counts are not truncated and addition cannot wrap into an
already-expired deadline. Even a maximum-value timeout remains cancellable.
On x86-64, the APIC timer's 32-bit initial count is bounded to a hardware
checkpoint; the logical deadline stays in the queue and is rechecked after
each interrupt, so a distant deadline does not cause an out-of-range panic or
an early completion. AArch64 programs an absolute 64-bit comparator, as
described in [Arm's Generic Timer guide, §4.4](https://documentation-service.arm.com/static/651fbd69bc48b0381ce0e06c).

Each event reserves one linear charge. Its prepared/queued node, event and
cancellation allocation share lifetime owners for that same reservation; they
do not reserve additional event counts. `queue::OwnedNode` holds admission
outside its Box, freeing the node before returning admission. Moving an event
out of a node retains admission through the returned event's destruction.
Cancellation state uses the charged allocator, so retained handles and weak
references keep admission even after event removal. The private charge holder
is freed before the original reservation is refunded. Queue-node allocation
rejection retains admission while any cancellation backing remains alive.

A fresh completion namespace has a fresh reference-counted budget owner; old
backing retains its original owner through final release, including after
namespace retirement and ASID reuse. Late release cannot credit a replacement namespace. Retained
detached CQ results also keep their existing operation-submission slot until
delivery, independently of whether their timer event has been reclaimed.

## Cancellation and publication

Each timer record owns its cancellation registration. Cancelling a timer marks
the event cancelled and produces a terminal `Cancelled` result immediately.
The event cannot subsequently produce a success result. This makes dropping an
owned hour-long timer cancel and close it without waiting an hour. Other I/O
continues to retain its buffer until its producer supplies a terminal result.

Local cancellation removes the event eagerly when the LP queue is available.
Cancellation from another LP, or from a callback already borrowing that queue,
only flags the event. Its owner purges it on the next queue reconciliation;
admission remains charged until then. Cancellation never blocks on a remote
queue or recursively acquires the current queue's write guard.

The observer and cancellation owner are installed before enqueueing. If
teardown wins before enqueue, the cancelled event is discarded. Timer
callbacks retain a weak reference to their original completion object and
verify that exact object under the registry lock before transitioning it or
publishing a CQ entry. Reusing an ASID and numeric capability does not allow an
old timer callback to complete the new object. Thread-exit and endpoint-close
observers use the same captured-object check rather than resolving their old
numeric handle again.

## Verification and remaining work

Boot-path tests cover domain/node rejection, reserved platform progress,
counter reconciliation, 64 cancellation/close cycles with hour-long timers,
capability/detached shared capacity, maximum-value timeout rollback, deferred queue reclamation, and
captured-callback rejection after exact numeric namespace reuse. Additional
kernel fixtures remove real nodes while retaining cancellation handles and 128
weak aliases, reject new admission until final release, exercise preparation
failure in both handle/event drop orders and check old weak backing against an
exactly reused ASID. See [timer backing audit](../reports/audits/2026-10-07-security-timer-backing.md).
The scoped
EL0 security probe additionally fills its timer capacity, drops the owning
batch within a bounded deadline, churns 64 more hour-long timers and completes
a short timer after recovery. Pure checked event counters and saturating
deadline boundaries run in host tests.
The x86-64 hardware-checkpoint boundary has host tests and build/lint coverage;
it still needs a guest/hardware run.

Completion-domain limits cover capability and detached timers; sleeps/watchdogs
have separate generation-owned domain accounts and share this node pool.
Separate [record admission](completion-record-budgets.md) bounds retained
completion objects and detached results. Scheduler observer entries also have
separate owning admission. Kernel workers, general callback/control-block and
weak-only storage and other kernel metadata still need aggregate admission and
fallible allocation review.
Separate [CQ admission](completion-queue-budgets.md) bounds queue counts and
kernel-owned backing, but not physical ring mappings.
Cross-LP cancellation is bounded by the retained charge, but the regression
simulates deferred reclamation through a busy local queue rather than forcing
an actual remote-LP purge. This is a partial SEC-07 remediation, not a complete
hostile-workload containment claim.
