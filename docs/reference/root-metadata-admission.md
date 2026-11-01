# Root policy, usage, authority and memory-account storage

Kernel admission capacity for this family comes from the default compile-time
[resource policy](resource-policy.md). Related values can be adjusted together
in a reviewed build profile; see [Configuration](../guides/configuration.md).
Structural/protocol bounds and original-charge/physical-completion rules remain
separate contracts.

Each userspace root prepares four registry nodes: launch limits, usage/telemetry,
signed-authority metadata and its generation-owned memory account. These
allocations share byte/count admission, including partial preparation, detached
retirement receipts and closed accounts retained by outstanding memory objects.

| Dimension | Whole node | Ordinary preparation |
| --- | --- | --- |
| Requested node-layout bytes | 4 MiB | 3 MiB |
| Prepared, published or retained nodes | 16384 | 12288 |

A complete root needs four nodes. Their requested sizes include their payload,
link and typed charge; each type reserves its actual layout before allocating.
The count limit is shared across types. These fixed kernel limits have no
userspace setter. Allocator overhead, root/namespace control blocks,
waiter/timer sponsors and other metadata remain separate. Retained/candidate
[ASID-table vectors](root-table-admission.md) have a separate byte/slot pool and
pre-publication signed sponsor; they are not refunded with these four nodes.

Including the existing memory-account node in this pool increases the count
ceiling from 12288/9216 to 16384/12288, preserving the previous count-only capacity of 4096 total or 3072
ordinary complete roots before other retained/prepared nodes. Native layouts
are 392 bytes for the three policy nodes and 128 bytes for the account, totaling
520 bytes per complete root. Those count ceilings consume 2,129,920 total or
1,597,440 ordinary layout bytes, within the unchanged 4/3 MiB byte ceilings.
The larger count ceiling preserves complete-root capacity while allowing more
simultaneous small nodes; the byte ceiling still bounds mixed/retained storage.
It does not preallocate RAM. A separate pool or combined allocation would
complicate independent retirement lifetimes. Shared admission with extra count
capacity retains the existing ownership structure; other ceilings still apply.

## Publication and lifetime

Preparation uses the existing trusted classification captured from the
unpublished root's translation account. Names, roles, artifact classes and
manifest fields cannot select reserve. Later promotion cannot reclassify an
existing node or change which allowance its final refund returns.

`PreparingAccount` and `PreparingMetadata` retain their admitted nodes inside
`PreparingRegistration` before lifecycle/table publication guards. Quota or
allocation rejection disposes all
unused partial storage after local guards. It returns
`MemorySponsorAllocationFailed` for account preparation or
`DomainMetadataAllocationFailed` for policy preparation, without publishing an
ASID. One, two or three free slots cannot produce a partially published root.

Successful publication moves the same charged nodes into the existing admitted
maps, without allocation or counter entry. Closing detaches the three policy
nodes into the existing root receipt. Failed invalidation and abandonment retain
their storage and original charges. Confirmed metadata disposal destroys each payload and
actually deallocates its node before refund. A missing row, logical exit or
reused ASID supplies no refund authority.

The memory-account node stays in the existing ledger under its exact
`(ASID, generation)` while memory-object charges remain. Successful root teardown
marks it closed; only the final memory charge permits detachment. `forget` and
the final charge's destructor leave the ledger before releasing that node. Its
payload and allocation are destroyed before its original admission is refunded.
ASID reuse, platform promotion and independently surviving waiter/timer sponsors
cannot refund or reclassify it. Those sponsor Arcs still require separate backing
admission; this node's layout covers only their inline owner fields.

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

The [memory-account evidence](../reports/audits/evidence/2026-11-01-memory-account-admission.json)
extends those controls to four-node roots: byte/count denial before allocation,
64 account allocator rejections, all partial preparation phases, publication and
abandonment with heap/pool guards held, one/two/three-slot rollback, and exact
node retention/refund across failed invalidation, root close and ASID reuse.

This bounds these requested node layouts. It does not complete aggregate
principal accounting, a physical RAM ledger or namespace/sponsor/observer heap
admission. One essential service has no guaranteed share of the common reserve.
Concurrent hostile preparation, long-held roots, lookup/IRQ hold times and sustained
management/timer/teardown fairness remain separate SEC-07/18 qualification.
