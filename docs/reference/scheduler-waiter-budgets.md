# Owned completion and CQ waiters

The scheduler uses fallible `Observable::try_register_waiter` admission before
publishing `Blocked` or removing a Ready thread from its run queue. Completion
and CQ sources return an owning registration; rejection leaves the thread's
state, queue membership and migration constraints unchanged. Already-terminal
completions and unconsumed CQ work return an immediate-ready marker without
parking. Registration never invokes a callback inline while the scheduler holds
the master thread table.

| Live registration limit | Current policy |
| --- | ---: |
| Per completion or CQ source | 64 |
| Per waiting domain generation | 1,024 |
| Per node | 8,192 |
| Ordinary-domain share | 6,144 |

These limits are separate from completion records, endpoint-close watches,
timer events and CQ backing. The waiting thread's domain sponsors the entry,
not the source's owner. Kernel and supervisor-designated platform domains may
use the remaining node pool; no per-service progress entitlement is implied.
The constants are kernel policy, not deployment descriptor overrides.

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
source/list locks. Completion lists close permanently; CQ lists drain and can
be reused. Detached entries stay charged until released by the batch, even if
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

## Migration scope and verification

Only completion/CQ **scheduler waiters** use this admission. The default trait
implementation deliberately returns a marked legacy token and retains the old
registration behavior for timer, lock, pending-call, endpoint-readiness and
other not-yet-converted sources. Raw kernel completion callbacks, thread-exit
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
Allocator-failure injection, exhaustive cross-LP race exploration and sustained
hostile-pressure containment remain unverified. SEC-07 remains partial.
