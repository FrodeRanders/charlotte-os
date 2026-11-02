# IOMMU backend registry-node backing

VT-d, AMD-Vi and SMMUv3 domain cells and requester/stream fences use independently
admitted registry nodes. VT-d context-table metadata uses the same pool. Typed
[`backend_registry::budget`](../../crates/catten/src/device/backend_registry/budget.rs)
storage follows existing preparation, unit registry and detached-node ownership;
hardware completion remains with the existing typed backend owners.

| `iommu_nodes` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested node-layout bytes | 18 MiB | 24 MiB |
| Prepared/published/retained nodes | 196608 | 262144 |

The total count is four times the existing 65536 unified-capability planning
ceiling, leaving headroom for domain cells, persistent requester fences, context
metadata and overlapping candidates. A quarter remains shared trusted reserve.
These are planning defaults, not a promise that all independent resource ceilings
can be occupied together. Existing resource limits and hardware ID limits are
unchanged. No RAM is preallocated; retained fences can consume admission after
their original domains disappear.

Each inline charge adds 16 bytes on supported 64-bit targets: 4 MiB at the total
count ceiling. The byte dimension admits the actual requested retirement-node
layout, including charge and inline payload storage. Nested table ledgers,
mapping/walker metadata, allocator bookkeeping and aggregate principal/general
heap accounting remain separate. Admission adds a short pool operation at
allocation/release and an existing `dma::Error::MapFailed` rejection.

## Captured class and lifetime

The containing `GrantAdmission` retains the storage class captured with its exact
root generation. `PreparedDmaDomain` transfers that immutable value into its
`DmaCreation` before backend entry, separately from resource and waiter grants.
Each `Nodes` preparation copies that same captured class for all its candidates.
No allocator, backend lock or release path resolves a reusable ASID or trusts a
manifest, role or requester ID to choose reserve. Raw kernel fixtures explicitly
select ordinary storage.

Nodes reserve actual bytes and one count before their own heap allocation. All
required domain/source/context candidates precede reset, next-ID mutation,
physical domain backing and hardware descriptor publication. A partially rejected
preparation stays in the complete grant until its existing post-guard explicit
cancellation; no new scalar cleanup ladder is introduced.

Publication relinks the same nodes. A domain cell remains admitted while its
payload is extracted for maintenance or physical finalization. Successful table
release and data unpins precede cell detachment and post-backend node disposal.
The shared retirement collection frees the node before refunding its original
bytes/count. Hardware timeout, partial physical release and abandoned owners
retain the existing claim, complete payload and charge; metadata admission grants
no cleanup or replay authority.

Requester fences and context metadata have the unit's longer lifetime. Domain
destruction sets the original requester value to zero without freeing or
reclassifying its node. A later trusted domain can reuse an ordinary fence in
place; the old charge remains ordinary. Published VT-d context metadata remains
charged with the hardware-visible context table. Current units have no teardown
path that refunds these nodes. This retention is distinct from a domain-cell
refund and does not establish device reset completion.

The shared `AdmissionPool` and retirement collection enforce common counter and
deallocation mechanics. Each backend retains its original typed engine, domain,
claim and completion policy. The pool itself changes only counters and holds no
guard across allocation, callbacks or another subsystem.

Native node layouts are 48 bytes for requester/context nodes, 128 for a VT-d
domain, 120 for an AMD-Vi domain and 152 for a SMMUv3 domain. Requester-only
count ceilings request 9/12 MiB ordinary/total; domain-layout controls can reach
the independent 18/24 MiB byte ceilings with count headroom.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-02-iommu-node-admission.json)
identifies source and native artifacts. Controls allocate real empty requester
nodes to ordinary and total count ceilings, then exercise every applicable actual
layout under the heap guard. Separately, empty nodes with the active backend's
actual domain layout reach ordinary and total byte limits with count headroom;
128 heap-held byte denials and complete recovery are required.

All applicable layouts inject 64 node-allocator rejections each. Complete grant
preparation uses the actual captured root class: ordinary backing rejects at its
ceiling, trusted backing progresses, and both reject at total exhaustion. One,
two and three returned slots exercise partial construction and exact rollback.
These empty candidates own no physical domain or hardware obligation.

Existing heap-held publication/claimed-cell/detachment controls also hold the new
pool guard where applicable. Trusted reuse preserves the original ordinary
fence/context charge and exact payload address. The existing complete-grant
abandonment probe holds both device-node and backend-node pool guards alongside
its prior holds, retaining the same original owner without an additional leak.

The real NVMe fixture independently saturates ordinary count admission and runs
32 actual grant rejections before reset, ID or table publication. Requester,
registry, table charges, authority usage and disabled PCI configuration stay
unchanged. This adapter control uses counter saturation; real backing pressure
is qualified separately above. Existing staged node rejection, reset/reassignment,
private rollback, map/unmap and confirmed retirement tests remain required.

These are serialized kernel fixtures, not sustained hostile concurrent EL0
fairness or general heap OOM qualification. Node allocation errors are injected;
byte/count pressure allocates actual backing for empty production-layout nodes.
No new hardware timeout or outstanding device I/O is injected. Per-domain
mapping/walker and table-ledger backing, aggregate principal accounting, common
live usage reporting and wider caller/IRQ/latency qualification remain open.
SEC-07 remains partial; C14/C15 and G1/G4 cleanup boundaries continue to apply.
