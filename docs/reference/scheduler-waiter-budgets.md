# Owned scheduler waiter budgets

Connection, retained-call and reply-token counts have separate
[IPC record admission](ipc-record-budgets.md). Waiter entries count a different
lifetime: the thread's parked registration, not the operation it waits for.

The scheduler uses fallible `Observable::try_register_waiter` admission before
publishing `Blocked` or removing a Ready thread from its run queue. Completion,
CQ, endpoint-readiness, pending-call, blocking-lock and timer sources return an owning registration;
rejection leaves the thread's state, queue membership and migration constraints
unchanged. Already-terminal completions, unconsumed CQ work, readable/closed
endpoints and replied calls return an immediate-ready marker without parking.
Registration never invokes a callback inline while the scheduler holds
the master thread table.

| Live registration limit | Current policy |
| --- | ---: |
| Linked entries per completion, CQ, IPC, blocking-lock or timer wait source | 64 |
| Per waiting domain generation | 1,024 |
| Per node | 8,192 |
| Ordinary-domain share | 6,144 |

These limits are separate from completion records, endpoint-close watches,
timer events and CQ backing. The waiting thread's domain sponsors the entry,
not the source's owner. Kernel and supervisor-designated platform domains may
use the remaining node pool; no per-service progress entitlement is implied.
The domain/node pools are shared across all migrated waiter categories.
RwLock reader and writer sources each have a separate 64-entry linked ceiling;
a mutex has one source. Their combined entries still share the same domain/node pools.
The constants are kernel policy, not deployment descriptor overrides.
The source ceiling bounds its currently linked list. A reusable source can
rearm while an older notification batch remains detached; those detached
entries still occupy the domain/node pools, not the source's linked count.

## Lifetime and locks

Thread construction captures the generation-owned sponsor from the memory
ledger. Parking reserves through independent domain/node counter locks; it
does not look up the address-space table or enter the memory ledger under the
master table. Retirement closes that sponsor to new admission. Old entries
retain their original account even after numeric ASID reuse. Platform promotion
affects future waiter reservations only: an existing ordinary charge remains
ordinary until released.

The thread's Waker owns its registration token. A successful Blocked-to-Ready
transition explicitly cancels it, including when a watchdog wins and another
strong Waker reference survives. Aborted Blocked threads cancel when their
staged Thread is actually reaped, not necessarily when abort is requested.
Token destruction enters only the independent list lock, never a source
registry or callback. Removed entries are freed outside that lock.

Sources detach notification batches without allocating, then notify outside
source/list locks. Completion and pending-call lists close permanently; CQ and
endpoint-readiness lists drain and can be reused until source closure.
Detached entries stay charged until released by the batch, even if
their tokens are cancelled first. Entry and weak-reference storage are freed
before invoking callbacks. Removal is bounded by the source's 64-entry ceiling;
destruction is iterative rather than recursively consuming kernel stack.

Completion/list/Waker allocations use fallible allocation in the migrated
path. Counts bound the fixed-size entry storage, not allocator overhead, every
weak-only Arc/control block or all associated registry metadata.

Namespace destruction explicitly discards registrations even when an old
completion is retained. CQ replacement/destruction still requires a quiescent
consumer; discarding a source does not promise to resume an arbitrary live
reactor or preserve its work. Domain teardown must abort and quiesce threads.

## Application contract under pressure

Shared kernel condition waits, timed completion syscalls and timed CQ waits
mask local IRQs across parking, watchdog enqueue and the condition recheck.
Otherwise a quantum interrupt can switch out a newly Blocked waiter before
its deadline wake exists. `LocalInterruptMask` restores the entry IRQ state
on every early return and is explicitly dropped before yielding. It owns no
lock and cannot be sent to another LP. Watchdog storage itself is still outside
these waiter-entry budgets.

Timed completion waits return `completion_status::WAIT_ADMISSION_FAILED` (3)
when parking fails. The operation is still live. `Completion::wait_timeout`
returns `CompletionError::Status(3)` **without closing or consuming its owner**:
retry, poll, or drop it to cancel and wait for terminal cleanup. Poll never
returns this status. A timeout likewise preserves ownership.

