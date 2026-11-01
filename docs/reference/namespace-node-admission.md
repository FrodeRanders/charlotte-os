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

Native layouts are 96 bytes each for capability and IPC nodes, 160 bytes for a
completion node, 80 bytes for a mailbox-capability node and 120 bytes for a word
namespace node: 552 bytes for all five. Twice the complete-root count capacity
uses 4,521,984 total or 3,391,488 ordinary layout bytes, below the 8/6 MiB byte
ceilings. Current layouts hit count ceilings first, even for the largest type;
byte-denial controls deliberately use oversized synthetic layouts.

The typed charge adds one requested-byte count and original-class flag to each
node. Reusing the existing admitted map and retirement collection preserves
allocation-free relinking and their post-deallocation refund order. This adds
admission work outside publication guards and at actual node release; it does
not bound allocator overhead, nested payload bytes or aggregate principal use.

## Capture, publication and release

Capability and IPC preparation capture trusted storage class from the owned
unpublished translation root. Completion and lazy mailbox/word preparation
capture it with the exact current generation before allocation, using their
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
nodes outside these five global maps, Waker/callback storage, aggregate principal
and general-heap admission remain open. There is no sustained hostile-pressure
fairness, worst-case lookup/IRQ-latency, arbitrary destructor or physical hardware
claim. SEC-07 remains partial; existing C02/C03/C10/C18 and G1/G4 boundaries in
[cleanup/recovery](cleanup-recovery.md) continue to apply.
