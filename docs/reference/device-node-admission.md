# Device registry-node backing

Device namespaces and individual MMIO/IRQ/DMA capability payloads use typed
[`node_budget`](../../crates/catten/src/device/node_budget.rs) storage with
independent backing admission. This covers unused candidates as well as published,
detached and retained nodes. Unified authority records keep their separate
[node admission](authority-node-admission.md) and resource counts.

| `device_nodes` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested node-layout bytes | 12 MiB | 16 MiB |
| Prepared/published/retained nodes | 104448 | 139264 |

Count defaults are twice the sum of existing unified-capability ceilings and
the root-metadata pool's complete-root planning envelope:
`2 * (49152 + 3072)` ordinary and `2 * (65536 + 4096)` total. Device payloads
share unified authority limits with other capability kinds. The additional
capacity permits two-node grant preparation, unused namespace candidates and
retirement to overlap. Existing resource limits are unchanged, and no RAM is
preallocated. A quarter remains shared trusted reserve, without per-device or
per-service progress guarantees.

The inline charge adds 16 bytes per node on supported 64-bit targets, or
2176 KiB at the total count ceiling. Preparation/release also adds a short pool
operation and another possible `DeviceError::ResourceLimit` rejection. The byte
dimension uses the actual requested retirement-node layout, including charge and
inline payload storage. Allocator bookkeeping, nested allocations and aggregate
principal/general-heap consumption remain outside this pool.

Native node layouts are 48 bytes for a namespace and 72 for a capability on both
architectures. Even all-largest-layout populations at the count ceilings request
about 7.2/9.6 MiB ordinary/total, below the 12/16 MiB byte ceilings. This leaves
layout-growth headroom while keeping bytes independently bounded.

## Captured identity, publication and release

`GrantAdmission::new` captures root generation and trusted backing class together,
then acquires the exact root lease before allocating either node. A retirement or
reuse race rejects that lease; it cannot apply an old class to a successor root.
Permanent kernel work is trusted. Names, roles and manifest claims cannot choose
reserve, and node allocation/release never re-resolves an ASID.

Both prepared nodes retain this original class, independently of later promotion,
publication, root retirement or slot reuse. Trusted backing does not grant
platform capability-resource admission; the existing resource policy and
publication checks remain separate. The candidate namespace is discarded after
guard exit when an existing namespace wins. Its charge ends only after its own
node is freed, independently of the retained published namespace.

Each node reserves actual bytes and one count before its heap allocation. Quota
denial calls no allocator. If the second node cannot be admitted or allocated,
the unused first node is freed and refunded before the complete grant returns
its rejection. Both nodes precede unified authority preparation, grant resource
reservation and hardware creation/reset. Existing failure handling releases the
exact root lease last, outside local publication guards. No MMIO/IRQ/DMA authority
is published on rejection.

Publication only relinks prepared nodes under lifecycle/device serialization.
The existing typed close, DMA operation and namespace-retirement owners carry
the same charged nodes through detachment and confirmed cleanup. The shared
retirement collection deallocates a node before refunding its original backing.
Metadata detachment and hardware completion are distinct: failed invalidation,
scratch release or DMA completion retains the existing complete owner, node
charge, authority and root fence. No error authorizes refund or physical retry.

Complete grant, explicit-close and namespace fallbacks remain inert beneath
unknown guards. They retain every original field rather than freeing storage or
entering the new pool. Ordinary explicit disposal retains its existing qualified
caller/IRQ requirements. The leaf pool only changes counters and holds no guard
across heap allocation, callbacks or another subsystem.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-02-device-node-admission.json)
records sources and native artifacts. Actual mixed-layout empty candidates fill
ordinary and total ceilings. Controls include 256 heap-held individual-layout
denials, 128 complete-storage denials, 128 injected allocator rejections, and
independent ordinary/total byte rejection before allocation. All three real grant
adapters reject under the heap guard before authority/hardware work at exhausted
admission. One available node exercises partial rollback; two permit fresh grant
progress with exact recovery. A promoted root refunds its original ordinary node.

A trusted real root can use backing reserve without gaining a platform resource
grant. A separate empty-storage control retains both original candidate nodes
across root close and exact ASID reuse, then checks original-class refunds against
a trusted successor. Those candidates contain no device authority, mapping or
hardware obligation; this is backing-lifetime qualification.

Existing publication and explicit MMIO/IRQ claim controls also hold the new pool
guard. Mapped scratch/direct MMIO and IRQ close retain node charges through cleanup
and recover after exact root completion. Existing complete grant, backend-grant,
explicit-close and namespace abandonment controls add the pool guard to their
existing holds; no extra deliberate-retention owner is introduced. The shared
collection's host tests cover deallocation-before-refund and retaining fallback.

The campaigns are serialized kernel-fixture pressure, not sustained hostile
concurrent EL0 fairness. Oversized layout types test byte denial without allocating
those payloads; native general heap OOM is not forced, and allocation errors are
injected. IOMMU domain/requester/context, mapping and walker-cache nodes share separate
[backing admission](iommu-node-admission.md). Table-ledger capacity shares that pool's byte dimension with zero node count. Physical
reset/completion, arbitrary wider destructor/IRQ contexts, aggregate principal
accounting and latency remain separate qualification. SEC-07 remains partial;
existing C12/C13/C14 and G1/G4 cleanup boundaries remain applicable.
