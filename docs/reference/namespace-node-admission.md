# Global namespace-node backing

The global capability, IPC, completion, mailbox-capability and mailbox-word maps
use independently admitted nodes. This covers their prepared empty storage as
well as published, detached and retained nodes. Account bundles and nested
payload allocations have their own admission; preparing a global node no longer
depends on reaching a later account constructor to obtain a charge.

| `namespace_nodes` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested node-layout bytes | 6 MiB | 8 MiB |
| Prepared/published/retained nodes | 30720 | 40960 |

Each type reserves its actual `PreparedEntry<T, Charge>` layout, including the
charge field, before allocation. Both dimensions must fit. Quota or allocator
rejection returns without publishing authority; unused allocation failure
refunds the original reservation. One leaf pool serves all five layouts, while
each retains its existing typed prepared/retired owner and registry.

The count ceiling allows twice the five-node needs at the root-metadata pool's
count-only capacity of 3072 ordinary or 4096 total complete roots. Root creation
needs capability and IPC nodes before ASID publication; the other three are
prepared when their interfaces are initialized. Headroom admits overlapping
preparation, namespace replacement and retained generations. The byte ceiling
independently caps requested storage as layouts grow. A quarter remains shared trusted reserve,
without per-type or per-service progress entitlements. No existing resource limit
is raised and no RAM is preallocated.

Native layouts are 152/144 bytes for capability/IPC nodes, 240 bytes for a
completion node, 112 bytes for a mailbox-capability node and 152 bytes for a word
namespace node: 800 bytes for all five. These layouts include the captured
[namespace-account](namespace-account-admission.md) and
[reactor-account funding references](completion-account-admission.md). The capability
namespace also retains its immutable principal/class token for authority-record
preparation. Twice the complete-root count capacity
uses 6,553,600 total or 4,915,200 ordinary layout bytes, below the 8/6 MiB byte
ceilings. A mix consisting only of 240-byte completion nodes is byte-bound at
34952 total or 26214 ordinary nodes before those shared count ceilings. The
count-pressure controls use capability namespace nodes; they do not demonstrate
count-first capacity for every layout. Independent byte-denial controls use
oversized synthetic layouts. Increasing the byte ceiling would admit more of the
largest type but consume more heap; the reviewed complete-root mix needs no such
increase.

The 32-byte typed charge carries requested bytes, captured class and the original
principal policy. `NodeCharge<NamespaceNodes, MetadataSponsor>` funds actual node
layouts against the shared 8 MiB metadata allowance before family admission and
allocation. Capability/IPC roots capture the unpublished prebound translation
policy; completion and mailbox capture the exact current root before preparation.
Family denial rolls back unused principal funding. Actual free precedes both
refunds, including losing candidates prepared beside an existing namespace.
Reusing the existing admitted map and retirement collection preserves
allocation-free relinking and their post-deallocation refund order. This adds
admission work outside publication guards and at actual node release; it does
not bound allocator excess, nested payload bytes or general kernel heap use.

## Capture, publication and release

Capability and IPC preparation capture original principal and trusted class from the
owned unpublished translation root. Completion and lazy mailbox/word preparation
capture that policy with the exact current generation before allocation, using their
existing captured identities and leases. Permanent kernel namespaces are trusted;
raw fixture namespaces remain ordinary. Names, roles and manifest fields do not
select reserve. Metadata admission grants no resource reserve privilege.

Publication relinks the same prepared node after existing identity/retirement
validation; it allocates, reserves and refunds nothing under local publication
guards. Partial root or namespace rejection disposes unused nodes after those
guards. A losing mailbox candidate retains its original charge until explicit
post-guard disposal, even when an existing namespace wins.

Node admission follows actual backing, independently of its payload accounts.
Logical retirement, account release, namespace removal, promotion and ASID reuse
cannot refund or reclassify a surviving node. The shared retirement collection
keeps its charge outside payload destruction and returns it only after the node
allocation is freed. Completion namespace finish may transfer its payload for
later disposal; freeing that global node ends only its own charge. Its surviving
accounts, queues and records keep theirs.

Capability and mailbox nodes joining the final root receipt remain charged
through failed invalidation and terminal abandonment. Only the existing typed
completion permits release. Root, namespace, mailbox and detached-node fallback
remain inert beneath unknown guards; explicit cancellation/disposal retains its
existing caller-context requirements. This adds no cleanup family, retry owner,
root lease or physical completion proof.

[Principal qualification](../reports/audits/2026-11-06-q4-container-principals.md)
reconciles all five namespace layouts separately from authority/endpoint nodes.
Mailbox preparation retains both its existing namespace and losing candidate;
six quota prefixes preserve the existing receiver, counters and identity. The
existing complete-root recovery/abandonment fixtures now carry funded principals
through failed invalidation and confirmed or terminal release. This preserves
typed physical prerequisites rather than adding a new cleanup owner.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-01-namespace-node-admission.json)
records actual capability-layout storage to ordinary and total node ceilings,
byte denial with count headroom, heap-held admission rejection, partial root
rollback, completion/mailbox rejection and trusted preparation of all five nodes
at ordinary exhaustion. Closing a promoted root at total exhaustion returns its
five original ordinary nodes and bytes, allowing exact fresh creation/recovery.
It also records each actual node layout and complete-root capacity planning.

Five-layout controls inject 64 allocator rejections each with exact admission
recovery. Existing IPC and completion publication/removal probes hold the actual
heap and node-pool guards. Existing capability/root/mailbox/namespace parent
abandonment controls retain their nodes with that pool held; no additional
deliberate-retention fixture is introduced. Failed final-root invalidation
retains its captured capability node, and confirmation returns its original
bytes/count independently of intervening root creation. Shared collection host
tests check deallocation before refund and retaining fallback.

Pressure allocates actual empty capability-layout nodes, not the largest mixed
footprint or maximum nested payload set. Native heap exhaustion is not forced;
allocator errors use the named injection controls. Individual payload/registry
nodes outside these five global maps and aggregate principal/general-heap
admission remain open. Wakers/callbacks have admitted
[observer handles](observer-ownership.md). There is no sustained hostile-pressure
fairness, worst-case lookup/IRQ-latency, arbitrary destructor or physical hardware
claim. SEC-07 remains partial; existing C02/C03/C10/C18 and G1/G4 boundaries in
[cleanup/recovery](cleanup-recovery.md) continue to apply.
