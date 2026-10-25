# Owned kernel completion callbacks

Kernel admission capacity for this family comes from the default compile-time
[resource policy](resource-policy.md). Related values can be adjusted together
in a reviewed build profile; see [Configuration](../guides/configuration.md).
Structural/protocol bounds and original-charge/physical-completion rules remain
separate contracts.

Kernel code subscribes using `completion::observe`, which takes a strong
`Arc<dyn Observer>` and returns a `CompletionObservation` owner. Retain the owner
until notification or cancellation. Dropping it removes only the subscription:
the operation remains live, its producer continues, and transferred buffers
remain owned until terminal completion. No syscall exposes Rust callback
objects; applications use `catten_rt::owned`.

Captured-operation preparation uses the owner-local
`observe_registered(asid, cap, &captured, observer)`. It checks exact `Arc`
identity under the namespace guard and rejects a replacement even with identical
ASID/capability numbers. The numeric convenience function captures the current
object and uses the same checked path. Callbacks should likewise use captured
producer identity rather than later resolving stale numbers.

The implementation lives under
[`completion/record/callbacks.rs`](../../crates/catten/src/completion/record/callbacks.rs),
where capture/publication can borrow the record's private state atomically.
The registry and producers receive whole records and installation/terminal
receipts; they do not obtain a raw inner-state guard or buffer view. The
captured helper remains local to the record boundary; the public numeric
`completion::observe` entry point keeps the same checks and return owner.

Capture records the exact completion, namespace/root identity, watch account,
classification and existing callback list under short registry/inner borrows.
Watch admission, lazy list allocation and entry preparation then run outside
both guards. Publication revalidates that record/account/root and retirement,
then checks terminal state and links the prepared entry under the same inner
lock as completion. Competing first initializers use the published list; losing
storage stays in the preparation until post-guard disposal.

## Admission and errors

Each completion accepts 128 non-scheduler callback entries. They share the
[event-watch account and node pool](close-watch-budgets.md) with endpoint-close
and thread-exit entries: configured completion capacity clamped to 1024 per
namespace, 8192 per node and 6144 for ordinary domains. Kernel-designated
platform generations may use the remaining shared pool. Default service
capacity of 16 does not entitle it to 128 linked callbacks.

`ObserveError` distinguishes an absent/retired namespace, a missing/replaced
capability and resource/allocation rejection. Rejection leaves the operation,
buffer and existing subscriptions unchanged. A failed entry returns its staged
charge. List construction uses a fallible charged Arc with separate
[list allocation admission](observer-list-admission.md); entries use `Box::try_new`. Lists are
created lazily on first pending subscription, avoiding callback-list allocation
for scheduler-only completions.

An already-terminal or observed operation invokes its callback immediately,
outside registry/source guards. It needs no entry or charge even when another
operation fills the watch account. This is not a scheduler parking API.
If it becomes terminal during preparation, unused charged storage is disposed
after publication guards leave and before the immediate callback runs. Failure
and a full source likewise leave the complete preparation owned until unlock.

## Lifetime and notification

Terminal checking and insertion share the completion state lock: registration
precedes terminal publication or takes the immediate path, never appending after
one-shot notification has drained. Notification follows CQ publication and runs
outside completion/source guards. Each entry and weak reference are released
before its callback runs. Detached storage remains charged until freed, even if
its token is dropped after detachment. Destruction is iterative.

Retirement rejects new subscriptions. Teardown and unpublished-record rollback
discard callback entries despite retained completion objects. Source destruction
also closes its list, so retained tokens cannot retain abandoned entries. Discard
does not invoke callbacks or fabricate a terminal result.

Successful namespace close/replacement and CQ replacement carry the complete
detached namespace/queue out of `COMPLETIONS` before payload/source destruction.
Authority removal and detached submission-slot accounting remain serialized.
Original record, watch and CQ charges follow the retained payload, including
external strong/weak references. The existing quiescent-consumer/root contract
is still required; an identity reference or discarded subscription is no
physical-release proof.

Terminal completion moves its event and worker subscriptions into one transition
owner. Terminal state and the effective CQ result publish under the registry;
only after unlock does that owner dispose its subscriptions, followed by CQ and
completion notification. The CQ source is captured at publication, so reentrant
subscription destruction cannot redirect the wake to a replacement queue.
Event/worker installation likewise returns replaced or rejected subscriptions
until both inner and registry guards have left. Event cancellation rejects its
late watch; worker cancellation retains the producer-exit watch. Late terminal
installation retains neither. See [terminal-disposal evidence](../reports/audits/2026-10-16-completion-terminal-disposal.md).

This qualifies local completion guards. Timer-observer installation/cancellation,
CQ registry-map/authority metadata, record construction and outer producer/IRQ contexts remain
separate work. No subscription disposal proves physical completion or grants
operation replay authority.

Cancellation can race a callback already captured for notification. Callbacks
must be short, tolerate that race and use captured identity. The owner retains
its strong callback until Drop, even after notification; the token alone does
not retain the operation. Callback captures may retain an operation, whose
separate record charge follows the allocation through both strong and weak
references; see [completion-record admission](completion-record-budgets.md).
List allocation admission covers retained empty list/control-block backing.
Entry counters do not account for arbitrary capture sizes, other weak-only
allocations or the whole kernel heap.

## Scheduler sources are a different contract

`Observable` requires every scheduler source to implement `try_register_waiter`;
the weak-only default and legacy token are gone. Completion/CQ/IPC/lock/timer
sources, steady-state publication and self-test results use
[owning waiter admission](scheduler-waiter-budgets.md). Registration never notifies
inline under the scheduler thread table. Publication detaches before callbacks;
results no longer prune weak entries after timeouts. The timer's embedded internal
callback slot remains separate from this general callback API.

## Validation and unfinished work

Kernel fixtures cover 128 real entries, source/shared-account rejection,
full-batch notification with retained owners, 512 cancel/rearm cycles,
reentrant/late notification, buffer preservation, pending producer cancellation,
rollback with retained objects, retirement and exact numeric namespace reuse.
Injected entry-allocation failure checks charge/weak-reference rollback and
recovery; it is not physical allocator exhaustion.

Both actual boot-status sources have 64-entry rejection, 512 cancel/rearm and
detached/reentrant notification tests. Scheduled fixtures run 64 timed waits on
each source, temporarily substituting only the executing kernel test thread's
sponsor to measure its cleanup independently of other verifiers. Thirty-two
real immediate-return workers exercise registration versus terminal delivery.
The scoped EL0 mask remains `0x7fff`; these added conditions are kernel fixtures.

Comprehensive metadata/heap/page-table/capability admission, principal aggregates
and hostile-pressure validation remain open. CQ payload staging now has its
[CQ payload-preparation follow-up](../reports/audits/2026-10-16-cq-payload-preparation.md); fresh registry-map and authority metadata, record construction,
timer-specific cleanup and other caller/IRQ contexts remain open. This does not close
SEC-07/18. See the
[callback preparation and disposal audit](../reports/audits/2026-10-15-completion-callback-preparation.md).
