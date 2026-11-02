# IPC payload-node backing

IPC capability, endpoint, pending-call and reply-token payload nodes use the
shared admitted collection with `ipc_nodes` requested-byte/count admission.
This covers unused candidates, published nodes and detached cleanup storage.
It is separate from logical endpoint/connection/call/reply counts, unified
authority backing, namespace accounts, queues and attachment buffers.

`ipc::node_budget::Charge<R>` keeps backing admission and an optional later-bound
logical resource reservation in the same allocation. The common collection's
`BindCharge` contract binds the latter without replacing backing admission.
Capability, pending and reply nodes use record charges; endpoint nodes use their
endpoint charge. Non-connection capability payloads still own backing admission
without an additional logical record charge. Requested layouts include payload,
linkage and both charge fields. No holder allocation or cleanup registry is added.
Actual node deallocation precedes both refunds through existing post-IPC disposal.

## Captured classification

| Node | Backing classification | Independent logical sponsorship |
| --- | --- | --- |
| Capability payload | `PreparedReservation` captures the exact recipient authority namespace's immutable storage class before preparation; IPC borrows it without another lookup | Connections retain their original grantor/requester record charge; other kinds use shared authority admission |
| Endpoint payload | Original endpoint reservation's class, captured against the accepted exact root before allocation | Original endpoint owner and its account |
| Pending call / reply token | Original call-record reservation's class, captured against the accepted exact caller root | Requesting caller and its original record account |

Trusted backing grants no logical resource privilege. An ordinary grantor may
create a trusted kernel-recipient payload while retaining an ordinary connection
charge. Promoting a root cannot reclassify a namespace's captured storage class,
an endpoint reservation or an already prepared node. Retirement and ASID reuse
cannot redirect their eventual refunds.

## Preparation and retirement

Every factory admits its actual layout before allocation. `PreparedCall` retains
its exact root, both logical reservations and the pending/reply prefix across
unlocked preparation. Capability authority/count preparation precedes its private
payload factory; the same captured storage class accompanies both. Endpoint
preparation retains its original endpoint/queue reservations before allocating
its node. Quota or allocator denial returns `IpcError::ResourceLimit`. Ordinary
rejection explicitly cancels the complete prefix after local guards, root last.
A failed capability factory leaves authority with its containing preparation
until finish; no attachment or payload is published early.

Publication and detachment only relink complete nodes. Scalar reply, cancellation,
endpoint/namespace close and hidden/unobserved connection cleanup retain these
same nodes in existing owning operations and retirement lists. Failed loan cleanup
retains its fenced record. Closed endpoints referenced by queued or returned
connections keep backing and endpoint admission until their actual disposal.
No metadata ticket reconstructs ownership or supplies physical completion/replay
permission. Complete call, capability, endpoint/growth and detached-list fallback
remains inert under IPC, heap and metadata-pool guards. Wider active containing
destructors, caller IRQ masks and copy latency require separate qualification.

## Capacity and administration

The expanded pool uses ordinary/total limits of **12/16 MiB** and
**73728/98304 nodes**. The earlier pending/reply-only defaults were 6/8 MiB and
18432/24576 nodes. Logical resource ceilings remain unchanged. The increase
accommodates capability and endpoint payloads alongside calls, avoiding an early
shared backing ceiling merely because the covered family grew.

A conservative count plan permits every ordinary/total unified capability slot
(49152/65536) to hold an IPC payload, plus all pending and reply nodes
(6144/8192 each) and endpoints (768/1024): **62208/82944 nodes**. This leaves
candidate/layout headroom while preserving a quarter of each pool for trusted
admission. Native capability/endpoint/pending/reply layouts are 120/272/200/176
bytes. This upper-bound mix requests 8417280/11223040 bytes (about 8.03/10.70 MiB),
within the 12/16 MiB budgets before other nested allocations and allocator
overhead. Added backing fields cost 16 bytes per capability/endpoint node
(780/1040 KiB at those capability/endpoint ceilings), without a holder allocation.
Independent ceilings do not promise simultaneous RAM fit or progress.

Sharing the existing pool avoids another reserve and administration category,
but these layouts now compete for same-class bytes and slots. Raising limits
permits larger retained heap consumption; lowering them can restrict otherwise
valid logical resources. Review actual mixed workloads and adjust central byte
and count limits together. General heap/principal admission remains open.

The 23-pool catalog, statistics version 10, header/record widths, SDK and signed
runtime ABI remain unchanged. Services consume current limits from snapshots;
no service source or bundle rebuild is needed for this kernel-only extension.

## Qualification and remaining scope

The [payload evidence](../reports/audits/evidence/2026-11-02-ipc-payload-node-admission.json)
identifies source/artifacts and diagnostics. Earlier [pending/reply evidence](../reports/audits/evidence/2026-11-02-ipc-node-admission.json)
retains its original limits and version-10 HTTP qualification separately.
The full host suite includes actual deallocation tracing for backing plus a
later-bound logical resource charge. Native controls inject 512 allocator
rejections across all four layouts, exhaust real empty-node admission, and prove
heap-held pre-allocator quota rejection with exact recovery. Independent byte
saturation also uses an explicit counter adapter; it is not general heap OOM.

Public capability controls cover 64 quota denials after authority reservation,
namespace classification through promotion, and an ordinary grantor using trusted
recipient backing without trusted logical admission. Endpoint controls cover
64 quota rejections after endpoint/queue admission and promotion between reserve
and node allocation. Existing heap-held publication/detachment, root reuse,
hidden connection cleanup, scalar/vector receive, cancellation, returned authority
and guarded complete-owner abandonment exercise the same admitted nodes.

The full host suite and strict test/minimal kernel checks passed on both
architectures. Fresh four-CPU security campaigns completed ARM 19/19, Intel VT-d
17/17 and AMD-Vi 17/17, with zero failures or pending tests. Current signed
services were reused because SDK and wire formats are unchanged.

Nested attachment/control storage, other remaining allocations, general heap
and aggregate principal admission remain open. Serialized controls do not prove
sustained hostile concurrent EL0 fairness or all caller/IRQ/destructor contexts.
SEC-07/18 remain partial.
