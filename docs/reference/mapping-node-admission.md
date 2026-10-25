# Memory-mapping node admission

[`object/mapping.rs`](../../crates/catten/src/memory/object/mapping.rs) groups
map admission, installation, rollback and invalidation with its
[`storage` child](../../crates/catten/src/memory/object/mapping/storage.rs).
Installation helpers are private to that boundary except the existing two
object-family retirement-fixture hooks. Shared mapping records and retirement
pins still serve their original namespace/loan cleanup owners; storage release
keeps its object-family scope. Root `map`, `map_any` and `unmap` remain facades.
No admission, guard or physical-completion policy changes. See the
[refactoring evidence](../reports/audits/2026-10-25-refactoring-memory-mapping.md).

CPU memory-object mapping records share fixed requested-layout admission:

| Dimension | Total | Ordinary callers |
| --- | --- | --- |
| Requested node-layout bytes | 1 MiB | 768 KiB |
| Prepared, published or retained nodes | 8192 | 6144 |

These values are the default of the central compile-time
[resource policy](resource-policy.md). `CATTEN_RESOURCE_POLICY` can select a
reviewed build profile; see [Configuration](../guides/configuration.md). Allocation
and cleanup checks keep the same owners and captured charges. Boot-time policy
installation and live resizing are not implemented.

Each node reserves its complete layout, including payload, link and typed charge,
before fallible allocation. Quota or allocator rejection returns `ResourceLimit`.
The independent leaf counter leaves before allocator entry. Mapping preparation
captures trusted classification once against the exact calling namespace handle,
before either counter or factory. Later promotion and ASID reuse cannot change
that node's classification or original refund. Caller names/roles/manifests supply
no reserve claim. Snapshot buffers keep their separate [admission](snapshot-buffer-admission.md).

The existing prepared node carries its charge into the existing mapping map,
`MappingRetirementPin`, selected retirement or whole-map `RetiredObjectMappings`.
Publication and detachment only relink storage; neither allocates, refunds nor
enters the new counter. Ordinary unused preparation explicitly finishes after
local memory/table guards. Its inert fallback retains even partially constructed
storage and charges without allocator, registry or counter entry.

Published cleanup still requires its original leaf identity, invalidation,
scratch and backing-pin proofs. Confirmed cleanup disposes payload and actually
frees the node before refunding its original charge; a captured peer lease
completes afterward. Failed or abandoned cleanup retains the complete node,
original charge and containing owner. No replacement node is needed for unmap,
and refund supplies no hardware proof, recovery custody or replay permission.

Both ceilings apply to overlapping unused candidates and retained nodes as well
as live mappings. The first applicable dimension rejects; this single layout can
reach its count ceiling before its byte ceiling. Requested layout excludes
allocator overhead/free heap backing and other metadata. No aggregate principal
limit, new root/pin registry or guaranteed reserve share for one service is added.

The [audit evidence](../reports/audits/2026-10-25-security-mapping-node-admission.md)
qualifies actual-node exhaustion, public map/map-any denial without snapshot/pin
publication, trusted reserve, real unmap at total exhaustion, late promotion,
root reuse and existing guarded uncertainty. This advances R7-2/R7-3 and C05/C18;
other metadata families, principal/general-heap budgets, broader cleanup contexts
and sustained concurrent essential-service progress remain open. SEC-07/18 stay
partial.
