# Scheduler timer-event admission

Sleep and timed-wait watchdog events have a lifetime distinct from their
[scheduler waiter entries](scheduler-waiter-budgets.md). Cancelling a waiter
does not free an event still held by an LP queue. Both require admission.

| Event limit | Kernel policy |
| --- | ---: |
| Sleep/watchdog events per sponsoring domain generation | 1,024 |
| Completion-backed events per completion namespace | Smaller of its completion capacity and 1,024 |
| Anonymous events per node, shared by both families | 8,192 |
| Ordinary-domain share of that node pool | 6,144 |
| Scheduler quantum/idle-wake storage | One embedded slot per LP, outside anonymous admission |

The two domain accounts are independent, not a single aggregate allowance.
For example, a service with completion capacity 16 can sponsor 1,024 scheduler
events plus 16 completion events, subject to the **shared** node pool.
There are no deployment overrides or application-controlled platform setters.
Kernel/supervisor-designated platform generations may use the node reserve;
it is not a progress entitlement for each service or a fairness guarantee.

## Generation and storage ownership

Thread construction captures the scheduler sponsor from the generation-owned
memory ledger. Timed waits clone it under the thread table, then release that
guard before reservation. Counter operations take domain then node and enter
no other subsystem. Retirement rejects new reservations. Promotion changes
future charges only; an existing ordinary charge remains ordinary until release.
Outstanding events retain their original account after teardown and ASID reuse.
Dropping an old event cannot credit a replacement generation.

One reservation follows an event while prepared, queued, selected and notifying callbacks,
and any cancellation backing retained after event destruction. Event and node
owners share the charged allocator's private holder; no extra event counts are
reserved. The node's outside owner keeps admission until its Box is freed.
Cancellation state is the adapter's one permitted allocation; other clones
retain lifetime only, including strong and weak cancellation references.
The final owner frees the private holder before refunding its original charge.
Anonymous queue nodes are fixed-size `Box` allocations prepared fallibly before
Blocked state or completion publication. The sorted linked queue inserts those
owners without allocation. Removal detaches complete nodes into `RetiredEvents`
without freeing backing or refunding charges under the queue borrow; it does
not retain high-water `VecDeque` backing. Explicit post-queue finish destroys
the detached chain iteratively. Abandoned receipts or queue storage retain
every event/node/source/callback-reference/charge field without cleanup.
Insertion, cancellation scanning and diagnostics remain linear in queue size;
the bound is not a timer-throughput claim.

The quantum/idle-wake event occupies an inline slot in `TimerQueue`. Removed
and duplicate/rejected keyed events transfer by value into the same receipt's
one inline slot; no new disposal node or anonymous reservation is allocated. Its
deadline participates in the queue's ordered iteration and earliest-deadline
selection. An existing quantum's deadline is preserved across voluntary yields.
There is no separately maintained `armed` bit, and filling anonymous admission
cannot consume this slot. Hardware comparator mutation remains IRQ-masked.

Cancellation state is fallibly allocated and shared with its handle. Publication
updates the actual owner LP before insertion, so preparation followed by
relocation does not leave a stale cancellation target. Cancellation before
publication flags the state; insertion returns that complete prepared node
for post-queue disposal. Local available
queues remove cancelled events eagerly. Remote/busy queues retain the flagged
node and charge until owner reconciliation; cancellation never waits on a
remote queue or re-enters a borrowed local queue.

Filtering, purging, expiry and explicit removal now return complete retirement
owners. Keyed ensure/removal, anonymous enqueue, eager cancellation, idle
processing and both architecture IRQ adapters release their local queue borrow
before explicit finish. Comparator/diagnostic updates stay serialized; IRQ
state is preserved. Backing admission may outlive logical
removal through the complete queue transaction, final cancellation references
and weak-only allocation backing. Receipt release creates no new physical,
root-quiescence or replay proof. See the
[queue-disposal follow-up](../reports/audits/2026-10-16-timer-queue-disposal.md).

The [callback-delivery follow-up](../reports/audits/2026-10-16-timer-callback-delivery.md)
now captures the already-due prefix against one monotonic cutoff under the
queue, preserving anonymous FIFO and quantum-first equal-deadline ordering.
Its complete `PendingDelivery` retains selected and purged owners, releases
the borrow, and then invokes each event's callback and waiter notifications
on the captured LP with IRQs masked. Temporary observer/list-entry destruction
and event disposal leave the queue too. No extra allocation, admission or
registry is used. The owner is neither Send nor Sync; runtime checks also
reject callback-continuation migration or loss of the caller's mask.

