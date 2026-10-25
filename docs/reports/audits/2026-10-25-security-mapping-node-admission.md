# Security audit: memory-mapping node admission

Date: 2026-10-25. Reviewed baseline: `d7881862`.

Memory-object mapping records now reserve their requested node layout and count
before allocation: **1 MiB / 8,192 total**, **768 KiB / 6,144 ordinary**. Original
admission follows the same node through unused preparation, publication, selected
or whole-map retirement and abandonment, until actual node deallocation.

The [tracked evidence](evidence/2026-10-25-security-mapping-node-admission.json)
records source/model/artifact hashes and relevant diagnostics. This advances
R7-2/R7-3 and C05/C18. The ledger remains **29 corrected / 6 partial / 3 open**;
SEC-07/18 remain partial and production trust remains disabled.

## Acceptance scope

| Gate | Added enforcement/evidence | Remaining work |
| --- | --- | --- |
| R7-2/R7-3 | Shared node-layout/count admission covers overlapping and retained mapping nodes before allocation. | Other metadata families, complete allocation inventory, aggregate signed-principal/general heap and allocator overhead. |
| R7-4/R7-5 | Real-node exhaustion, trusted reserve and existing-owner cleanup at total capacity. | Sustained concurrent EL0 pressure and measured management/timer/teardown progress; shared reserve alone proves no fairness. |
| R18/G1 | Original node/charge stays in the existing complete owners through guarded publication, retirement and abandonment. | Broader caller/destructor/IRQ contexts, concurrent hardware failures and authorized recovery custody. |

The [current contract](../../reference/mapping-node-admission.md) describes the
precise policy. Snapshot buffers retain their separate existing admission.
No new root/pin, registry, recovery family, custody or physical/replay authority
is introduced. No finding disposition changes.

## Allocation lifetime and captured classification

Mapping preparation captures trusted policy against its exact caller namespace
handle before counter/factory entry. A reused scalar ASID cannot select a
successor's reserve; caller names/roles/manifests supply no reserve claim. Late
promotion cannot reclassify the node's original charge or refund. Snapshot policy
is captured independently at its own existing preparation boundary.

The independent leaf counter reserves the complete requested node layout,
including payload, link and typed charge, then leaves before the fallible node
allocator. Quota or allocation rejection returns `ResourceLimit`. Rejected
allocation releases unused admission, and ordinary partial preparation explicitly
finishes its existing complete prefix after local memory/table guards.

Prepared storage, the object's mapping map and selected/whole-map retirement
owners now use the same charged node type. Publication and detachment relink;
neither reallocates, refunds nor enters the counter. The common `release_node`
keeps admission outside payload disposal and actual node free, refunding only
afterward. Any captured mapping peer lease completes after that refund. Inert
preparation and detached-node/list fallback retain allocations/charges without
entering allocator, registry or counter. Failed physical cleanup retains the
existing complete pin/root/record; node refund supplies no quiescence proof.

Requested layout excludes allocator overhead/excess storage, free heap backing,
snapshot buffers and other metadata. A single node type reaches whichever
dimension applies first. On all three tested targets the layout is **120 bytes**:
the count ceilings bind at **737,280 ordinary / 983,040 total requested bytes**,
below the byte ceilings. These tests do not independently exhaust the byte caps
or force global allocator OOM.

## Verification

Serialized kernel controls exercise:

- **64 allocator-adapter rejections**, with the exact byte/count reservation live
  and local memory/table/heap/frame guards available, then exact baseline refund.
- Classification captured before promotion, retained through root retirement and
  real ASID reuse, then refunded against the original ordinary share.
- Isolated metadata publication/detachment while the counter is held. The charge
  remains live through payload disposal; common node-free ordering refunds last.
  No leaf or physical completion proof is fabricated by that isolated control.
- Actual ordinary and reserve mapping-node allocations to the real count caps;
  **64 ordinary quota rejections** and a total-cap rejection occur before the
  allocator. Public `map` and `map_any` reject without snapshot reservation,
  mapping/pin publication or new data leaves. Trusted public mapping uses reserve
  at ordinary exhaustion, and its confirmed unmap restores the prior counters.
