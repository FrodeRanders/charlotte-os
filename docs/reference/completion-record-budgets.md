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

A capability-backed completion retains its charge through the allocation's
lifetime, including weak-only control-block backing. Closing its capability
returns its submission slot, but strong or weak kernel references keep the
record charged. The last strong reference destroys the payload; the last weak
reference permits allocation release and the original reservation's refund.
Merely observing its result does not release either resource. Namespace teardown
can remove the public handle while retained backing remains charged.

`CompletionRef` and `CompletionWeak` use `ChargedAllocator`, whose private charge
holder follows every allocator clone. All holder owners use `Arc::into_inner`,
so its allocation is freed before its one reservation is refunded, including
concurrent final destruction. No weak reference to that holder escapes. Clones
share one allocation allowance and cannot allocate another record or revive a
freed allocation. `ChargedAllocator::try_arc` retains a preparation owner through
failed construction, keeping the charge until the rejected payload is destroyed.
This adapter is for fixed record allocations, not general growable containers.

A detached operation moves its charge into an owning CQ backlog entry if its
result cannot enter the ring. Its operation ID is no longer addressable, but
both submission slot and record admission remain occupied. Ring publication
destroys that retained record and returns both resources. The fixed-size ring
entry remains in independently allocated CQ backing.

Cancelling non-timer work still waits for its producer to post a terminal
result, except [endpoint-close watches](close-watch-budgets.md), whose owning
registration can be cancelled locally. Timer cancellation can terminate the
payload immediately; for a capability-backed timer, its producer's weak
completion reference retains record admission until queue reclamation. A cancelled event on a busy or remote LP also retains its separate
[timer-event charge](completion-timer-budgets.md) until queue reclamation.

Detached terminal publication now returns the complete remaining record,
including its typed timer observer and cancellation owner, beyond registry
unlock. A direct ring result leaves admission with that owner until disposal;
only a backlogged result consumes its original charge into the retained entry.
Disposal precedes notification of the exact CQ source captured at publication.
A missing-CQ record likewise retains its original charge until post-guard
disposal. The detached ID is already unaddressable; duplicate completion rejects
without changing its terminal result. See the [detached timer-disposal follow-up](../reports/audits/2026-10-16-detached-timer-disposal.md).

A firing timer callback still owns its queue node and producer reference while
CQ callbacks run. Its separate event charge survives until those owners leave,
even though the detached record charge can already have been released. Neither
publication nor cancellation is physical quiescence or a replay owner.
Registry-map removal, existing backlog draining and the timer queue's outer
callback context are separate boundaries.

Kernel-controlled CQ replacement discards the old queue's undelivered results.
Discarded detached results now return their submission slots as well as their
record charges. Capability results remain available through their registered
objects. This is teardown semantics, not a lossless queue-migration API; live
applications should not replace an active CQ expecting its old results to move.

Successful namespace close/replacement and CQ replacement now retain their
complete detached payload until the completion registry guard leaves. Source
subscriptions, backlog records and ring/account backing are destroyed afterward;
detached submission slots and authority removal still update under serialization.
CQ payload staging/allocation rejection now runs outside the registry; see the
[CQ payload-preparation follow-up](../reports/audits/2026-10-16-cq-payload-preparation.md). Record construction, authority/map metadata,
timer-specific producer cleanup and outer callers remain separate.
See the [callback/source audit](../reports/audits/2026-10-15-completion-callback-preparation.md).

## Admission, retirement and rollback

Operation identities share one boot-lifetime checked counter across
capability-backed operations, event watches and detached operations/timers.
Only `1..u64::MAX` is issued: zero is excluded and `u64::MAX` remains the syscall
submission-failure sentinel. Once the last usable identity is issued, admission
permanently rejects with the existing `SubmitError::WouldBlock` / submission
failure result. This rejection precedes record/watch/timer admission and
capability reservation; it publishes no operation and enqueues no producer.
Existing operations can still complete, cancel and close normally.

IDs consumed by subsequently rejected preparation are never refunded. Namespace
close/replacement and numeric ASID reuse do not reset the counter. The existing
failure result does not promise that retrying can make progress. See the
[operation-ID exhaustion controls](../reports/audits/2026-10-16-operation-id-exhaustion.md).

