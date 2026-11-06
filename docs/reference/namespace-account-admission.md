# Capability, IPC and mailbox account backing

New roots prepare one unified capability account and two IPC accounts: endpoint
records/queue slots and connection/call/reply records. Mailbox endpoint and word
queue adapters lazily prepare a fourth account shared by those two interfaces.
These fixed shared account allocations have backing admission independent of
their capability, record and ring limits.

| Policy field | Ordinary | Total |
| --- | ---: | ---: |
| `namespace_accounts` | 24576 | 32768 |

Each slot covers one fixed-layout account Arc and its private `ChargedAllocator`
holder. The four typed account families share this pool. Reservation precedes
either allocation; unused holder/payload rejection frees backing before the
original slot returns. The family pool counts fixed bundles. Signed principals
also have a **1 MiB actual requested-layout allowance** for these four account
types in their existing boot-lifetime history record. That separate allowance
funds both actual Arc allocations: payload/control/alignment and private holder.
It does not count allocator bookkeeping/excess or resource payloads. Global namespace nodes have separate
[requested-byte/count admission](namespace-node-admission.md).

The defaults allow twice the four-account needs at the root-metadata pool's
count-only capacity of 3072 ordinary or 4096 total complete roots. Headroom
accommodates independently retained generations, concurrent preparation and
losing lazy-mailbox candidates. A quarter remains trusted reserve. Any type can
consume the headroom, so there is no per-type or per-service entitlement. No
existing limit is raised and no RAM is preallocated.

The existing allocation owner retains one private holder allocation per account.
Its captured backing allocator funds both holder and payload with their actual
layouts; no Arc header formula is duplicated and no additional holder or registry
is introduced. Account references carry the immutable funding token inline. Root/account nodes retain their existing
ownership structure. Mandatory roots need three slots before publication;
mailboxes need an additional lazy account. Opening a new mailbox target can
reject when it cannot prepare a fresh candidate even if an existing namespace
would win. Existing receiver reuse and established word queues need no new
account, and continue through that rejection. This preserves preparation before
publication without adding an allocating fallback beneath registry guards.

## Classification and lifetime

Root preparation takes backing classification from its owned, unpublished
translation root, including its prebound verified principal. Signed launch
prepares the principal record before root construction. Kernel IPC bootstrap
is explicitly trusted and unidentified; trusted raw preparation/fixtures retain
family admission without claiming signed-application coverage. Lazy mailbox
capture records the exact generation, original principal and trusted storage class together under
the address-space table hold before account allocation; the operation retains
that capture and root lease through publication/rejection. It never refreshes
classification or principal from a later occupant of an ASID. Permanent kernel namespaces
are trusted; raw kernel fixture namespaces are ordinary.

Backing reserve gives no platform capability, endpoint, IPC-record, mailbox or
word-ring permission. Those adapters retain their existing explicit,
generation-qualified grants. Promotion affects future resource admission only;
an existing account's backing class, principal and final refund cannot change. Names,
roles and caller-supplied manifests are not storage-class authority.

Capability tokens, delegated IPC connections and their closed endpoints,
retired records, mailbox payloads and queue backing retain their original
accounts. Retirement closes the corresponding admission paths, while root
close and exact ASID reuse cannot refund surviving account storage or credit a
successor. Final strong release does not refund weak-only backing. Both account
allocation and private holder must be freed before their original slot returns.
Each allocation refunds its original principal bytes only after actual free. An
extra allocator clone can therefore retain holder bytes after payload free; final
strong release still cannot refund weak-only payload storage.

Complete root, namespace and mailbox preparation/retirement owners retain all
fields and charges on abandonment. Their existing guarded controls also hold
the account pool. Ordinary cancellation and confirmed disposal leave local
publication guards before allocation destruction and refund. The leaf pool
only changes counters and releases its lock before allocation; it does not
resolve roots, run callbacks or confer cleanup/retry authority.

## Byte funding and capacity

