# Endpoint and queue admission

Kernel admission capacity for this family comes from the default compile-time
[resource policy](resource-policy.md). Related values can be adjusted together
in a reviewed build profile; see [Configuration](../guides/configuration.md).
Structural/protocol bounds and original-charge/physical-completion rules remain
separate contracts.

Endpoint creation reserves two independently checked resources: one endpoint
record and its preallocated queue backing slots. Charges remain with the
creating IPC namespace, including after delegation or domain retirement.

| Limit | Current kernel policy |
| --- | --- |
| Endpoint records per domain generation | 64 |
| Queue backing slots per domain generation | 8,192 |
| Endpoint records per node | 1,024 |
| Queue backing slots per node | 32,768 |
| Ordinary-domain share of node capacity | 768 records and 24,576 slots |
| Single endpoint's admission capacity | 1–4,096 messages |

These are initial kernel policies, not signed descriptor parameters or
application-facing settings. They bound endpoint storage, independently of
the TCP/IP service's socket capacity. A connection is not a new endpoint.
Kernel/supervisor-designated platform domains may use the remaining node
share; classification is captured against the namespace's exact live ASID and
generation. Names, application roles and descriptor contents cannot confer
this designation. The reserve is a shared pool, not a guaranteed allowance
for each essential service.

## Queue backing and resize

Creation captures the IPC namespace's exact root, leaves IPC before acquiring
its operation lease, then revalidates that generation before reserving one
record and all queue slots atomically. Slot reservations round capacity up to a
power of two, with a minimum of four slots. The complete preparation owns the
original account/classification, both charges, endpoint node, queue, close and
readiness lists, and any shared authority reservation.

Node, backing and observer-list allocation run fallibly outside IPC and local
lifecycle/table guards. Revalidation precedes authority publication and endpoint
node relinking. Ordinary rejection explicitly finishes the complete prefix after
local guards; its exact-root lease finishes last. Abandonment retains every
field without cleanup. Accepted sends do not grow backing beyond its admitted
size. Message attachments and their allocations are outside the slot count.

Growth beyond retained backing captures the exact endpoint, original account
and original classification under IPC. Its existing root-owning operation then
reserves and allocates a second queue outside IPC. Both backing allocations count
during that transition, so growth can reject even when the final size would fit.
Before moving messages in FIFO order, publication rechecks the captured root,
capability, endpoint identity, closing state and current backing capacity. A
competing larger resize can supply sufficient backing; the unused preparation
then disposes after unlock. Concurrent close rejects without restoring authority
or replaying work. Ordinary preparation failure leaves the old queue unchanged.

The operation retains replaced backing through IPC unlock, explicitly frees it
before returning its original slot charge, and finishes its root last. Later
caller promotion cannot reclassify existing endpoint storage or queue growth.

Shrinking changes admission policy only. Existing messages survive and new
sends fail while depth reaches or exceeds the new limit. The retained backing
and its charge do not shrink: repeated grow/shrink cycles cannot conceal
allocated memory. Growth within retained backing needs no new reservation.

## Closure, delegation and generation reuse

Closing the owning endpoint drains/cancels queued operations, releases their
attachments and frees queue backing. A closed endpoint record can remain
referenced by delegated connections; its record charge follows the complete
node through actual disposal.
Final-reference removal detaches a closed, empty, unclaimed endpoint into the
existing cancellation, endpoint-close or serialized cleanup owner. That owner
leaves IPC before freeing payload/node and refunding the record charge. Queued
and unobserved returned connections follow the same rule as explicit close.
The detached nodes form a temporary owning list without a new registry;
abandonment retains the entire list. Detachment grants no physical cleanup or
replay authority.

Every namespace has a reference-counted budget owner. Retained closed records
keep the old owner alive after domain teardown. Reusing the numeric ASID gives
the replacement a fresh budget; a late old-generation connection close cannot
credit that replacement. Endpoint creation and resizing reject a retiring
generation before publication or growth.

Budget locking is domain-before-node, with checked all-dimension reservation
and rollback. Budget critical sections do not allocate or call another
subsystem. Explicit ordinary finish frees admitted storage before releasing
its charge.
Containing preparation/retirement fallback retains storage and charge together;
it does not enter budget locks, the allocator or notification callbacks.

## Errors and verification scope

The kernel returns `IpcError::ResourceLimit` for exhausted endpoint/queue
admission or failed queue allocation. Status code 10 is reserved as
`ipc_status::RESOURCE_LIMIT`. The existing creation and resize syscalls still
return zero on failure rather than a detailed status; runtime clients see
their existing creation/resize error. This change does not introduce a
userspace counter interface or a distinguishable quota error for those calls.

Synchronous boot tests cover endpoint and queue ceilings, failed-growth
rollback, retained backing after shrink, delegation and unobserved-return
cleanup, retired-generation rejection, forced ASID reuse, and exact counter
reconciliation. Tests saturate each ordinary and total node dimension using
reservations without allocating the corresponding maximum heap footprint.
The scoped EL0 probe fills its endpoint budget, performs scalar IPC while it
is full, drops the owning batch and completes 128 create/drop cycles.
Host tests check multidimensional overflow/underflow and atomic rejection.

These controls do not bound the complete capability namespace, general observer
storage, attachment vectors, all completion metadata, loader/page tables or the
entire kernel heap. Separate [IPC record budgets](ipc-record-budgets.md) cover
connections, retained pending calls and outstanding reply tokens. Separate
[record budgets](completion-record-budgets.md) cover retained completion objects
and detached results. Endpoint nodes publish by relinking prepared storage;
[capability payloads](ipc-capability-storage.md) now do the same. Endpoint
shared-authority storage prepares unlocked, admission revalidates/relinks under
IPC, and private payload allocation leaves IPC again before final publication.
Other capability callers retain their guarded factory boundaries;
[borrowed publication](ipc-publication-batches.md) removes the temporary batch
vector. Mixed memory payload maps still allocate under their registry hold. No global allocator-exhaustion injection,
sustained hostile-pressure soak or per-principal aggregate across multiple
domains is claimed. SEC-07 remains partially implemented; deployment still
requires the [security audit's restrictions](../reports/audits/2026-10-03-security-remediation.md).

Endpoint-readiness and pending-call **waiter entries** now have separate
[shared scheduler admission](scheduler-waiter-budgets.md) and owning cancellation.
Their source-list allocation is fallible before attachment transfer; that does
not by itself admit records or all metadata. Call submission now reserves its
separate pending-call/reply-token records before preparing that list.

The [endpoint storage audit](../reports/audits/2026-10-24-security-ipc-endpoint-storage.md)
records 384 preparation-stage and 96 growth-stage rejections, allocation-free
relinking/FIFO replacement, competing growth/close, hidden-reference retirement
and guarded abandonment. These are scoped rejection controls. Endpoint lookup,
insertion and repeated close/reference scans are linear; count limits do not
qualify worst-case IRQ hold time, contention or sustained pressure fairness.

The [unlocked preparation follow-up](../reports/audits/2026-10-24-security-ipc-unlocked-preparation.md)
extends the endpoint controls to 640 stage rejections and 64 complete endpoint
publications with heap held. Its existing root remains live through every
factory and final publication. Earlier 384-stage results retain their own dated
checkpoint; quota/classification policies and physical cleanup proofs are unchanged.
