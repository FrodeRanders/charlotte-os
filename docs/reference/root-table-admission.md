# ASID-table backing admission

The global address-space table uses
`IdTable<AddressSpace, registration::table_backing::Charge>`. Like the
[thread table](thread-table-admission.md), it uses complete prepared replacement
buffers and scalar-error insertion; its type cannot invoke the uncharged
allocating `try_add_element` API. Both tables share the same growth/slot machinery.

## Capacity and sponsorship

The [resource policy](resource-policy.md) supplies independent limits for every
retained, candidate and replaced vector set together:

| Dimension | Node | Ordinary work | Each signed principal |
| --- | --- | --- | --- |
| Requested bytes | 16 MiB | 12 MiB | 8 MiB |
| Backing slots | 16384 | 12288 | 8192 |

A set includes the complete payload, generation/lease/closing state and return-ID
vector layouts. Growth starts at four slots and uses powers of two. Full old and
candidate capacities overlap in the accounts until actual deallocation, including
competing preparations and empty retained slots. These defaults leave room for
replacement overlap and trusted headroom; they are separate from the three
per-root [policy/usage/authority nodes](root-metadata-admission.md), translation
frames, allocator overhead and general physical heap backing.

The unpublished root's owning translation account supplies the signed principal
and trusted ordinary/platform classification. The signed loader binds that
account before construction/registration; no ASID, role, manifest or later
authority lookup selects sponsorship. Trusted raw fixture roots with no signed
owner consume node/class admission without manufacturing a principal. Later
identity installation cannot retroactively reclassify that allocation.

The root triggering growth sponsors the whole new buffer set. The previous set
keeps its original charge through post-guard deallocation. Published root close,
principal retirement, later platform promotion and ASID reuse do not refund the
table's retained capacity. A reused slot needs no new capacity charge. Principal
counters live in the existing boot-lifetime history record.

Kernel slot zero is installed once through a separate mandatory bootstrap
preparation. Its trusted class comes from that boot boundary, not an incoming
root or caller-selected numeric identity. Its buffers are charged too; subsequent
ordinary growth cannot use the trusted reserve.

## Registration transaction

`PreparingRegistration` retains the unpublished root, capability/IPC namespaces,
memory account, policy/usage/authority nodes, replacement backing and eventual
publication lease together. Its `ManuallyDrop` fallback retains every field
without entering allocators, counter locks or physical cleanup.

Preparation captures an opaque table identity/capacity request under the table,
then releases that guard before admission and all three buffer allocations.
Under lifecycle/table serialization, registration prepares the hardware ASID,
validates capacity and consumes the root only on successful insertion. It then
relinks the already prepared accounts/namespaces/metadata and retains the exact
new generation through IPC namespace publication after unlock.

A typed `CapacityChanged` means insertion published no root or generation.
Registration may dispose the unused candidate and re-prepare outside its guards,
retaining the same root, hardware tag and all metadata, for at most eight
attempts. Quota, allocation and hardware-ASID rejection are terminal for that
submission. There is no retry of an accepted root or uncertain physical cleanup.
The existing `TableAllocationFailed` rejection covers backing admission,
allocation and exhausted optimistic attempts; the external ABI is unchanged.

Success carries empty old buffers out for consuming disposal. Rejection carries
the complete unpublished root out too. Buffer deallocation precedes the original
charge refund, and root/hardware cancellation remains after local guards.
Successful registration keeps its publication lease through that final disposal;
abandonment cannot make its slot reusable early. Detached root retirement still
requires its independent invalidation and physical completion proofs.

## Qualification and limits

The shared host production-table tests trace allocation/deallocation, partial
buffer construction failure, competing/stale/foreign requests, exact slot leases,
closing retirement, one-time owner consumption and inert abandonment. Native
controls use real principal-bound unpublished roots and real buffers, independent byte/
slot denial, promotion, guarded scalar publication, retained sponsorship after
root disposal and late refund after exact ASID reuse. Registration controls test
one unstarted conflict followed by success and eight-attempt exhaustion, including
exact frame/account recovery after post-guard cancellation. Ordinary node-pool
saturation and trusted headroom use separate counter-only controls.

One fixture intentionally abandons a four-slot buffer set under lifecycle,
root-table, history, heap and pool guards. Its original charge remains retained
after the original root is disposed; there is no reconstruction/recovery path.

The [execution evidence](../reports/audits/evidence/2026-11-01-root-table-admission.json)
records artifact identities, native receipts and the finite target matrix.
These controls do not prove sustained EL0 backing-pressure containment,
all-family principal accounting, essential-service fairness or physical-platform
recovery. The guarded copy scales with admitted capacity and has no qualified
IRQ-latency bound. Wider loader/constructor masks, nested account/control blocks
and physical cancellation contexts remain SEC-07/18 work.