`ChargedAllocator<C, P, A>` retains the same typed family reservation `C` and
payload policy `P`, with a captured backing allocator `A` for both heap objects.
The default `Global` keeps unmigrated owners' representation and policy. These
accounts select `SponsoredAllocator<NamespaceSponsor>`; its common byte mechanics
are also used by `ByteAllocator` for existing buffers. Constructor, partial and
parent fallback keep their original inert ownership. Ordinary rejection cancels
after local guards; allocation denial restores unused byte and bundle prefixes.

`Policy.memory.principal_namespace_account_bytes` defaults to 1 MiB, independently
of the 16 MiB kernel-buffer allowance. This prevents retained account storage from
consuming the buffer/IPC-cleanup byte allowance; neither ceiling guarantees
progress at saturation. Sibling roots and retained generations share this same
principal account ceiling. It provides no per-principal bundle-count share or
saturated cleanup guarantee. No new family pool, eager backing or resource-count
limit is added. Global bundle ceilings remain 24576/32768 ordinary/total.
Current 64-bit native measurements are 72/80/104/104 bytes per complete
capability/endpoint/record/mailbox bundle: a 24-byte private holder plus the
actual payload Arc. One root needs 256 bytes for its three mandatory accounts,
or 360 bytes with mailbox backing. The largest fixed 104-byte bundle gives a
2.4375/3.25 MiB ordinary/total upper bound from the existing count ceilings;
slots remain retained through partial preparation, weak backing and allocator
clones. The four account constructors are the only supported consumers of this
fixed-layout pool. Extending that set requires revisiting this bound.

Principal funding is inline in each account reference, which now occupies
32 rather than 16 bytes on the qualified targets. Enclosing node layouts include
those reference fields automatically; their general principal coverage and
allocator bookkeeping remain separate Q4 work. The runtime pool record still
reports bundle counts, not principal byte usage. The version-2 policy census
reports selected principal ceilings; no new runtime budget mutation surface is
introduced.

The [principal qualification](../reports/audits/2026-11-06-q4-namespace-principals.md)
records actual layouts, reference-field costs, the derived global count-to-byte
bound and rejection/retention controls. Larger workload profiles may need a
reviewed principal ceiling; lowering it can reject mandatory root or lazy mailbox
preparation. A limit change must include joint heap and latency qualification.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-01-namespace-account-admission.json)
records real capability-account allocations to both shared ceilings, 256
heap-held denials across all four constructors and zero/one/two-slot root
rollback with exact frame, root and account recovery. Trusted root/mailbox
backing uses reserve while actual authority, endpoint, connection, mailbox and
word-queue adapters still consume ordinary resource admission before a grant.
Closing a promoted root at total exhaustion returns its original four ordinary
slots and permits fresh root/mailbox creation.

Existing real-root controls retain capability tokens, delegated IPC connections
and closed endpoint storage, then check original-account final weak refunds
after exact ASID reuse. Mailbox controls check retired identity rejection and
final weak release without affecting a successor. Sixty-four IPC publications
and detachments run beneath heap/account-pool guards. Existing partial and
complete parent abandonment retains the same objects with their new charges;
no additional deliberate-retention fixture is introduced.

Account pressure allocates actual capability-account bundles, rather than the
largest possible mixed footprint or resource payloads. Shared allocation-owner
host tests cover holder/payload failure, clones, weak lifetime and deallocation
ordering; native heap exhaustion is not forced. Global namespace nodes have
[separate admission](namespace-node-admission.md), including empty preparation.
Wakers/callbacks have admitted [observer handles](observer-ownership.md). Other
registry allocations, other account families, allocator excess and aggregate
general-heap admission remain open. This family now has original-principal
actual-layout funding; the [current checklist](metadata-admission.md#principal-coverage-and-remaining-reconciliation)
keeps that scope separate from the remaining Q4 work. This supplies no
sustained hostile-pressure fairness, arbitrary destructor, worst-case lookup/
IRQ-latency or physical hardware guarantee. SEC-07 remains partial; existing
C02/C03/C10/C18 and G1/G4 boundaries in [cleanup/recovery](cleanup-recovery.md)
continue to apply.
