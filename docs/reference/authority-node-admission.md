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
preserving capability limits. The current 64-byte native layout reaches count
ceilings at 6/8 MiB; independent byte ceilings bound future layout growth.
Admission adds 16 bytes per authority node, a short pool operation at preparation
and release, and another possible `ResourceLimit` rejection. These ceilings
reserve no RAM and cannot guarantee heap availability or progress if retained
work consumes the headroom. A quarter of the pool remains trusted reserve.

## Identity, classification and release

Root preparation captures trusted backing classification from its unpublished
translation owner and stores it immutably in the capability namespace alongside
the existing resource-grant flag. `PreparedReservation::try_new` captures that
class under `CAPABILITIES`, verifying the supplied exact generation before
releasing the guard and admitting/allocating its record. Later publication still
revalidates identity and retirement. No new root lease is supplied by this
metadata snapshot. Permanent kernel backing is trusted; raw fixture namespaces
remain ordinary. A missing or mismatched real generation rejects as `Retired`
before allocating a node.

Promotion changes future capability resource grants, without changing the
namespace's backing class. Trusted backing alone cannot grant platform
capability admission. Names, caller roles and a later numeric ASID lookup do not
select the reserve. Node admission rejects before the allocator; allocator
failure returns `AllocationFailed` and releases its unstarted reservation.

Publication, escrow and detachment only relink prepared storage. The shared
retirement collection frees the node before refunding its byte/count charge;
`RetiredRecord` also retains the original capability resource charge until
deallocation completes. Namespace removal, root retirement and ASID reuse cannot
refund a surviving detached allocation. Existing complete parent owners retain
their records through uncertain cleanup. Unfinished preparation and detached
record Drop remain inert under unknown guards; ordinary unused preparation and
confirmed release use their existing explicit consuming operations.

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
fairness, bound allocator overhead or aggregate principal consumption, or change
containing caller/IRQ/implicit-destructor requirements. Other payload/registry
nodes, waker/callback storage and general heap admission remain open. SEC-07
remains partial; existing cleanup owner families and G1/G4 apply.
