# Observer-list allocation admission

Every `ObserverList` allocation reserves a separate node slot before allocating
its charged owner or list backing. These slots cover empty list/control-block
storage retained by sources, registration tokens and weak references. Scheduler
waiter and event-watch entry charges account for a different lifetime.

| Limit | Kernel policy |
| --- | ---: |
| Retained list allocations per node | 8,192 |
| Ordinary share | 6,144 |
| Platform reserve | Remaining shared pool; no per-service entitlement |

There is no additional per-domain list account or signed override. Existing
completion, CQ, endpoint, call, waiter and watch limits still apply separately.
The new pool bounds list allocation quantities, including the auxiliary charge
holder, not allocator overhead, arbitrary callbacks or all kernel heap bytes.

## Classification and lifetime

Eager completion waiter lists capture the completion record charge's platform
classification. CQ lists and endpoint readiness/close lists use their existing
generation-qualified source admission. Pending-call lists use the captured call
charge. Lazy completion callbacks and thread-exit lists use the first admitted
event-watch charge; other lazy `WaiterSource` lists use the first waiting
generation's captured sponsor, with the first published candidate fixing that
list's classification. Retirement checked by that sponsor rejects fresh
lazy initialization. No list creation resolves a reusable numeric ASID under
its source lock or accepts an application role/name as reserve authority.

An existing list retains its original classification through promotion, later
subscribers and source destruction. A platform subscriber does not reclassify
an ordinary list. A source initially allocated by a platform subscriber stays
platform-charged; there is no cross-subscriber fairness or principal attribution
claim for this shared metadata.

`ListRef` is the kernel-private owner, wrapping a charged Arc. Registration
tokens retain this owner even after their entry is detached or discarded.
Closing or draining frees entry charges when their owning batches release them;
it does not refund a still-retained list. The final strong reference destroys
the list payload, while weak-only backing continues to hold admission. Final
allocation release and private charge-holder deallocation precede refund.
The allocator's clones cannot admit multiple allocations or revive freed backing.

The [shared holder constructor](../reports/audits/2026-10-16-timer-event-construction.md)
now retains the original charge in an inert preparation while allocating empty
Arc storage. No charge is handed to the allocator callback; private storage
initializes once. Ordinary owning rejection cancels after callback return, and
abandonment retains all phase fields. The parent-borrowed form retains its
reservation on failure for ordered payload disposal. Published list accounting,
single-allocation allowance and outward errors stay unchanged. The shared
[payload constructor](../reports/audits/2026-10-16-charged-arc-payload-construction.md) now
retains owning values outside empty allocation, initializes once and cancels
ordinary rejection after callback return; inert phase fallback retains every
field. The outer list factory, source destruction and wider caller guards/masks
remain separate work.

Entry construction now captures its exact weak observer and original entry
charge before allocating empty `Box<MaybeUninit<EntryNode<C>>>` storage. The
allocator owns no payload; failure leaves both fields with the same inert owner
until explicit post-callback cancellation. Private initialization consumes the
same storage once without allocation, locks or disposal. Cancellation frees
unused backing and the weak reference before original admission; abandonment
before/after storage retains every field without cleanup. This is terminal
retention, not a registered waiter or retry owner. Completed `PreparedRegistration`
now also uses inert abandonment and explicit finish as described below.
Registration tokens, source/batch destruction and wider containing caller contexts
retain their existing policies and separate qualification. See the
[entry-construction follow-up](../reports/audits/2026-10-16-observer-entry-construction.md).

Completion callback preparation now also retains its exact captured record,
original weak observer, watch reservation and candidate/entry fields in one
inert containing owner before admission. Ordinary rejection and successful/ready
publication finish explicitly after local guards; readiness notification follows
that disposal. Its four phase fallbacks retain complete fields. This qualifies
that containing owner, while active linked-entry/token/source
finalization and other caller contexts remain separate. See the
[callback-owner follow-up](../reports/audits/2026-10-16-completion-callback-ownership.md).

## Rejection and locks

Lazy `WaiterSource` initialization now captures its original sponsor and weak
observer before list work in one inert containing owner. Candidate and selected
winner stay with it through first-winner publication; losing storage releases
after the source-slot guard and before waiter admission. The shared waiter child
then captures that winner/sponsor. Ordinary results finish explicitly, and six
handoff-stage fallbacks retain complete fields, including emptied child metadata
after token publication. Inner list-factory, active source/token/batch destruction
and wider contexts remain separate. See the
[lazy-source owner follow-up](../reports/audits/2026-10-16-lazy-waiter-source-ownership.md).

Completed private `PreparedRegistration` retains its complete entry on
abandonment too. Direct registration and callback/exit-watch/waiter containing
owners explicitly finish unused or emptied preparations after local guards.
Publication transfers the same entry once; finish cannot refund a published
entry. Ordinary private disposal preserves weak/node-before-charge order.
Active linked-entry/token/source/batch destructors and wider contexts remain
separate. See the
[completed-entry follow-up](../reports/audits/2026-10-16-completed-observer-entry-ownership.md).

