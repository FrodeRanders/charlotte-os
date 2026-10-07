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
generation's captured sponsor. Retirement checked by that sponsor rejects fresh
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

## Rejection and locks

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
callbacks. List initialization and registry operations still allocate under some
existing source/subsystem guards; this does not close SEC-18.

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
