# Endpoint and queue admission

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

Creation reserves backing before a fallible `VecDeque::try_reserve_exact`.
Slot reservations round capacity up to a power of two, with a minimum of four
slots. Failed admission or allocation drops the staged owners and returns all
reservations; no endpoint or owning capability is published. Accepted sends do
not grow this backing beyond its admitted size. Message attachments and their
separate allocations are not included in the slot count.

Growth beyond allocated backing stages a second charged queue, then moves the
messages in FIFO order and replaces the old queue. Both backing allocations
count during that transition. This can reject growth even when the final size
would fit: the temporary peak must fit too. Failure leaves the old capacity,
depth and messages unchanged.

Shrinking changes admission policy only. Existing messages survive and new
sends fail while depth reaches or exceeds the new limit. The retained backing
and its charge do not shrink: repeated grow/shrink cycles cannot conceal
allocated memory. Growth within retained backing needs no new reservation.

## Closure, delegation and generation reuse

Closing the owning endpoint drains/cancels queued operations, releases their
attachments and frees queue backing. A closed endpoint record can remain
referenced by delegated connections; its record charge remains until the last
reference is removed. Internal cancellation of an unobserved returned
connection follows the same reclamation rule as explicit connection close.

Every namespace has a reference-counted budget owner. Retained closed records
keep the old owner alive after domain teardown. Reusing the numeric ASID gives
the replacement a fresh budget; a late old-generation connection close cannot
credit that replacement. Endpoint creation and resizing reject a retiring
generation before publication or growth.

Budget locking is domain-before-node, with checked all-dimension reservation
and rollback. Budget critical sections do not allocate or call another
subsystem. Storage-owning structs release charges through `Drop`, after freeing
their admitted backing.

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
and detached results. Allocation of registry
and capability metadata is still infallible. No allocator-failure injection,
sustained hostile-pressure soak or per-principal aggregate across multiple
domains is claimed. SEC-07 remains partially implemented; deployment still
requires the [security audit's restrictions](../reports/audits/2026-10-03-security-remediation.md).

Endpoint-readiness and pending-call **waiter entries** now have separate
[shared scheduler admission](scheduler-waiter-budgets.md) and owning cancellation.
Their source-list allocation is fallible before attachment transfer; that does
not by itself admit records or all metadata. Call submission now reserves its
separate pending-call/reply-token records before preparing that list.