The untimed, void completion-wait ABI cannot report admission failure to a
caller that expects borrowed buffers to become safe on return. It retries by
cooperatively yielding while runnable, and rechecks terminal state after every
wake. This is a safety fallback, not efficient idle waiting or a guarantee of
producer progress under overload. `ReadOperation` retains its buffer borrow
through cancellation and terminal waiting.

CQ wait ABIs retain their existing return shapes: admission failure returns
without parking (timed waits report no observed work). They do not expose a
distinct exhaustion status. Callers must recheck their work condition; these
ABIs already permit deadline returns without work.

## IPC receive and reply waits

`wait_readable` and `wait_reply` use owning registration under the IPC registry
lock. A message drains the endpoint's current receiver batch. Reply, reply-token
Drop, endpoint death or pending-call close closes/detaches the affected call
batch. IPC releases its registry before notifying any scheduler observer. An
endpoint-close operation can splice multiple detached batches without allocating
a callback vector, and detached entries retain their individual charges.
Source destruction discards residual entries without callbacks under IPC.

Untimed receive/reply waits cooperatively yield and retry if admission is
rejected. A reply wait does not report a transient quota failure as the result
of a still-live call: a server may still have a delegated memory loan. Reply or
explicit cancellation revokes that loan before the caller can release its Rust
borrow. No new IPC ABI status or userspace owner is needed. Kernel-only timed
reply waits can return `false` on admission failure while retaining the call.
The two debugger counters `IPC_WAIT_ADMISSION_RETRIES` record receive/reply
fallback attempts; they neither grant authority nor drive policy.

Endpoint creation prepares its readiness list fallibly before publishing the
endpoint. All six call submission variants prepare their pending-call list
before moving, copying, lending or vector-transferring memory and before minting
delegated connection attachments. List allocation failure returns before these
effects. This is not a claim that capability/registry insertion and every later
allocation are now fallible. Separate IPC record admission bounds call, reply
and connection counts; list control blocks and other general metadata still
need comprehensive admission and fallible allocation.

## Kernel blocking locks

`cpu/scheduler/sync/{mutex,rwlock}` now use owning waiter registrations rather
than unbounded queues of weak observers. These are the kernel's scheduler-blocking
locks, not the interrupt-masking spin locks used by registries and allocators.
There are currently no production callers of the blocking family; the new
kernel fixtures exercise it directly. This is infrastructure hardening, not a
change to service IPC or a production lock-performance result.

A const-initializable `WaiterSource` prepares its list fallibly on first
contention. Uncontended data-lock acquisition allocates nothing. Initialization
uses a short independent spin guard; admission/entry allocation follows after
that guard is released. The list control block remains with the lock source
until destruction, outside the entry count. Source destruction closes/discards
entries even if registration tokens retain the list. A free mutex can report
ready without retaining a registration; normal acquisition always retries CAS.

Exhausted admission leaves the caller runnable. `lock()` yields cooperatively,
retries the CAS and attempts owning registration again. It does not claim to
have acquired the lock or park without a wake. The existing post-registration
state checks cover unlock-before-registration races. Source/initialization and
scheduler guards are gone before `yield_lp()`. A local interrupt-mask owner
spans park and that recheck: otherwise a quantum could switch out a Blocked
caller before it observes an unlock that preceded insertion, with no later
notification to resume it. The mask restores entry IRQ state on rejection
and is dropped before yielding. The debugger-only
`LOCK_WAIT_ADMISSION_RETRIES` counters distinguish mutex, shared RwLock and
exclusive RwLock fallback; they neither grant authority nor drive policy.

Unlock releases data ownership and detaches all linked candidates before any
callback. RwLock detaches both writer and reader batches before notifying either;
shared unlock notifies only when the final reader leaves. Notifications are
hints, not reserved ownership handoffs. A cancelled or expired writer therefore
cannot suppress waiting readers. CAS chooses the next owner; there is no FIFO,
writer-priority, starvation-freedom, priority-inheritance or owner-death recovery
guarantee. Broadcasts can wake up to 64 mutex candidates or 128 RwLock candidates,
and are not a throughput optimization. Detached entries retain budget charges.

## Timer observers and sleep

