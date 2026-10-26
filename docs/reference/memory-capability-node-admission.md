# Memory-capability and read-borrower node admission

Memory-capability payload records, namespace nodes and read-borrower nodes share
requested node-layout byte/count admission before allocation. This closes one SEC-07
gap: shared authority counts alone cannot bound unused payload/namespace candidates
or abandoned metadata after namespace authority has been retired.

| Dimension | Ordinary callers | Total |
| --- | --- | --- |
| Requested node-layout bytes | 3 MiB | 4 MiB |
| Prepared, published or retained nodes | 24576 | 32768 |

These are the defaults of `Policy.memory_capability_nodes`, selected through the
central [resource policy](resource-policy.md). They are compile-time adjustable with
`CATTEN_RESOURCE_POLICY`; see [Configuration](../guides/configuration.md). No
boot-time input or live resizing is introduced.

## Admission and ownership

The existing `cap_storage::Preparing` captures trusted classification once against
the exact destination namespace handle, before any of the three node factories.
Names, roles and caller-supplied principals grant no reserve. Promotion and ASID
reuse cannot change a node's original class or eventual refund.

Each node reserves its actual requested layout, including payload, link and typed
charge, plus one count before its fallible allocator. All three layouts share one
pool; the first applicable dimension rejects. Allocation/quota rejection returns
`ResourceLimit`. Failure on a later node explicitly disposes the unused prefix and
then cancels its hidden authority, before source escrow or pin mutation. No payload
capability or data backing is published by that rejected preparation.

Even a destination that already has a namespace prepares a candidate: that candidate
consumes quota until explicit post-guard disposal. A preparation needs two temporary
slots, or three for a read loan. Established namespaces retain only the new payload
on ordinary success, and also the borrower node on read-loan success; write-loan
borrower state remains inline. Size the pool for overlapping preparations, retained
generations and cleanup uncertainty as well as live authority. Raising the unified
capability count does not automatically raise this independent metadata limit.

The same typed charge follows the node through `Caps`, namespace and borrower
storage, source-transfer retirement, read/write-loan cleanup and complete
object/backing receipts. Publication/detachment only relink storage and enter no
admission counter. Confirmed disposal destroys the payload and frees the node before
its original charge returns. Independent unified authority remains retained until
its own existing disposal boundary.

The capability preparation owner’s partial/full/published fallback retains all
fields without registry, allocator or counter entry. Namespace teardown may retire
hidden authority while abandoned payload/namespace metadata and its original
admission remain retained. Logical exit, disappearance of authority, promotion or
root reuse cannot refund that storage. Uncertain physical cleanup keeps its original
complete owner and backing charges; this metadata admission adds no physical proof
or retry owner.

## Qualification and remaining scope

The [audit
follow-up](../reports/audits/2026-10-25-security-memory-capability-node-admission.md)
records allocator-adapter rejection for both layouts, actual ordinary/total
exhaustion, trusted reserve, two-node partial rejection, public allocation/
move/copy/loan rejection without persistent source/destination change, detached
lifetime and real cleanup/root reuse at exhaustion. Guarded abandonment retains four
unused nodes and their original charges through real root retirement and successor
reuse.

The [read-borrower
follow-up](../reports/audits/2026-10-26-security-borrower-node-admission.md) adds
the third layout to the same pool. Actual ordinary/total exhaustion leaves two
available slots: write-loan preparation can cancel normally, while read-loan
preparation rejects without source mutation. Trusted reserve admits a real read
loan; mapped revocation at exhaustion frees the exact borrower and capability nodes
with their original charges, including after promotion. A native control restoring
the old uncharged factory fails the third-node assertion. Unused borrower
cancellation still follows the active containing transfer policy; this does not
qualify arbitrary outer destructor/IRQ contexts.

This advances R7-2/R7-3 and C05/C18. It bounds these requested node layouts,
including their inline owning fields; nested authority/account allocations, object
nodes, allocator overhead and general heap/principal admission remain separate. It
supplies no universal RAM-fit or essential-service fairness guarantee. Sustained
concurrent EL0 pressure and broader outer guard/IRQ/ destructor contexts remain
SEC-07/18 acceptance work.
