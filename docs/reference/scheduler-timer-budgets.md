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

One reservation follows an event while prepared, queued and notifying callbacks,
and any cancellation backing retained after event destruction. Event and node
owners share the charged allocator's private holder; no extra event counts are
reserved. The node's outside owner keeps admission until its Box is freed.
Cancellation state is the adapter's one permitted allocation; other clones
retain lifetime only, including strong and weak cancellation references.
The final owner frees the private holder before refunding its original charge.
Anonymous queue nodes are fixed-size `Box` allocations prepared fallibly before
Blocked state or completion publication. The sorted linked queue inserts those
owners without allocation and frees nodes on removal; it does not retain
high-water `VecDeque` backing. Chain destruction and cancellation removal are
iterative to avoid consuming kernel stack proportional to queue length.
Insertion, cancellation scanning and diagnostics remain linear in queue size;
the bound is not a timer-throughput claim.

The quantum/idle-wake event occupies an inline slot in `TimerQueue`. Its
deadline participates in the queue's ordered iteration and earliest-deadline
selection. An existing quantum's deadline is preserved across voluntary yields.
There is no separately maintained `armed` bit, and filling anonymous admission
cannot consume this slot. Hardware comparator mutation remains IRQ-masked.

Cancellation state is fallibly allocated and shared with its handle. Publication
updates the actual owner LP before insertion, so preparation followed by
relocation does not leave a stale cancellation target. Cancellation before
publication flags the state; insertion discards that event. Local available
queues remove cancelled events eagerly. Remote/busy queues retain the flagged
node and charge until owner reconciliation; cancellation never waits on a
remote queue or re-enters a borrowed local queue.

## Failure before parking

Watchdog preparation reserves event admission, allocates cancellation state,
the callback owner and the queue node **before** calling the scheduler's park
operation. Waiter admission is a separate subsequent check. All prepared owners
drop on failure. No event allocation or queue growth remains after parking.
Local IRQ masking still spans park, enqueue and the lost-wake recheck; all
guards and the mask are released before yield.

- Sleep falls back to cooperative runnable waiting until a rebased counter
  deadline on event, node or waiter rejection. It preserves its void ABI and
  at-least-duration contract without publishing a wake-less Blocked state.
- `block_until` returns its condition on failed preparation/admission.
- Timed CQ wait returns `false` without parking. It still cannot distinguish
  admission pressure from no observed work in its existing ABI.
- Timed completion wait returns `WAIT_ADMISSION_FAILED` (3), leaving the
  operation/capability live. The Rust owner retains ownership as documented in
  the waiter reference. Poll and immediate-terminal fast paths are unchanged.
- Capability and detached completion-timer submissions prepare their node and
  fallibly allocate their producer observer before callback registration,
  record/authority publication and enqueue. Observer allocation rejection drops
  the prepared event/node/cancellation state, record and any hidden capability
  reservation. Failure returns `SubmitError::WouldBlock`, with the existing
  submission-failure syscall representation. Registry insertion still uses
  infallible `BTreeMap` allocation; this does not make the entire submission
  path safe under heap exhaustion.

Timed callbacks capture the executing thread's generation before parking;
source registration confirms that same live generation. Prepared nodes hold no
source/queue guard. Enqueue occurs only after releasing completion registries.
Callbacks still execute under the LP queue borrow and must not re-enter it.

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

Scheduled fixtures temporarily substitute only their own kernel thread's
sponsor to force rejection. They check Running state/constraints, generic/CQ
wait failure, a synthetic timed-completion syscall returning status 3 with its
pending capability preserved, runnable sleep duration, busy-local deferred
reclamation and 64 sleep plus 64 watchdog accounting cycles. An owner-LP fixture
simulates relocation before publication; it is not an actual cross-LP purge or
migration test. The scoped real-EL0 verifier is unchanged.

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