`TimerEvent` scheduler registrations use the same owning source and shared
entry budgets. Wake, competing watchdog admission to Ready, thread reaping and
event destruction cancel/release them. A cancelled event suppresses callbacks;
its entries remain charged until cancellation or actual event destruction.
Notification detaches the waiter batch and the internal callback before invoking
either, with no source guard held. Timer-queue processing still owns its LP-local
queue borrow during callbacks: callbacks must not re-enter that queue.

All current non-scheduler timer producers (quantum, completion timers and timed
wait watchdogs) install exactly one callback. Its weak reference now lives in
an embedded single slot, not an unbounded queue. A second internal registration
is a kernel programming error, not silently rejected wake delivery. The slot
does not allocate observer-list backing or consume waiter-entry admission.
It is not a general multi-callback registration API; thread-exit and raw
completion callback sources retain their separate legacy behavior.

Sleep preserves its void ABI and at-least-duration contract. If scheduler
registration fails, it discards the unqueued event, restores the entry IRQ state
and cooperatively yields until a counter deadline while remaining runnable.
`SLEEP_WAIT_ADMISSION_FALLBACKS` is diagnostic only. Successful parking still
rebases the interval after admission and queues the timer before restoring IRQs.
This fallback avoids a kernel panic or false early success; it is not efficient
idle waiting or an overload progress guarantee.

These entry limits are separate from [sleep/watchdog event admission](scheduler-timer-budgets.md).
That admission shares the completion-timer node pool and prepares fixed-size
queue nodes before parking. Aborted sleeps may retain their charged event and
empty source/control block until the deadline, without a linked weak waiter.
General timer/control-block accounting remains incomplete.

## Migration scope and verification

Only the migrated **scheduler waiter** categories use this admission. The
default trait implementation deliberately returns a marked legacy token and
retains the old registration behavior for not-yet-converted sources.
Raw kernel completion callbacks and thread-exit
observers are not covered. Silent bounded insertion on
those old paths would lose wake sources and is not an acceptable conversion.

Synchronous tests cover source/domain/ordinary/node rejection and rollback,
local cancellation, detached batch retention/discard, callback reentrancy,
rearming, platform promotion, retirement and exact ASID reuse. Node saturation
is counter-only; it does not allocate the maximum entry footprint. Scheduled
tests check non-mutating Running/Ready/new rejection, wake/reap cancellation
despite a retained Waker, and 64 completion/CQ timeout cleanup cycles alongside
normal CQ wakes.
Host ownership tests check retry and Drop after timed admission failure.
IPC tests cover both source ceilings, 512 receiver cancel/rearm cycles,
message/reply/closure notifications, reentrant callbacks, reply-token Drop,
call cancellation and loan revocation, retired sponsorship and replacement
ASID accounting. Scheduled tests check 64 reply/readiness timeout cycles,
non-mutating full-source rejection, and forced untimed receive/reply recovery
while a real read loan remains live until reply. These are kernel fixtures, not
forced-quota real-EL0 ABI tests.
Lock fixtures cover source ceilings/rollback, ready acquisition, 512 cancel/rearm
cycles, detached retention, source destruction with retained tokens, retirement,
callback reentrancy, expired writers and final-reader notification. Scheduled
tests add 64 timed cleanup cycles and non-mutating rejection, force all three
lock-acquisition fallback counters before remote workers park, and verify
mutex/read/write data access after release on another LP. The holders only poll
while owning data guards; they never explicitly park/yield with those guards.
These are not lock userspace-ABI tests or an exhaustive fairness/race proof.
Timer fixtures cover source limits/rollback, 512 cancel/rearm cycles, callback
reentrancy, expired internal callbacks, cancelled-event suppression, event
destruction with retained tokens and retired sponsorship. Scheduled tests run
64 normal sleeps, 64 competing-watchdog cleanup cycles, non-mutating rejection
and a forced full-source runnable sleep fallback with an elapsed-duration check.
The pressure fixture supplies a pre-filled timer source to the normal sleep
implementation; it does not saturate a real EL0 domain or fault-inject allocation.
Allocator-failure injection, exhaustive cross-LP race exploration and sustained
hostile-pressure containment remain unverified. SEC-07 remains partial.
