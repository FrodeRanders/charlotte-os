# IOMMU backend registry-node backing

VT-d, AMD-Vi and SMMUv3 domain cells, requester/stream fences and DMA mapping
records share admitted metadata nodes. VT-d context-table metadata and SMMU
walker-cache nodes use the same pool. Typed
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
allocator bookkeeping and aggregate principal/general heap accounting remain
separate. Admission adds a short pool operation at
allocation/release and the existing registry `MapFailed` or mapping/cache
`Memory` rejection.

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

Native node layouts are 48 bytes for requester/context/walker nodes, 104 for a
DMA mapping, 136 for a VT-d domain, 128 for an AMD-Vi domain and 168 for a SMMUv3
domain. Mapping/cache charges add 16 bytes per node; retained classification adds
8 bytes to each domain record owner and another 8 to SMMU cache ownership. Requester-only
count ceilings request 9/12 MiB ordinary/total; domain-layout controls can reach
the independent 18/24 MiB byte ceilings with count headroom.

## DMA mapping records and SMMU walker cache

The same `iommu_nodes` budget covers `mapping_storage::Records`, its quarantine
list and `WalkerCache`. These allocations have the domain's lifetime and trusted
classification. Each backend captures `DmaCreation.backing_platform` into its
records/cache during domain construction, before hardware publication. Subsequent
map or cache preparation uses that immutable value. Root promotion, ASID reuse,
map callers and later requester lookups cannot reclassify it; a fresh grant may
capture a newly trusted class.

A prepared mapping node owns its original requested-layout byte/count charge
alongside the DMA pin. Admission precedes the allocator and data/table leaves;
quota denial returns the existing `Memory` error without allocator entry.
Ordinary unused preparation explicitly unpins and frees its node after backend
restoration. Publication, detachment and quarantine relink that same admitted
node. Failed prefix or unmap completion retains pin, original charge and node
until confirmed domain-table release. Unpin completes before node deallocation,
and node deallocation precedes refund. A pin's frame snapshot has separate
snapshot-buffer admission; these byte counts must not be added as physical RAM.

SMMU cache admission precedes branch allocation/linking. Partial walks retain one
unused admitted node for later reuse; cached nodes stay charged through ordinary
unmap. Both cached and unused nodes release only after confirmed private
cancellation or domain-table completion. Containing `DetachedDomain`,
`MappingMaintenance` and prepared-grant fallback remain inert under the heap,
backend and metadata-pool guards. Metadata admission supplies no hardware proof
or physical retry authority.

This extends one related metadata pool instead of adding another reserve or
statistics grammar. Numerical limits stay 18/24 MiB and 196608/262144 nodes;
all 22 existing inventory rows and the version-9 payload remain current. Empty
real-layout pressure measures the independent byte and count ceilings. The
104-byte mapping layout can occupy about 181000 ordinary and 242000 total nodes
before byte denial when other usage is small, exceeding the 65536 unified
capability planning ceiling. This is a single-layout measurement, not a promise
that all layouts or independent resource ceilings can be filled together. A
limit increase is not needed for these controls; sustained mixed workloads may
justify one after their retained-byte requirements are measured. Sharing
bounds retained mapping/cache storage but makes it compete with other IOMMU
metadata in the same class. A quarter remains trusted reserve. These limits do
not preallocate RAM; a deployment needing more combined headroom may raise the
reviewed central limits without changing kernel ownership or completion logic.
General principal/heap accounting and fairness remain separate gates.

## Qualification and remaining scope

The [original registry evidence](../reports/audits/evidence/2026-11-02-iommu-node-admission.json)
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
No new hardware timeout or outstanding device I/O is injected. Table-ledger
backing, aggregate principal accounting and wider caller/IRQ/latency qualification
remain open. The shared read-only inventory reports this pool; it grants no
cleanup authority.
SEC-07 remains partial; C14/C15 and G1/G4 cleanup boundaries continue to apply.

The [mapping/cache admission evidence](../reports/audits/evidence/2026-11-02-dma-metadata-admission.json)
records the extended layouts and native outcomes. Fresh four-CPU, 240-second
security campaigns passed ARM 19/19, Intel VT-d 17/17 and AMD-Vi 17/17, with
zero failures or pending tests. Strict kernel checks passed for both test and
minimal configurations; 64 shared-admission and 16 shared-retirement host checks
passed. Controls inject allocator
rejection for the actual mapping/cache layouts, deny every layout with count
headroom exhausted, and fill the byte ceiling with actual empty mapping nodes.
Storage fixtures retain exact charges across publication, detached quarantine,
prefix failure and unused cache reuse, and check trusted admission while
ordinary capacity is exhausted. The existing complete mapping/grant abandonment
fixtures also hold the metadata pool guard, retaining the same original owners.
A real NVMe control promotes an ordinary domain's root, rejects 32 maps without
table mutation or lost pins, then uses a fresh trusted grant to map/unmap under
ordinary metadata pressure and confirms complete refund at domain teardown.