One inline claim in the existing queue fences nested drains until the current
prefix finishes. Callbacks may enqueue or cancel through the queue API; new
events and deadlines that become due during delivery belong to the next pass.
The comparator is reconciled before notification and again against current
queue state at handback. Each event rechecks its shared cancellation flag just
before starting signal; selected queue absence does not prove completion.
`cancel_event` still returns only actual queue removal, so false also covers
selected/unpublished work. Cancellation does not undo a signal already started.
Fired counters describe expiry selection, not confirmed notification.

Abandonment retains the complete selected/purged owner and leaves its claim
fenced, without cleanup or recovery authority. Explicit callback delivery and
disposal can still execute in IRQ context; unrelated outer caller/producer
guards, raw event/partial preparation, final handles and arbitrary blocking callback
work remain unqualified. No scheduled disposal worker or physical/root proof
is added. This is bounded prefix processing, not a throughput/fairness guarantee.

Eager removal borrows the handle's retained cancellation-state `Arc` and
compares exact allocation identity with the queued event's state. The private
numeric cancellation ID and its global wrapping counter are removed. Equal LP,
deadline or cancellation flags do not identify the same event. A handle keeps
its allocation alive throughout lookup, preventing address reuse from turning
a stale cancellation into authority over a successor; no scalar address escapes.
Weak-only backing retains its original charge but cannot become a new strong
handle after the payload has died. This adds no new admission/exhaustion result
and leaves public operation IDs separate. See the
[exact timer-cancellation evidence](../reports/audits/2026-10-16-timer-cancellation-identity.md).

## Failure before parking

Watchdog preparation reserves event admission, allocates cancellation state,
the callback owner and the queue node **before** calling the scheduler's park
operation. Waiter admission is a separate subsequent check. Unused completed
nodes are explicitly consumed after local publication/registry guards leave;
their abandonment fallback retains complete payloads. No event allocation or
queue growth remains after parking.
Local IRQ masking still spans park, enqueue and the lost-wake recheck; all
guards and the mask are released before yield.

- Sleep prepares its event admission and node before its local park-publication
  mask, retaining any enclosing caller mask. It falls back to cooperative runnable
  waiting until a rebased counter deadline on event, node or waiter rejection. It preserves its void ABI and
  at-least-duration contract without publishing a wake-less Blocked state.
- `block_until` returns its condition on failed preparation/admission.
- Checked timed CQ wait reports admission rejection separately from readiness
  or deadline, retaining the operation owner; see [CQ wait](cq-wait.md).
- Timed completion wait returns `WAIT_ADMISSION_FAILED` (3), leaving the
  operation/capability live. The Rust owner retains ownership as documented in
  the waiter reference. Poll and immediate-terminal fast paths are unchanged.
- Capability and detached completion-timer submissions prepare their node and
  fallibly allocate their producer observer before callback registration,
  record/authority publication and enqueue. Observer allocation rejection
  explicitly disposes the prepared event/node and releases cancellation state,
  record and any hidden capability reservation. Failure returns `SubmitError::WouldBlock`, with the existing
  submission-failure syscall representation. All four completion maps now
  prepare admitted nodes before local publication; their aggregate admission
  and enclosing contexts remain separate. See the
  [global metadata follow-up](../reports/audits/2026-10-16-completion-namespace-metadata.md).

`PreparedEvent` now retains its complete private node behind an inert owner.
Ordinary unused preparation consumes `cancel_unpublished(self)`; enqueue
consumes that same owner into its original queue node. The explicit disposal
path shares the existing complete-event retirement machinery, introduces no
allocation/charge and never signals. Final cancellation strong/weak references
still retain original admission after node disposal. Ordinary scheduler observer
failure, elapsed preparation budget and waiter rejection dispose unused nodes
explicitly; local setup masks leave before that disposal. Capability and detached
submission owners likewise consume unused nodes in their existing post-registry
finish paths. Abandonment preserves the node, cancellation backing, source,
callback reference and admission without allocator, refund or source work.
This is terminal retention, not queue cancellation, Pending, recovery custody
or replay authority. Raw event/partial-construction destruction, final handles,
waiter preparation and enclosing masks/guards remain separate contexts. See the
[unpublished ownership follow-up](../reports/audits/2026-10-16-timer-unpublished-ownership.md).

Timed callbacks capture the executing thread's generation before parking;
source registration confirms that same live generation. Prepared nodes hold no
source/queue guard. Enqueue occurs only after releasing completion registries.
Callbacks execute after the LP queue borrow leaves. They must remain short and
nonblocking, on the original LP with IRQ policy intact; nested queue processing
defers to the current prefix rather than recursively invoking notifications.

