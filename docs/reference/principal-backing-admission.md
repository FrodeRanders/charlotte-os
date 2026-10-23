# Aggregate backing admission by principal

Every signed artifact principal has one backing ceiling shared by its live and
retired address-space generations on this node. Starting another instance,
restarting a domain or moving its objects to a service cannot reset this ceiling.
The principal comes from verified artifact metadata, not a deployment request's
name, a role claim or the current occupant of a reusable ASID.

| Dimension | Initial kernel policy |
| --- | --- |
| Combined memory-object, committed heap and ELF/runtime pages | 16,384 pages (64 MiB) per principal |
| Memory-object backing allocations | 1,024 objects per principal |

Both dimensions must fit. Existing generation, family/node, ordinary-share and
physical-floor checks remain in force. A node can therefore reject below the
principal ceiling. Images and heaps consume pages without consuming an object
unit; aliases and pins do not duplicate either charge. These are fixed kernel
limits, with no userspace setter or deployment override. Trusted platform roots
also share their signed principal's ceiling: promotion does not refund it or
turn retained ordinary backing into new principal headroom.

## Binding and admission

The existing boot-lifetime principal-history record now holds the aggregate
counter alongside stack/heap history. Each root already prepares its admitted
record storage before publication. First identity installation links that storage;
other instances reuse the same record without allocating under accounting guards.
There is no second principal registry.

The signed loader binds identity and backing sponsorship before its optional
preparation callback, ELF/runtime mapping or thread publication. Binding admits
all backing already present in trusted preparation in one checked reservation,
then splits that owner into the root's heap/image accounts and its generation's
memory-object account. Rejection leaves authority absent and all three accounts
unbound; the containing launch preparation owns cleanup. Raw unbound roots are
kernel preparation/fixture contexts, not an application launch path.

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
| Normal root destruction | Heap/image charges return after confirmed physical teardown. |
| Rejected or abandoned provisional cleanup | The existing account retains the principal share, including a reservation abandoned before frame allocation. |
| Partial/failed root release | The complete original heap/image charge remains retained, matching the existing node-charge policy. |
| Confirmed final object release | Physical release precedes generation and principal refund. |

Root backing accounts keep their aggregate owners behind the same retention
fence as their node charge. An implicit field destructor cannot refund uncertain
backing. Preparation abandonment only merges captured shares; it takes no
principal/pool/allocator lock. No retained counter supplies custody, physical
completion proof or permission to replay cleanup. See
[cleanup ownership](cleanup-recovery.md), [joint backing preparation](kernel-backing-preparation.md)
and [memory-object sponsorship](memory-object-budgets.md).

## Verification and limits

Native kernel controls exercise mixed real heap/image/object backing across
multiple roots, the independent object-count bound, copying/moving to another
principal, trusted promotion, 256 rejected rounds alongside unrelated allocation,
retained transfers across exit/reuse, and atomic identity binding. Two verified
instances of the same signed image also share the image/object ceiling. A failed
pre-launch callback can leave an externally held object charged to that same
principal. Existing preparation/root-release fault controls now reconcile exact
principal retention against their node charges.

The security fixture installs a smaller 128-page/128-object generation ceiling
before thread publication. Its sibling workers retain their common signed
principal and ordinary classification. This isolates exact-page restoration
checks without treating a shared principal ceiling as private capacity.

The finite `CharlottePrincipalBacking` model checks aggregate page admission,
transfer, retirement, explicit physical confirmation and one reused root slot.
Its negative controls use per-generation-only admission, early retirement refund,
move-based sponsorship changes and late release against the replacement principal.
It abstracts unit pages and does not model the object-count dimension, allocator
metadata, hardware, wall-clock progress or executable refinement.

This implements an aggregate boundary for these three backing families. It does
not bound general kernel heap/metadata, stacks, page tables, capabilities, IPC,
completions or every other family by principal. Principal-history records remain
for the boot lifetime; their total metadata needs separate admission. Principal-record lookup and IRQ-masked hold times still need hostile-pressure
qualification. Kernel-only unidentified work retains its existing family/node policy. Independent principals
can still reach the ordinary node ceiling, and the platform reserve is shared
headroom rather than guaranteed capacity per essential service. Concurrent EL0
multi-instance pressure, restart churn and full management/teardown fairness
remain SEC-07 R7-3/R7-4/R7-5 work. See the
[dated audit](../reports/audits/2026-10-23-security-principal-backing.md).
