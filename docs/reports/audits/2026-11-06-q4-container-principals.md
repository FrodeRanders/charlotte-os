# Q4 root and global namespace principal funding

Baseline `9508dd00`. All four root-metadata and five global namespace-node
layouts now fund their requested bytes against the original verified principal.
This covers two more catalog families. Q4 remains in progress; no finding
disposition, aggregate gate, production trust or physical-platform acceptance
changes.

[Version-controlled evidence](evidence/2026-11-06-q4-container-principals.json)
records frozen source/input identities, artifact hashes, selected policy, inline
milestones and both initial native failures separately from final qualification.
The [24-pool checklist](../../reference/metadata-admission.md#principal-coverage-and-remaining-reconciliation)
tracks remaining family coverage, allocator excess and joint heap fit.

## Capture and complete ownership

Root registration captures `CapturedStorage` from its unpublished prebound
translation account and passes the same policy into capability/IPC namespace,
memory-account and launch-policy/usage/authority preparation. Completion and
mailbox preparation pass their existing exact-root capture to the global node
adapter. No later ASID, worker, role or promotion selects the original principal
or refunds. Explicit resource grants, publication validation and physical
completion remain separate.

The existing `NodeCharge<Family, MetadataSponsor>` and shared principal record
supply funding. A common checked node-factory helper converts the actual layout,
reserves principal and family admission, then calls the fallible allocator.
Default unsponsored callers delegate through the same helper and keep their
original representation and scope. No registry, separate node owner or physical
cleanup controller is added.

The typed retirement collection frees each node before its charge returns.
Complete registration, namespace, closing-root and recovery owners keep their
original nodes through uncertainty and abandonment. A generation ledger node
remains independently funded while accepted memory charges survive root close;
its last charge detaches the node under the ledger and frees it outside that
guard. Allocator excess, nested payloads and other families remain separate.

## Capacity and limit choice

The existing **8 MiB metadata allowance** now includes root metadata and all five
global namespace nodes, alongside authority and mailbox endpoint nodes. The
buffer and two account allowances remain independent. There is no new history
field or pool, no new eager allocation and **no limit change**.

| Container layout | Requested bytes |
| --- | ---: |
| Launch limits | 88 |
| Usage/telemetry | 208 |
| Signed-authority slot | 144 |
| Generation memory account | 192 |
| Capability namespace | 152 |
| IPC namespace | 144 |
| Completion namespace | 240 |
| Mailbox capability namespace | 112 |
| Mailbox word namespace | 152 |

The four root nodes total 632 bytes, including the 32-byte funded charge in each
node. Their count-only complete-root capacity needs 2588672/1941504 total/ordinary
bytes, within 4/3 MiB. The five namespaces total 800 bytes. Twice complete-root
count headroom needs 6553600/4915200 total/ordinary bytes, within 8/6 MiB. A bare
registered root needs 928 metadata bytes: four root nodes plus capability and IPC
namespaces. Nine initialized containers need 1432 before their independently
funded payloads, accounts, rings and other backing.

A mix concentrated in 240-byte completion namespace nodes is byte-bound at
34952/26214 total/ordinary nodes before shared count limits. That is an isolated
pool bound, not a launch entitlement or joint-fit claim. Raising the byte pool
would admit more of the largest type and consume more heap; lowering count or
logical limits would restrict other mixes. The documented complete-root mix fits,
so neither adjustment is required. Current references record the larger layouts
and retain this distinction.

The catalog remains 24 pools with 19 byte-denominated independent ceilings,
totaling 205.5/154.125 MiB total/ordinary. The mapped default heap remains 72 MiB.
Those maxima are not disjoint observed usage. Broader allocator/control coverage,
coherent joint heap capacity and sustained service/cleanup progress remain open.
Protocols, SDK, logical limits and signed bundles are unchanged.

## Controls and qualification

Native fixtures prebind private principals at the trusted constructor boundary;
combined security campaigns separately exercise verified signed launch. Two roots
share their 928-byte mandatory funding. Twelve actual registration quota cases
cover zero or incomplete headroom at every container allocation boundary, using
current typed layouts. They preserve original principal bytes, both node pools,
account pools, physical frames and root counts without partial ASID publication.
128 heap-held direct denials reject before allocation; an independent healthy
principal still creates a root.

The five-namespace fixture reconciles actual global namespace additions separately
from authority and endpoint payloads. Losing mailbox candidates remain charged
beside the existing namespace until disposal. Six quota prefixes distinguish
endpoint/authority `ResourceLimit` from the existing namespace-preparation
`AllocationFailed` result, preserving public error behavior and the live receiver.
Existing record-pressure controls retain mandatory container charges and now
reach the same 8 MiB allowance with 87371 real authority candidates plus the
928-byte root, leaving 64 bytes. This is serialized allocation pressure, not
sustained EL0 fairness or essential-service progress acceptance.

The existing accepted-memory-charge fixture retains only its 192-byte generation
ledger node after confirmed root teardown and exact ASID reuse. Its last charge
returns those original principal bytes without affecting the successor. The
complete unpublished registration fixture retains 928 metadata bytes under
history/publication/heap/family guards, alongside its existing account charges.
No additional deliberate leak is introduced.

The existing failed-root recovery fixture now carries funded principals. Before
staging, its containers and payload records use 1672 bytes. Confirmed IPC drain
frees its 144-byte namespace and one family slot before final root invalidation.
The remaining **1528 bytes** stay charged through rejected invalidation, then
return after confirmed one-shot recovery. Exact ASID reuse by another funded
principal and stale extraction leave the successor unchanged. Guarded terminal
custody abandonment retains the same 1528 bytes and complete original root/slot,
mailbox words, authority and physical backing under history/root/namespace pool
guards. These controls qualify that parent; they add no retry or arbitrary
reference-destruction authority.

Shared host controls exercise the production funded factory through rejection,
principal/family rollback and transferred final release. Existing retirement
collection tests independently trace actual deallocation before charge refund.
The full host suite retains its allocation, crypto, protocol and ownership
controls. Native heap exhaustion is not forced.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting, including standalone byte-allocator fixtures | Passed |
| Full host suite and policy profiles | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19; 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each; 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed; 90-second budgets |

The 64 frozen source/input hashes remained unchanged through final qualification.
All five signed service-bundle identities match the preceding record-node batch.
Native AMD DWARF independently confirms all nine container layouts in that exact
qualified kernel. The initial two failures below retain their separate original
source and artifact identities; no timeout was increased.

## Preserved failures and limits

Both initial ARM and Intel campaigns stopped at the new recovery assertion. It
captured 1672 bytes before staging but compared them after confirmed IPC drain;
the observed 1528 bytes correctly excluded the already freed 144-byte namespace.
Production `namespace_close` drains that namespace through `close::drain_namespace_caps`,
which detaches and explicitly frees its node outside IPC. The correction records
funding after staging and independently requires exactly that layout and one
namespace slot to have been released. It preserves all remaining original bytes
through the rejected barrier. No production release logic or deadline changed.
Original diagnostics, kernels, disks, tooling and source identities remain
separate from later qualification; passing repeats alone are not the explanation.

The ARM display second-grant-probe, Intel retirement-pin and ARM exit-watch
incidents remain unresolved. Passing headless campaigns neither explain nor close
them. Remaining principal coverage, allocator excess, joint heap fit, wider
caller/IRQ and implicit-destruction review, physical recovery and sustained
progress remain Q4–Q8 work. Aggregate gates remain open.
