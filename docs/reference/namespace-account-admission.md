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
original charge returns. This counts fixed bundles, not exact heap bytes,
allocator overhead or resource payloads. Global namespace nodes have separate
[requested-byte/count admission](namespace-node-admission.md).

The defaults allow twice the four-account needs at the root-metadata pool's
count-only capacity of 3072 ordinary or 4096 total complete roots. Headroom
accommodates independently retained generations, concurrent preparation and
losing lazy-mailbox candidates. A quarter remains trusted reserve. Any type can
consume the headroom, so there is no per-type or per-service entitlement. No
existing limit is raised and no RAM is preallocated.

Using the existing allocation owner adds one holder allocation per account and
one machine word per account reference. Root/account nodes retain their existing
ownership structure. Mandatory roots need three slots before publication;
mailboxes need an additional lazy account. Opening a new mailbox target can
reject when it cannot prepare a fresh candidate even if an existing namespace
would win. Existing receiver reuse and established word queues need no new
account, and continue through that rejection. This preserves preparation before
publication without adding an allocating fallback beneath registry guards.

## Classification and lifetime

Root preparation takes backing classification from its owned, unpublished
translation root. Kernel IPC bootstrap is explicitly trusted. Lazy mailbox
capture records the exact generation and trusted storage class together under
the address-space table hold before account allocation; the operation retains
that capture and root lease through publication/rejection. It never refreshes
classification from a later occupant of an ASID. Permanent kernel namespaces
are trusted; raw kernel fixture namespaces are ordinary.

Backing reserve gives no platform capability, endpoint, IPC-record, mailbox or
word-ring permission. Those adapters retain their existing explicit,
generation-qualified grants. Promotion affects future resource admission only;
an existing account's backing class and final refund cannot change. Names,
roles and caller-supplied manifests are not storage-class authority.

Capability tokens, delegated IPC connections and their closed endpoints,
retired records, mailbox payloads and queue backing retain their original
accounts. Retirement closes the corresponding admission paths, while root
close and exact ASID reuse cannot refund surviving account storage or credit a
successor. Final strong release does not refund weak-only backing. Both account
allocation and private holder must be freed before their original slot returns.

Complete root, namespace and mailbox preparation/retirement owners retain all
fields and charges on abandonment. Their existing guarded controls also hold
the account pool. Ordinary cancellation and confirmed disposal leave local
publication guards before allocation destruction and refund. The leaf pool
only changes counters and releases its lock before allocation; it does not
resolve roots, run callbacks or confer cleanup/retry authority.

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
registry allocations and aggregate principal/general-heap admission remain open. This supplies no
sustained hostile-pressure fairness, arbitrary destructor, worst-case lookup/
IRQ-latency or physical hardware guarantee. SEC-07 remains partial; existing
C02/C03/C10/C18 and G1/G4 boundaries in [cleanup/recovery](cleanup-recovery.md)
continue to apply.
