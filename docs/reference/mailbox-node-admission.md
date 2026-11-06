# Mailbox endpoint-node backing

Mailbox sender and receiver records use independently admitted map nodes.
`PreparingMailbox` admits its empty endpoint candidate before other allocations;
publication relinks the same storage after exact-generation and resource checks.
Prepared, published, detached and retained nodes consume the original requested
bytes and one slot until actual node deallocation.

| `mailbox_nodes` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested node-layout bytes | 1536 KiB | 2 MiB |
| Prepared/published/retained nodes | 12288 | 16384 |

The count ceiling is twice the existing mailbox resource ceiling (6144/8192).
This leaves headroom for unused candidates and retirement without lowering
existing mailbox or capability limits. No memory is preallocated. A quarter
remains trusted reserve; it is shared capacity, not a per-service entitlement.
The current native layout is 96 bytes, including its original funded account
reference, so count ceilings bind at 1152/1536 KiB.
Independent byte ceilings bound later layout growth.

The complete charge occupies 32 bytes per endpoint node and a short pool operation at
preparation/release. Rejection can now report `ResourceLimit` before allocation.
Candidate headroom cannot guarantee heap availability, completion or fairness
when retained work exhausts it. Global namespace nodes, account bundles, word-ring
backing and authority nodes continue to consume their separate admission.

## Classification, publication and release

The existing `MailboxIdentity` captures the exact root generation, original
verified principal and trusted translation-backing class together before preparation. An owning root
operation is acquired before allocating storage. Later promotion cannot
reclassify a node or an already captured operation; trusted backing does not
grant platform mailbox resource admission. Permanent kernel backing is trusted; raw fixture
namespaces remain ordinary. A stale generation rejects before allocating or
entering the node pool.

Fresh captures after authorized promotion may use trusted reserve under the
existing classification policy; they do not reclassify earlier captures/nodes.

`NodeCharge<MailboxNodes, MetadataSponsor>` admits the actual endpoint-node
layout against the principal's shared 8 MiB metadata allowance and this family's
independent byte/count pool. Both precede allocation. Family rejection refunds
unused principal funding. Node free precedes both backing refunds; neither a
resource-grant change nor a later root lookup chooses the principal.

Endpoint admission precedes namespace-node/account/authority preparation, so an
exhausted endpoint pool rejects before touching the heap. Every partial failure
retains its original root and provisional storage through explicit post-guard
`finish`. Existing receiver reuse needs no new storage, even at total exhaustion.
Publication and detachment allocate and refund nothing under local mailbox and
lifecycle guards. Unfinished preparation and close fallback retain every field
without entering the heap, registries or admission pools.

`mailbox_node_budget::Retired` owns a detached endpoint and keeps its mailbox
resource charge outside the node-release callback. It frees the node before
refunding either backing admission or resource admission. Explicit close,
serialized fixture cleanup and final-root cleanup use that same owner. Unused
preparation also frees endpoint storage before refunding its provisional mailbox
resource charge. The authority record retains its own independently admitted
storage and original capability charge.

Final-root cleanup retains all endpoint nodes inside the existing complete root
receipt. Rejected invalidation preserves the original bytes/count and mailbox
charges. Confirmed retry releases metadata outside local guards, before root
backing and slot completion. Abandonment keeps that exact receipt and its slot;
it cannot authorize ASID reuse or reconstruct a cleanup operation. No cleanup
family, retry owner or physical completion rule changes.

[Principal qualification](../reports/audits/2026-11-06-q4-record-node-principals.md)
checks the real endpoint/authority composition, shared sibling funding, 64 injected
funded allocation failures, 128 heap-held quota denials, receiver reuse, healthy
sponsorship and four endpoint/authority quota prefixes. Rejection preserves
node/resource counts and original principal bytes. The existing unpublished and
retired mailbox abandonment control now uses prebound principals and retains
both funded nodes under history/publication/pool/heap guards, without another
intentional leak or a new physical completion claim.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-01-mailbox-node-admission.json)
records real empty endpoint candidates at ordinary and total node ceilings,
independent byte denial, 64 injected allocator failures with exact recovery and
128 admission denials while holding the actual heap lock. Existing receiver reuse
succeeds under the same pressure. An operation captured before promotion cannot
gain trusted backing reserve; a trusted root uses reserve without receiving
mailbox resource privileges. Closing a promoted root at total exhaustion returns its two original
ordinary nodes; a successor ASID generation consumes exactly that recovered
capacity. Stale captured publication rejects with heap and pool guards held.

Storage-stage rejection fixtures verify exact node rollback; cancellation and
staged-close fixtures also restore the original node snapshot before abandonment
controls. Publication/detachment probes hold the real heap and node pool;
preparation/close abandonment retains two nodes under actual outer guards.
The existing final-root recovery fixture verifies original node charges across
failed invalidation, confirmed retry, ASID reuse and guarded attempt abandonment.
These controls add no extra deliberate-retention fixture. Shared retirement
collection host controls cover deallocation before charge refund.

The initial ARM/Intel fixtures stalled by taking a fresh capture after authorized
promotion while holding the heap lock and expecting ordinary rejection. Debugger
captures identify that admitted allocation waiting on the fixture's own lock.
Both deadline captures remain in the evidence. The corrected fixture retains its
pre-promotion capture for denial and tests fresh trusted capture separately;
production admission and numerical limits did not change for that correction.

Pressure is serialized kernel-fixture work; native heap exhaustion is not forced
and allocator failures are injected. Sustained hostile EL0 fairness, allocator
overhead, aggregate principal/general-heap budgets and other payload/registry nodes
remain open. Wakers/callbacks have admitted [observer handles](observer-ownership.md). SEC-07 remains partial; existing G1/G4
caller, destructor and progress qualification still applies.
