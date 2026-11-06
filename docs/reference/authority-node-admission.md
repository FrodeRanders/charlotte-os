# Authority-record node backing

Every unified capability record uses independently admitted storage, including
empty candidates prepared before capability-count admission. The existing
`PreparingRecord`, namespace map and `RetiredRecord` carry a typed
`record_budget::Charge` through publication, escrow, detachment and actual
deallocation. Published authority and backing admission have separate lifetimes.

| `authority_nodes` dimension | Ordinary | Total |
| --- | ---: | ---: |
| Requested node-layout bytes | 12 MiB | 16 MiB |
| Prepared/published/retained nodes | 98304 | 131072 |

The count ceilings are twice the existing capability ceilings (49152/65536).
This leaves capacity for unused candidates and overlapping retirement while
preserving capability limits. The current 96-byte native layout, including its captured principal policy and
funded original account reference, reaches count ceilings at 9/12 MiB; independent
byte ceilings bound future layout growth.
The complete backing charge occupies 32 bytes per authority node, a short pool operation at preparation
and release, and another possible `ResourceLimit` rejection. These ceilings
reserve no RAM and cannot guarantee heap availability or progress if retained
work consumes the headroom. A quarter of the pool remains trusted reserve.

## Identity, classification and release

Root preparation captures verified principal and trusted storage classification
from its unpublished translation owner and stores that policy immutably in the
capability namespace alongside the existing resource-grant flag. `PreparedReservation::try_new` captures that
policy under `CAPABILITIES`, verifying the supplied exact generation before
releasing the guard and admitting/allocating its record. Later publication still
revalidates identity and retirement. No new root lease is supplied by this
metadata snapshot. Permanent kernel backing is trusted; raw fixture namespaces
remain ordinary. A missing or mismatched real generation rejects as `Retired`
before allocating a node.

Promotion changes future capability resource grants, without changing the
namespace's backing class. Trusted backing alone cannot grant platform
capability admission. Names, caller roles and a later numeric ASID lookup do not
select the reserve. Node admission funds the actual node layout against the
principal's shared 8 MiB metadata allowance before family byte/count admission and allocation.
`NodeCharge<AuthorityNodes, MetadataSponsor>` shares reserve/rollback/refund
mechanics with other sponsored nodes; the default unsponsored charge retains its
original layout for families whose principal coverage is still pending. Family
rejection rolls back unused principal funding; allocator failure returns
`AllocationFailed` and releases the complete unstarted reservation.

Publication, escrow and detachment only relink prepared storage. The shared
retirement collection frees the node before refunding its byte/count charge;
`RetiredRecord` also retains the original capability resource charge until
deallocation completes. Principal bytes return after that same actual node free,
using the original captured principal. Namespace removal, root retirement and ASID reuse cannot
refund a surviving detached allocation. Existing complete parent owners retain
their records through uncertain cleanup. Unfinished preparation and detached
record Drop remain inert under unknown guards; ordinary unused preparation and
confirmed release use their existing explicit consuming operations.

The [record-node principal qualification](../reports/audits/2026-11-06-q4-record-node-principals.md)
reconciles actual authority/mailbox layouts, sibling sharing, injected allocation
rejection, heap-held quota denial, healthy sponsorship and four mailbox preparation
prefixes. A detached real authority survives root close and exact ASID reuse by a
different funded principal; release returns old bytes and leaves the successor
unchanged. Real authority candidates make the default 8 MiB principal allowance
bind while global ordinary family slots remain. The existing deliberately
abandoned preparation/retirement objects now retain principal bytes under the
history lock as well as publication/heap/family guards; no extra leak is added.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-01-authority-node-admission.json)
records actual empty authority-layout candidates at ordinary and total count
ceilings, independent byte denial, 64 injected allocator failures with exact
recovery and 128 admission denials while holding the real heap lock. The real
publication/detachment fixtures cover all six authority kinds while holding heap
and node-pool guards. Existing preparation/retirement abandonment fixtures retain
two original ordinary nodes under lifecycle/registry/heap/pool guards; no extra
deliberate-retention fixture is introduced.

At ordinary exhaustion a promoted ordinary root still rejects storage, while a
trusted root can allocate backing without receiving a capability reserve grant.
A real detached record retains its original account and bytes/count through
root close and numeric ASID reuse. Explicit release at total exhaustion restores
exactly one ordinary node, which a successor generation can consume. A stale
generation rejects before touching either heap or node pool. Cancellation,
identity exhaustion and allocation rejection restore unused backing.

This does not force native heap exhaustion, qualify sustained concurrent EL0
fairness, bound allocator excess, wider principal consumption or joint heap fit, or change
containing caller/IRQ/implicit-destructor requirements. Other payload/registry
nodes and general heap admission remain open. Wakers/callbacks have admitted
[observer handles](observer-ownership.md). SEC-07
remains partial; existing cleanup owner families and G1/G4 apply.
