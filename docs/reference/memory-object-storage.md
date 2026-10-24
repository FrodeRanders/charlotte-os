# Memory-object record storage

The global memory-object registry uses prepared admitted nodes. Direct memory
allocation and copy preparation retain one unused object node with their private
backing, after the original page/object budget reservation and before frame
allocation. Node allocation is fallible; rejection disposes the unused prefix
through the existing owner without publishing authority.

Direct allocation claims its object identity before authority publication. A
copy claims its identity during preparation and carries it in `PreparedPayload`.
Publication only relinks the prepared object node. Cancelled preparations leave
identity gaps; identities never wrap, reset or recycle. Checked exhaustion
returns `ResourceLimit` before publication.

Memory-capability and namespace records also use prepared nodes, as described
below. Mixed memory/IPC commit performs no allocation or node disposal after
preparation. Mapping records and frame snapshots now also prepare outside
memory serialization, as described below. Scratch namespace/extent nodes also
prepare outside their local registry. Other preparation/rollback contexts and
comprehensive metadata admission remain separate work. Factory and ordinary rollback
contexts inherited from callers, including enclosing IPC/IRQ holds, remain
separate qualification; no new unlocked interval or root lease is introduced.

## Read-loan records

Read loans also use prepared admitted nodes. The exact destination's shared
capability reservation precedes fallible borrower-node preparation, outside
`MEMORY_OBJECTS`. Source/target generation and loan state are rechecked under
that registry before source escrow and pin mutation. Failed preparation drops
its unused prefix after local guards; no borrower or destination becomes live.
`PreparedTransfer` owns the unused node through cancellation or publication.
Commit relinks that same node after the existing atomic authority publication.
Write loans need no separate borrower allocation.

Revocation retains the complete read-borrower map in its existing `prior` owner.
Uncertain completion or abandonment retains those nodes with the backing pin and
Revoking fence. Confirmed revocation detaches the selected node and its shared
authority together; post-registry disposal frees the borrower node before shared
authority metadata and its original charge. Other readers remain linked.
Namespace cleanup uses the same paired detachment after its existing proof;
already detached borrower payloads cannot be removed twice. Final backing
release explicitly drains any remaining borrower nodes before object-node free
and original backing refund. Failed physical release retains the whole object.

The [borrower storage follow-up](../reports/audits/2026-10-24-security-memory-borrower-storage.md) records native controls and the
subsequently corrected capability/namespace-map boundary. Shared capability counts bound
admitted loans; this adds no separate metadata-byte or aggregate principal
ceiling. Existing containing IPC/IRQ/destructor contexts remain separate work.

## Capability and namespace records

Every direct allocation, copy, move and loan prepares a capability payload node
and a namespace candidate after exact shared-authority reservation. Both factories
run outside `MEMORY_OBJECTS`, before backing allocation or source escrow; copies
retain their already acquired source pin. The existing containing operation owns
the complete unused prefix. Namespace candidates bind the captured root generation;
validation rejects a different generation before shared publication. A missing
namespace may publish its candidate; an existing exact namespace keeps the unused
candidate in the preparation until ordinary post-guard disposal. Empty live
namespaces remain until existing root cleanup.

Publication relinks both prepared node types. No memory capability/namespace
`BTreeMap` insertion remains. Complete mixed batch validation, authority mutation,
object/borrower/capability/namespace insertion and source-payload extraction now
run with the heap held in the serialized maximum-batch control. This qualifies
commit, not all preparation factories, source-escrow cleanup or outer contexts.

Detached capability records pair their payload node and exact shared authority.
Explicit post-registry disposal frees the payload node before authority metadata
and its original count refund. Moves keep the detached source payload in their
existing `PreparedSource`, disposing it before the same `SourceEscrow` releases
its retired authority. Loan cleanup pairs the full capability record with its
borrower node; write-loan cleanup carries the same payload/authority owner.

Direct close and final-pin object destruction retain capability records in the
complete object/backing receipt through physical completion. Failure/abandonment
retains these nodes and their original authority charges with backing, without
retry custody. Namespace cleanup detaches its complete namespace node and pairs
remaining authority in place, then explicitly drains records and frees the node
after memory unlock. No snapshot, metadata ticket or recovery registry is added.

Unused read-borrower storage precedes destination preparation in the containing
transfer's field order. Cancellation frees that unused node and destination
storage before the destination reservation refunds. Successful publication keeps
any unused namespace candidate for ordinary containing-owner disposal; it has no
separate metadata-byte admission. Broader factory/destructor and IPC/IRQ contexts
remain unqualified. Shared capability counts and root counts are not a complete
kernel metadata-byte or principal budget.

The [capability storage follow-up](../reports/audits/2026-10-24-security-memory-capability-storage.md) records evidence and limits.

## Mapping records and snapshots