Record reservation precedes publishing a capability or detached operation.
Domain/node/ordinary reservations roll back on any later admission failure,
including timer-event exhaustion. The existing `SubmitError::WouldBlock` and
syscall submission-failure sentinel report backpressure; the runtime returns
`CompletionError::SubmissionFailed` without distinguishing the exhausted pool.

The completion registry checks the namespace's exact generation for retirement
before admission. Every namespace has a fresh reference-counted budget owner.
Old strong and weak references retain their original owner after ASID reuse, so late destruction
cannot credit a replacement. Completion close rechecks the captured object's
identity under the registry lock before removing a handle: a delayed close
cannot revoke a different object that reused its numeric capability.

Capability-backed admission takes registry, then domain and node counters.
Detached preparation reserves domain/node counters outside the registry, with
no source or timer-queue guard. Budget guards do not allocate or enter other
subsystems. Lifecycle teardown marks retirement
without retaining the address-space table/budget ledger locks while entering
the completion registry.

Detached plain and timer submissions now capture the exact root, record/timer
accounts, submission capacity, trusted classification and CQ source before
preparation. Record admission, timer cancellation state/node and observer
allocation run outside `COMPLETIONS` in one owner. Unpublished preparations
consume the original limits too; they cannot create an uncharged staging pool.
Publication borrows that owner and rechecks the captured namespace and exact
queue before moving the complete record. A replaced/missing queue returns
`NoCompletionQueue`; a stale/retired namespace returns `UnknownAddressSpace`.
Rejected owners dispose only after registry unlock as unstarted rollback, with
no public retry owner or accepted work to replay. Enqueue consumes accepted
preparation once after unlock; teardown before enqueue leaves the original
node charged until its cancelled insertion is discarded. See the [detached preparation follow-up](../reports/audits/2026-10-16-detached-submission-preparation.md).

Registry-map insertion/removal and backlog reconciliation remain under their
existing serialization. Capability-backed record/timer construction and unified
authority metadata still need separate work. Capture is not a root lease or
physical quiescence proof.

## Verification and remaining work

Synchronous guest tests cover retained strong and weak references after capability
close, 128 weak clones, failed upgrade after payload destruction, record
backpressure until final weak release, mixed capability/detached admission,
non-timer cancellation, retained
results and delivery, CQ replacement and teardown, submission rollback,
timer-event rejection, ordinary and total pool saturation, actual submission
progress for a kernel-designated platform domain, retirement rejection across
all four submission paths, and exact numeric ASID/capability reuse with a stale
captured close and a stale weak allocation charged to the old generation.
Reservation-only pool exhaustion does not allocate the
equivalent maximum record footprint.

The scoped EL0 probe fills its record capacity with owned endpoint-close
watches, checks that timer submission is also refused while scalar IPC still
works, closes the watched endpoint, consumes every completion and waits for a
short timer after recovery. A further probe drops an entire watch batch while
its endpoint remains live, validating local cancellation rather than requiring
endpoint death for destructor progress.
The shared checked counter has host overflow/rejection/release tests. Six host
tests execute the actual allocation adapter: strong/weak lifetime, holder
allocation rejection, rejected payload destruction before refund, prevention of
allocation multiplication/revival, concurrent final weak destruction and
concurrent cloned-allocator admission. Evidence:
[completion backing audit](../reports/audits/2026-10-07-security-completion-backing.md).

These are retained-state counts, not byte accounting for the entire kernel
heap. Separate [CQ admission](completion-queue-budgets.md) bounds registered
queues and kernel-owned ring/backlog backing. Completion record/control-block
storage and its auxiliary charge holder are covered by the record count, not
an aggregate byte budget. Observer-list backing has separate
[allocation admission](observer-list-admission.md). Registry nodes, arbitrary callback
captures and other weak-only Arc/control-block allocations are not covered here.
General metadata allocation and reclamation remain separate work. Endpoint-close and
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

The global operation-ID allocator currently uses a wrapping 64-bit increment.
Its documented nonreuse intent therefore still needs checked exhaustion;
finite admission/reuse fixtures do not exercise a complete counter wrap. This
is a separate recorded boundary in the detached preparation follow-up.
