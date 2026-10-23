# Aggregate backing admission by principal

Every signed artifact principal has one backing ceiling shared by its live and
retired address-space generations on this node. Starting another instance,
restarting a domain or moving its objects to a service cannot reset this ceiling.
The principal comes from verified artifact metadata, not a deployment request's
name, a role claim or the current occupant of a reusable ASID.

| Dimension | Initial kernel policy |
| --- | --- |
| Memory-object, committed heap, ELF/runtime and private-table pages, plus full user-thread stack capacity | 16,384 pages (64 MiB) per principal |
| Memory-object backing allocations | 1,024 objects per principal |

The page dimension counts reserved stack capacity even before every data page
is committed. Both dimensions must fit. Existing generation, family/node,
ordinary-share and physical-floor checks remain in force. A node can therefore reject below the
principal ceiling. Images and heaps consume pages without consuming an object
unit; aliases and pins do not duplicate either charge. These are fixed kernel
limits, with no userspace setter or deployment override. Trusted platform roots
also share their signed principal's ceiling: promotion does not refund it or
turn retained ordinary backing into new principal headroom.

## Binding and admission

The existing boot-lifetime principal-history record now holds the aggregate
counter alongside stack/heap history. Verified launch prepares that admitted
record before even initial private-root backing; other instances reuse it without allocating under accounting guards.
The root registration owner still prepares its namespace/policy metadata before
publication. There is no second principal registry.

The signed loader verifies the artifact signature/metadata and prepares the immutable sponsor before
calling the architecture constructor. x86 attaches it before PML4 admission;
AArch64 attaches it before its lazy root can allocate. The constructor retains
its original trusted ordinary/platform classification. Constructor or namespace
failure therefore cannot leave signed private-root backing outside the principal
ceiling.

Before optional preparation or thread publication, authority installation checks
that any prebound table owner has this same principal. It admits all remaining
trusted-preparation backing in one reservation and splits it among the original
owners. Previously charged tables are not billed twice. Rejection leaves authority
absent and new bindings unchanged; the existing table owner retains its exact
charge until confirmed cleanup. Raw unbound constructors remain kernel
preparation/fixture contexts, not the signed application launch path.

Each subsequent allocation first reserves its share of the same principal
counter. Failure of generation/node/physical/mapping admission returns that
reservation only through the existing confirmed rollback path. Memory objects
retain the aggregate share in their original generation's account; page
preparations carry a typed share into the embedded owning account. Counter work
allocates nothing and enters no allocator or subsystem.

## Retirement and retained backing

| Event | Aggregate result |
| --- | --- |
| Copy | The copying caller pays for new backing, including when a different principal receives the capability. |
| Move or loan | Original sponsorship stays in place. |
| Sponsor exit or ASID reuse | Outstanding object backing remains charged to the original principal and generation. |
| Normal root destruction | Heap/image/private-table charges return after confirmed physical teardown. |
| Rejected or abandoned provisional cleanup | The existing account or stack reservation retains the principal share, including admission abandoned before frame allocation. |
| Partial/failed root release | The complete original root-backing charge remains retained, matching the existing node-charge policy. |
| Confirmed final object release | Physical release precedes generation and principal refund. |

Root backing accounts keep their aggregate owners behind the same retention
fence as their node charge. An implicit field destructor cannot refund uncertain
backing. Preparation abandonment only merges captured shares; it takes no
principal/pool/allocator lock. No retained counter supplies custody, physical
completion proof or permission to replay cleanup. See
[cleanup ownership](cleanup-recovery.md), [joint backing preparation](kernel-backing-preparation.md)
and [memory-object sponsorship](memory-object-budgets.md).

User-thread stack admission borrows the root's immutable sponsor and reserves
its configured maximum user data plus sixteen kernel data pages before allocation.
The existing `StackSlot` keeps that share with its root lease and bitmap slot.
Explicit unused cancellation or confirmed release of both stack halves consumes
it once. Drop, failed retirement and uncertain kernel-half cleanup retain the
whole reservation. Data growth within that capacity does not pay twice; any new
private table branch still consumes its own table page. Cached branches remain
charged until root teardown. Pure kernel threads and inherited stacks retain
their existing separate policy; they do not gain an application principal.

## Verification and limits

Native kernel controls exercise mixed real heap/image/object backing across
multiple roots, private sparse hierarchies, full stack-capacity admission, the
independent object-count bound, copying/moving to another principal, trusted promotion, 256 rejected rounds alongside unrelated allocation,
retained transfers across exit/reuse, and atomic identity binding. Two verified
instances of the same signed image also share the image/object ceiling. A failed
pre-launch callback can leave an externally held object charged to that same
principal. Existing preparation/root-release fault controls now reconcile exact
principal retention against their node charges.

The security fixture installs a smaller 128-page/128-object generation ceiling
before thread publication. Its sibling workers retain their common signed
principal and ordinary classification. This isolates exact-page restoration
checks without treating a shared principal ceiling as private capacity.

The finite `CharlottePrincipalBacking` model checks unequal object/table/stack
footprints, prepaid data growth, transfer, retirement, explicit physical confirmation
and one reused root slot. Negative controls also charge growth twice or refund an
abandoned reservation. It abstracts page counts and does not model object-count
admission, allocator metadata, hardware, wall-clock progress or executable refinement.

This implements an aggregate boundary for these data/table/stack families. It does
not bound general kernel heap/metadata, fresh shared kernel tables, IOMMU/device metadata, capabilities, IPC,
completions or every other family by principal. Principal-history records remain
for the boot lifetime; their total metadata needs separate admission. Principal-record lookup and IRQ-masked hold times still need hostile-pressure
qualification. Kernel-only unidentified work retains its existing family/node policy. Independent principals
can still reach the ordinary node ceiling, and the platform reserve is shared
headroom rather than guaranteed capacity per essential service. Concurrent EL0
multi-instance pressure, restart churn and full management/teardown fairness
remain SEC-07 R7-3/R7-4/R7-5 work. See the
[table/stack follow-up](../reports/audits/2026-10-23-security-principal-structure.md)
and the [initial backing audit](../reports/audits/2026-10-23-security-principal-backing.md).
