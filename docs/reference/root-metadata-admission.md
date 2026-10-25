# Root policy, usage and authority storage

Kernel admission capacity for this family comes from the default compile-time
[resource policy](resource-policy.md). Related values can be adjusted together
in a reviewed build profile; see [Configuration](../guides/configuration.md).
Structural/protocol bounds and original-charge/physical-completion rules remain
separate contracts.

Each userspace root prepares three registry nodes: launch limits, usage/telemetry,
and signed-authority metadata. These allocations now share byte/count admission,
including partial preparation and detached retirement receipts.

| Dimension | Whole node | Ordinary preparation |
| --- | --- | --- |
| Requested node-layout bytes | 4 MiB | 3 MiB |
| Prepared, published or retained nodes | 12288 | 9216 |

A complete root needs three nodes. Their requested sizes include their payload,
link and typed charge; each type reserves its actual layout before allocating.
The count limit is shared across types. These fixed kernel limits have no
userspace setter. Allocator overhead, root/namespace/account control blocks,
waiter/timer sponsors and other metadata remain separate.

## Publication and lifetime

Preparation uses the existing trusted classification captured from the
unpublished root's translation account. Names, roles, artifact classes and
manifest fields cannot select reserve. Later promotion cannot reclassify an
existing node or change which allowance its final refund returns.

`PreparingMetadata` retains each admitted node in its current transaction before
lifecycle/table publication guards. Quota or allocation rejection disposes all
unused partial storage after local guards. It returns
`DomainMetadataAllocationFailed` without publishing an ASID. In particular, one
or two free slots cannot produce a partially published root.

Successful publication moves the same charged nodes into the existing admitted
maps, without allocation or counter entry. Closing detaches all three into the
existing root receipt. Failed invalidation and abandonment retain their storage
and original charges. Confirmed metadata disposal destroys each payload and
actually deallocates its node before refund. A missing row, logical exit or
reused ASID supplies no refund authority.

The authority payload may also contain a losing
[principal-history candidate](principal-history-admission.md). That separate
allocation keeps its own charge; the authority node's layout accounts for its
inline owning field. Disposing the candidate does not refund the authority node.
Published boot-lifetime history remains independent of root-record disposal.

These are the existing C02/C03 owners in [cleanup/recovery](cleanup-recovery.md).
No root/pin, registry, custody, physical completion proof or retry owner is added.
Abandonment remains inert under publication, accounting and allocator guards.
Ordinary cancellation and final disposal retain their existing post-guard
requirements. The admission counter is a leaf mutex, released before allocation;
it never enters root/registry/allocator locks or re-resolves an ASID.

## Evidence and limits

The [audit follow-up](../reports/audits/2026-10-24-security-root-metadata-admission.md)
records allocation failures for all three layouts, real ordinary/total exhaustion,
rollback with one/two slots available, trusted reserve and existing-root close at
exhaustion. Detached-root controls retain all three charges on invalidation
failure and return exactly their original reservation after confirmation.

This bounds these requested node layouts. It does not complete aggregate
principal accounting, a physical RAM ledger or namespace/account/observer heap
admission. One essential service has no guaranteed share of the common reserve.
Concurrent hostile preparation, long-held roots, lookup/IRQ hold times and sustained
management/timer/teardown fairness remain separate SEC-07/18 qualification.
