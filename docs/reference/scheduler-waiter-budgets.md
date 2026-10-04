# Owned scheduler waiter budgets

The scheduler uses fallible `Observable::try_register_waiter` admission before
publishing `Blocked` or removing a Ready thread from its run queue. Completion,
CQ, endpoint-readiness and pending-call sources return an owning registration;
rejection leaves the thread's state, queue membership and migration constraints
unchanged. Already-terminal completions, unconsumed CQ work, readable/closed
endpoints and replied calls return an immediate-ready marker without parking.
Registration never invokes a callback inline while the scheduler holds
the master thread table.

| Live registration limit | Current policy |
| --- | ---: |
| Linked entries per completion, CQ, readiness or pending-call source | 64 |
| Per waiting domain generation | 1,024 |
| Per node | 8,192 |
| Ordinary-domain share | 6,144 |

These limits are separate from completion records, endpoint-close watches,
timer events and CQ backing. The waiting thread's domain sponsors the entry,
not the source's owner. Kernel and supervisor-designated platform domains may
use the remaining node pool; no per-service progress entitlement is implied.
The domain/node pools are shared across these four migrated waiter categories.
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
allocation are now fallible. Pending-call/reply-token/connection records, list
control blocks and other general metadata still need comprehensive admission.

## Migration scope and verification

Only the four migrated **scheduler waiter** categories use this admission. The
default trait implementation deliberately returns a marked legacy token and
retains the old registration behavior for timer, lock and other not-yet-converted
sources. Raw kernel completion callbacks, thread-exit
observers and watchdog storage are not covered. Silent bounded insertion on
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
Allocator-failure injection, exhaustive cross-LP race exploration and sustained
hostile-pressure containment remain unverified. SEC-07 remains partial.