Shared waiter preparation captures its exact source, sponsor and weak observer
before entry admission in one inert owner. Entry factories borrow it; ordinary
outcomes explicitly finish after local guards, source before sponsor and after
transient entry/admission disposal. All three phase fallbacks retain complete
fields through source close and sponsor retirement. This qualifies that
containing owner, while active linked-entry and published token/source/batch
destruction and other caller contexts remain separate. See the
[waiter-owner follow-up](../reports/audits/2026-10-16-waiter-preparation-ownership.md).

Thread-exit watch preparation similarly retains its captured candidate list,
original weak callback, admitted watch charge and any completed entry in one
inert owner before list/entry allocation. Its scheduler hook borrows the owner;
ordinary outcomes explicitly finish after local guards, releasing the candidate
source last. Four phase fallbacks retain every field. This does not qualify
active linked-entry/token/source destruction or initial capture and
wider caller contexts. See the
[exit-watch owner follow-up](../reports/audits/2026-10-16-thread-exit-watch-ownership.md).

`ObserverList::try_new` has no uncharged alternative. Node/ordinary saturation
returns `RegistrationError::ResourceLimit`; fallible holder/list allocation
returns `AllocationFailed`. Reservation rolls back on rejected allocation.
Sources and registries preserve their existing outward failure shapes:

- Completion submission returns `SubmitError::WouldBlock` and publishes no cap.
- CQ preparation returns `CqOpenError::AllocationFailed`, preserving an existing
  queue. The CQ API currently groups list quota rejection with allocation errors.
- Endpoint/call preparation and callback/watch installation report their existing
  resource rejection before publication or attachment mutation.
- Lazy waiter rejection leaves the thread runnable through existing parking
  policy; a failed first initialization can be retried after admission recovers.

The new node counter holds only its independent spin guard. It neither allocates,
enters another subsystem nor looks up an address-space generation. Reservation
and refund do not retain that guard across list allocation/destruction or
callbacks. Lazy `WaiterSource` list preparation now runs outside its slot mutex;
publication borrows candidate storage and installs only the first winner.
Losing candidates retain their original charge until post-guard disposal.
Entry admission follows using the winning list outside that mutex. A failed
entry can leave an already initialized empty source charged, as before.

Scheduler IPC/CQ registration captures its exact list, prepares the entry
outside the registry and revalidates identity/readiness before borrowed,
allocation-free publication. Ready/rejected preparations dispose outside the
registry. This does not qualify source teardown or all other observer/registry
allocation and outer IRQ contexts; SEC-18 remains partial. See the
[source-preparation audit](../reports/audits/2026-10-15-waiter-source-preparation.md).

Completion callback list/entry preparation also leaves registry and inner-state
guards. Exact captured operation/account revalidation precedes allocation-free
publication; first-winner lists retain their original classification, and
losing/ready/rejected storage disposes after unlock. Successful completion
namespace/CQ replacement additionally carries whole detached source metadata
past the registry before destruction. These are scoped local boundaries;
other observer disposal, staging/error and outer contexts remain open. See the
[callback/source follow-up](../reports/audits/2026-10-15-completion-callback-preparation.md).

CQ source-list preparation now runs with its admitted backlog/ring payload
outside the completion registry. Exact namespace root/account/capacity and
retirement revalidate at publication; unused or rejected owners dispose after
guard exit. Physical-frame initialization follows serialized alias checks.
Registry-map/authority metadata, timer disposal and outer caller/IRQ contexts
remain separate; see the [CQ payload-preparation follow-up](../reports/audits/2026-10-16-cq-payload-preparation.md).

## Verification and limits

Serialized guest fixtures retain an empty list with a registration token and
128 weak aliases, then verify exact counters through final release. Injected
list allocation rejection returns its unused reservation. Sponsor promotion
preserves the original ordinary allocation; retirement rejects a new lazy source.

Counter-only saturation rejects ordinary list creation, lazy parking, completion
submission, CQ attachment, callback installation, endpoint creation and scalar
call preparation without consuming operation/entry quotas or queueing work.
A real platform waiter/list initializes from the reserve under ordinary pressure.
Total saturation rejects platform creation too. Releasing reservations permits
retry and restores the exact baseline after normal subsystem/root cleanup.

These fixtures do not allocate the maximum list footprint, force physical OOM,
prove exhaustive cross-LP races or establish whole-node containment. Existing
watch, waiter, timer, IPC, generation-reuse and scoped EL0 tests remain enabled.
Per-domain/principal list attribution, arbitrary callback captures, registry
nodes, sponsor backing and other weak-only allocations remain separate work.
SEC-07 remains partial. Evidence:
[observer-list audit](../reports/audits/2026-10-07-security-observer-lists.md).
