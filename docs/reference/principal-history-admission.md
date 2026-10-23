# Principal-history storage admission

The shared principal-history table holds sizing history and aggregate resource
counters for the boot lifetime. Its published records and unpublished root
candidates now share a bounded allocation pool.

| Dimension | Whole node | Ordinary preparation |
| --- | --- | --- |
| Requested node-layout bytes | 1 MiB | 768 KiB |
| Allocated or reserved nodes | 4,096 | 3,072 |

Every dimension must fit. One quarter is kept outside ordinary admission for
trusted platform preparation. These fixed kernel limits have no userspace
setter. They cover the exact charged retirement-node layout, including key,
history/counters, link and charge. Allocator overhead, root policy/usage/authority
nodes and other heap allocations remain separate. Count limits bind first for
the layouts exercised by the native controls.

## Preparation and publication

`PreparingHistory` reserves before its fallible node allocation. Signed launch
passes its existing trusted ordinary/platform classification after verification,
before initial architecture-root backing. Root registration captures classification
from the unpublished root's translation account before moving that root into
its registration owner. Artifact names, roles, classes and manifest fields do
not select this reserve.

Each root still prepares a candidate. Installing a new principal moves that
same charged node into the existing history table. If the principal already
exists, its record and counters remain unchanged; the losing candidate stays
in the root's authority slot. It follows the existing closing/final-root receipt
until confirmed invalidation and explicit metadata disposal. No second registry,
new sponsor allocation or post-publication lazy allocation is introduced.

The shared `AdmittedMap` carries the retirement list's typed charge parameter.
Publication and detachment relink existing nodes without allocation or counter
entry. Default uncharged map layouts stay unchanged.

| Event | Admission result |
| --- | --- |
| Node/ordinary denial | Reject before allocating the history node. Signed pre-root preparation reports backing admission failure; registration reports domain-metadata allocation failure without publishing an ASID. |
| Fallible node allocation rejects | Release the unstarted reservation. |
| New principal publishes | Keep the original charge for the boot lifetime. |
| Existing principal reuses its record | Keep the candidate charge with its current root until confirmed disposal. |
| Later platform promotion or record reuse from another class | Preserve every allocation's original classification. |
| Unused preparation cancels | Deallocate its node, then return its original reservation. |
| Failed root invalidation or abandoned candidate/table | Retain the node and original charge. |
| Confirmed final metadata disposal | Deallocate the duplicate/unused node before refund. Published history remains charged. |

Published history is never evicted or reset on domain exit. A node can eventually
reach its record ceiling and reject new preparation even after earlier domains
have exited. Candidates also consume the same finite allowance, so an existing
principal is not promised restart admission at exhaustion. The reserve is shared
headroom; it does not guarantee a complete platform launch or protected-service
latency across other resource families.

## Cleanup and evidence

Candidates remain in C02/C03; this adds no root/pin, physical proof, recovery
custody or retry permission. Their fallback is inert under history, admission,
root and allocator guards. Explicit unused cancellation and confirmed root
metadata disposal retain their existing post-guard context requirements.

The admission counter is an independent leaf mutex. Preparation leaves it before
allocation or history publication. Published-record lookup/update never enters
it; deallocation and final refund happen outside the history/table/lifecycle
holds. No ASID lookup redirects a late refund. See
[cleanup/recovery](cleanup-recovery.md) and
[audit evidence](../reports/audits/2026-10-23-security-history-admission.md).

Native controls fill both pools with real candidate nodes, reject ordinary root
publication, admit a trusted constructor from reserve and close an existing root
at total exhaustion. They check duplicate and failed-invalidation retention,
post-deallocation refund and guarded abandonment. Host allocation tracing checks
the charged map's payload/node/refund order.

The finite `CharlottePrincipalHistory` projection checks shared ceilings,
new-record publication, duplicate/root retention, original classification and
boot-lifetime records. Allocator free and root-invalidation proofs are boundary
inputs. General heap admission, vector/control-block capacity, lookup/IRQ hold
times and sustained concurrent pressure remain outside this pool's qualification.