Detached completion retirement now carries the complete timer-bearing record
past the completion registry before disposal, then notifies only its captured
CQ source. Direct-result record admission releases with that owner; backlog
admission stays with its entry. A firing timer retains event/node admission
through its enclosing queue callback even after record retirement. This does
not qualify the queue borrow itself or all timer preparation/cancellation
contexts; see the [detached timer-disposal follow-up](../reports/audits/2026-10-16-detached-timer-disposal.md).

Detached submissions now prepare original record/event admission, cancellation
state, timer node and observer outside `COMPLETIONS`, retaining them through
borrowed publication. Exact namespace/CQ identity and retirement revalidate
before record insertion; enqueue follows unlock once. Teardown between those
phases retains the prepared node's event charge until its cancelled insertion
is discarded. Capability-backed preparation and registry-map/authority metadata
remain separate; see the [detached preparation follow-up](../reports/audits/2026-10-16-detached-submission-preparation.md).

## Evidence and limits

Kernel fixtures cover 1,024 real sorted nodes, reuse after removal, iterative
filter/destruction, the inline quantum, shared node/ordinary saturation
(counter-only at the node maximum), platform promotion, retirement and exact
ASID reuse. A deterministic queue-node allocation failure checks charge rollback;
it is not a physical allocator-exhaustion test.

Completion fixtures inject producer-observer allocation failure after real
event/node and record preparation, for both capability and detached timers.
Repeated rejection restores capability, record, timer and list counts without
publishing a CQ result or disturbing an existing pending operation. A retained
weak reference to the rejected record keeps its original charge until final
release; real immediate expiry and cancellation recover afterward. The injected
allocator is private to the kernel fixture and substitutes only that allocation.

Scheduled fixtures receive a private trusted sponsor before thread publication
and any event capture; no live sponsor is replaced. They check Running state/constraints, generic/CQ
wait failure, a synthetic timed-completion syscall returning status 3 with its
pending capability preserved, runnable sleep duration, busy-local deferred
reclamation and 64 sleep plus 64 watchdog accounting cycles. An owner-LP fixture
simulates relocation before publication; it is not an actual cross-LP purge or
migration test. The scoped real-EL0 verifier is unchanged.

Serialized disposal controls detach 64 actual exact-identity nodes with heap
and queue held, retaining all charges and waiter sources. Additional controls
cover cancelled-node purging plus rejected enqueue, anonymous/keyed expiry,
duplicate and removed quantum handback, final weak-only original-account
release and enclosing IRQ preservation. The shared explicit finish probes
local queue and heap guard availability immediately before destruction.
One guarded abandoned receipt deliberately retains one original event charge
and source until guest shutdown; later independent waiter cancellation releases
only its own entry. This negative control runs after the fixture's full-pool
reservation checks and never resets node counters. It does not qualify every
production outer caller or prove cross-LP cancellation progress.

Callback controls select 64 real due nodes with heap and queue held, then check
ordered post-queue callbacks and waiter delivery. One selected event is cancelled
before its signal check; another queued future event is removed reentrantly.
Immediate callback enqueue remains pending across a nested drain and runs on
the next pass. Private comparator fixtures check quantum-first ties and stable
anonymous FIFO. Final strong observer destruction for both callback and waiter
paths reenters the queue and checks heap availability. A guarded private claim
abandons one selected plus one purged event, retaining two original event
charges and source allocations; independent waiter cancellation refunds no
event backing. No real LP claim or live sponsor is abandoned in that control.

Unpublished-node controls explicitly dispose 64 complete watchdog preparations
with live callback/waiter references, preserve weak-only original charges, and
exercise noncancellable sleep-node disposal beneath an existing caller mask.
Guarded completed-owner abandonment retains one event charge, its callback
reference and source allocation until shutdown; independent waiter cancellation
releases only its entry. All ordinary paths restore their baseline separately.

These counts bound retained event/node/cancellation backing quantities, not
the entire kernel heap or allocator overhead. Queue diagnostics count actual
membership, while event admission can remain occupied by cancellation backing
after removal. Independent waiter-list backing has separate
[allocation admission](observer-list-admission.md). Sponsor allocations,
general callback metadata and other weak-only storage remain incomplete.
Evidence: [timer backing audit](../reports/audits/2026-10-07-security-timer-backing.md).
Observer failure correction:
[timer observer allocation audit](../reports/audits/2026-10-07-security-timer-observer-allocation.md).
Aborted sleepers can retain
charged event storage until the original deadline. Per-principal aggregates,
deadline-indexed cancellation and production fairness remain future work.
SEC-07 remains partial; this is not hostile-workload containment certification.