- At total exhaustion, an actual ordinary mapping unmaps and invalidates using
  its existing record. Even after promotion its node refunds the original share.
  The root then closes; a real successor with the same ASID/new generation uses
  the freed slot for its own public mapping and closes independently. All unused
  nodes deallocate, restoring the exact original admission baseline without
  resizing/reset/synthetic refunds. Free heap backing is not a RAM-free baseline.
- Existing 192 mapping preparation prefix rejections now assert both snapshot and
  node-admission restoration. Heap-held publication also holds both counters.
  Existing guarded partial/full/published fallback retains **two unused nodes**;
  the existing uncertain real-prefix rollback retains **one mapping node** with
  its two original data pages/root/backing charge. Those selected existing
  footprints now retain **360 requested node bytes / three original ordinary
  slots**. No new user-data/root quarantine is added; this is not total retention.

Actual-node pressure has **6,132 ordinary + 2,048 reserve unused nodes on Arm**
and **6,124 + 2,048 on each x86 backend**, plus one public ordinary mapping.
Preexisting live/retained ordinary mappings occupy the remaining **11 / 19** slots
respectively. Those baselines are included in the same ceiling rather than reset
or discounted. The evidence records each backend's exact observation.

Existing actual backend map/rollback/unmap/destruction, loan/cancellation,
returned-resource, root-custody and readiness controls remain enabled. Strict
base-kernel Clippy passes on both architectures. The kernel-coupled path runs
directly; no alternate host implementation is introduced.

Each backend's first guest execution completes successfully, with all **42 HTTP
checks** and both security-service generations per backend. Host request attempts
remain separate from guest results:

| Backend | Deferred tests (passed/failed/pending) | Healthy beside stalled peers | Trickle rejection | Cancellation requests |
| --- | --- | --- | --- | --- |
| smmuv3 | 20/0/0 | 0.343–0.370 s | 30.15 s | 19,096 |
| intel-vtd | 18/0/0 | 0.390–0.449 s | 30.29 s | 17,700 |
| amd-vi | 18/0/0 | 0.207–0.368 s | 30.20 s | 17,724 |

No guest failure or repeat occurs in this batch. No deadline, attempt bound or
admission limit is widened. Passing runs do not attribute historical failures.

Initial metrics observations, retained with surrounding host output:

- **smmuv3:** HTTP transport TimeoutError; retry 1/2; then PASS within the original bound.
- **intel-vtd:** PASS with no preceding retry diagnostic.
- **amd-vi:** PASS with no preceding retry diagnostic.

All transport retry diagnostics remain inline in the evidence, separately from
guest completion. Any host timeout remains unattributed; passing retries do not
explain historical failures or supply physical/replay authority.

Workspace formatting, whitespace and local references pass. Native executable
assembly-section checks remain enabled. Bundled services are reused; userspace
sources and the tracked manual PDF remain unchanged. Historical failed runs keep
their separate evidence and verification limits; passing guests do not attribute
their causes. No deadline, attempt bound or admission limit is widened.

## Formal qualification and remaining work

All **23 focused harness cases** pass: three safe configurations and **20 expected
counterexamples**. DMA operation explores **29,175 distinct / 62,003 generated
states**, depth **24**; IPC transaction explores **26 / 71**, depth **11**;
root retirement explores **2,978 / 3,733**, depth **33**. Required safe actions
execute; negative configurations violate their named invariant through the
intended unsafe action.

No model inputs change. Existing root original-charge, IPC loan/rollback/timeout
and DMA ownership projections remain relevant. None represents concrete mapping
node layouts, counters, classification, allocator or destructor/IRQ contexts.
Native/source controls qualify those boundaries without executable refinement,
arbitrary-schedule or sustained-pressure/fairness proof.

The acceptance gaps above remain open. A shared platform reserve grants no
guaranteed share to one essential service. Other metadata, aggregate principal/
general heap, broader outer contexts and sustained concurrent progress still
need work. Historical mapping/root-close/metrics failures remain unattributed.
Node refunds authorize neither physical recovery nor accepted-work replay.
The capacity values remain family-local compile-time constants. Central typed,
operator-configurable resource policy is outstanding; enforcing a ceiling does
not establish a suitable configuration mechanism for future workloads.