Mapping admission first validates capability, rights, full range, loan/pin state
and the absence of an existing mapping. The existing caller retains its exact
root throughout. A private preparation then allocates an empty mapping node and
frame-snapshot capacity fallibly outside `MEMORY_OBJECTS`. It holds no frame
identity or pin during that factory interval. Final validation rechecks the
captured object/size and every admission predicate before filling the snapshot,
relinking its node and acquiring the existing retirement pin under one hold.
A removed source or a competing mapping rejects the unused preparation after
local unlock. No table publication or invalidation follows a pin-free rejection.

The table walk and prefix rollback retain their existing serialization. Snapshot
capacity is prepared beforehand, so filling it and publishing the mapping record
need no allocation under the memory registry. Snapshot disposal also leaves
both local memory and table guards. Shared count limits and original backing
ownership remain; there is no new mapping-byte/aggregate-principal pool. Multiple
unused preparations can overlap, and their peak allocation/pressure remains
unqualified.

Ordinary rejected-prefix rollback detaches the original mapping node into the
existing `MappingRetirementPin`, retaining its exact published-prefix count.
Confirmed invalidation and required scratch completion permit explicit node
release before pin decrement/backing release. Failed maintenance or abandonment
retains the complete node and original backing fence, without retry authority.
Explicit unmap retains its live record until leaf cleanup, invalidation and
scratch completion all succeed, then uses that same detached-node owner.

Loan revocation carries its confirmed detached mapping in its existing pin.
Namespace/fixture retirement moves the complete admitted map or selected node
into `RetiredObjectMappings`. Failure/abandonment retains nodes and exact peer
leases without allocator entry. Physical success permits explicit node disposal;
each peer lease stays owned through actual node free and completes afterward.
No new root/pin, registry, recovery family or metadata ticket is introduced.
Containing IPC/IRQ/destructor contexts retain separate qualification.

See the [mapping storage follow-up](../reports/audits/2026-10-25-security-memory-mapping-storage.md) for rejection, revalidation,
heap-held publication and guarded uncertainty controls. Bounded snapshot size,
count limits and serialized controls supply no sustained-pressure fairness proof.

## Scratch namespace and extent storage

Scratch now uses a prepared namespace node plus one prepared node per live
extent. Its original root owner stays retained while factories leave the
scratch registry; MMIO also keeps its existing descriptor claim while leaving
`DEVICES`. Captured namespace generation and current first fit revalidate before
publication. Losing candidates finish explicitly after local guards; partial
abandonment retains all fields. No allocating namespace insertion, extent-vector
growth or silent generation replacement remains.

Exact release detaches its original extent, then frees it after scratch unlock.
Whole-root sealing transfers the complete namespace/extent nodes in the existing
metadata/root receipt through final invalidation. Rejection/abandonment retains
all fields; confirmed release disposes nodes after local guards before root
backing/slot completion. This adds no recovery family or retry proof. Scalar
ranges still depend on the containing mapping/close owner; they are not allocation
nonces. Raw serialized fixture and broader outer contexts remain unqualified.
See [scratch admission](scratch-admission.md) and [evidence](../reports/audits/2026-10-25-security-scratch-storage.md).

## Complete retirement owner

Object removal detaches the complete node into the existing `ChargedFrames`
backing owner, carried by `RetiredMemory` or the existing last-pin cleanup path.
The node retains the object, frame vector, original backing charge and remaining
metadata. It leaves `MEMORY_OBJECTS` before physical release or node disposal.
No teardown snapshot, recovery registry or separately movable metadata ticket
is added.

Physical release keeps the existing sixteen-frame batches. Attempted identities
are consumed before each allocator call and never retried. The owner records
its started/failed fence before the first attempt. Its fields use `ManuallyDrop`;
interruption or rejected release cannot run implicit charge refunds or metadata
cleanup. Ordinary complete release frees payload and node before dropping the
original charge, then clears the fence. Ordinary private rollback retains its
existing release path; this does not qualify every outer destructor context.

A failed physical attempt retains the whole published node and original charge,
even when all frame identities have been consumed. An empty vector is not a
completion proof. This is terminal retention, without retry or recovery custody.
`RetiredMemory` abandonment retains its complete backing owner. Original account
identity survives root retirement and ASID reuse; disposal cannot credit a
successor generation.

## Evidence and limits

The [storage audit](../reports/audits/2026-10-24-security-memory-object-storage.md)
records charged preparation rejection, copied-prefix rollback, heap-held object
relinking/detachment, physical-before-node-before-refund ordering, guarded
abandonment and terminal release rejection across exact slot reuse. Shared
collection tests qualify node deallocation before charge release. The physical
rejection adapter deliberately leaves one data page allocated; it is never
re-adopted or released later.

These nodes use existing page/object admission. They add no comprehensive
metadata-byte or principal heap ceiling. Object lookup, sorted insertion and
cursor scans are now linear; existing count limits do not prove worst-case
hold time, pressure fairness or management progress. Other memory payload maps,
factory/destructor contexts, physical-failure causes and historical failures
remain open. SEC-07/18 remain partial.
