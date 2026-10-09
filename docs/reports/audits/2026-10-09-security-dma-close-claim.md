# DMA close claim without allocating reinsertion

This follows the [complete-unit creation correction](2026-10-09-security-dma-creation-phases.md)
within C13/C14 and G1/G2/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial: **20 corrected, six partial and four open** findings,
with eighteen owner families and seven acceptance gates. No registry, new owner
family or recovery interface is added.

## Defect and correction

Explicit DMA close removed its device payload from the capability registry
before backend destruction. A returned hardware error reinserted that payload
with an allocating `BTreeMap::insert` under device serialization. Extraction
could also destroy a map node there. The public capability lacked its own
in-flight claim through this interval, so abandonment could lose its payload
record even while the backend retained unfinished ownership.

Close now uses the existing [`DmaOperation`](../../../crates/catten/src/device/mapping.rs)
owner shared with map/unmap. It revalidates authority, leases the exact root and
claims the original capability record under lifecycle/device serialization.
Local guards leave before hardware work. The record and authority remain in
place through backend destruction; ordinary close/map/unmap and namespace
preparation reject its busy claim. No failure path extracts or reinserts a node.

A returned backend error already restores or retains its complete registered
maintenance/physical owner. Public completion revalidates the exact cap/domain,
clears only its busy claim in the original cell and explicitly completes its root
lease. Authority and the original charge survive. Retiring domains still reject
mapping; frozen physical release remains terminal. This is not a new physical
retry receipt or permission to reclaim backing from an error code.

Only confirmed backend success permits exact cap/domain revalidation and
one-time authority/payload consumption, followed by root completion. The shared
owner's inert fallback retains root/claim through interruption or abandonment.
Its existing complete-backend-owner contracts retain uncertain tables, queues,
pins and metadata separately. No destructor clears the public claim or invokes
hardware. The old scalar extraction/reinsertion branch is removed.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Root/capability or busy admission rejects | No hardware work or payload mutation; ordinary unused root admission completes explicitly. |
| Claimed close before/during backend destruction | Original payload, authority and exact root remain; competing public operations reject. |
| Returned backend error | Original backend owner is restored/retained. Public claim completes in place without allocation/deallocation, preserving authority and charge. Existing backend phase determines whether maintenance retry is possible; frozen physical backing never retries. |
| Confirmed backend success | Exact cap/domain claim is revalidated before authority/payload removal. Root completion follows after device unlock. |
| Staged root close | Remains Pending through the older close lease. Confirmed close consumes authority before that same closing owner progresses. |
| Abandoned close before terminal publication | Root, original claim and authority remain; no forced clearing or destructor cleanup. Backend ownership retains uncertain physical work. |
| Interrupted terminal authority/payload removal or root completion | This phase can start only after backend success. Retain the unfinished root/metadata dependencies; authority or the payload may already have been consumed. Never infer permission to repeat physical release from a remaining record. |

## Execution evidence

Serialized synthetic busy, hardware-timeout and physical-error returns execute
with the **actual heap allocator held**. The whole rejection path must complete
without allocating or deallocating registry nodes. Tests require the same payload
address, unchanged domain ID, visible authority, original capability charge and
cleared ordinary claim afterward. Nested close/unmap and immediate exact-root
close reject while the claim is live. These callbacks inject error codes; they
do not simulate actual hardware timeouts or partial physical release.

A separate synthetic success fixture stages close of the exact root during its
older capability close. It remains Pending and rejects new operation leases;
confirmed fake backend success consumes authority before that same closing owner
finishes. Existing guarded `DmaOperation` plus complete synthetic domain/engine/
pending-pin abandonment tests exercise the same shared root/claim fallback.
There is no new intentional table/data/root retention: table-only totals remain
VT-d **38/29**, AMD-Vi **34/25**, SMMUv3 **40/31** charges/frames; independently
charged retained roots/data frames remain documented in earlier reports.

Real QEMU NVMe close injects rejection after actual hardware detachment and
maintenance submission, preserving the same payload address, authority and
charges. The subsequent close adapter carries the public claim through actual
pre-maintenance and post-drain/before-physical-release probes. They require
local lifecycle/device/root-table/physical/heap guard availability, preserved
entry IRQ state, nested close/unmap rejection, busy exact-root close and visible
claimed authority. Existing backend probes also require mutation/reset exclusion
and live data-pin protection. Only actual backend success removes authority and
payload once. Reassignment and operational storage tests run afterward.

## Validation

Selected diagnostics omit routine boot output:

```text
[DMA close ownership] busy/timeout/physical rejection with heap held preserves the same payload cell and authority; nested close/map and exact-root close reject; confirmed close consumes authority before the original staged root completes
[DMA close recovery] original payload cell/authority survived actual rejected drain; public close claim fenced nested close/map/root through real maintenance and physical release, then confirmed success consumed authority once without reinsertion
```

| QEMU target, four LPs and fresh isolated storage | Authoritative result |
| --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; security probe `0xffff`, publication generations 1/2, 4,884 cancellation requests retired |

Both x86 runs used SHA-256
`238fa937afe539c69052100218ba150ec8bd22f0a79b7f6c7ae466767bf9acb5`.
Arm used
`be03a12b921a71dc026c04dd30955653198b63a8f0ddb6a99a1f3c77857afc93`.
All three emitted both selected diagnostics and passed on their first attempt;
no guest validation failed for this change. Historical root-lease timeouts remain
separately documented and causally unresolved.

Strict default-feature kernel Clippy passed for both custom targets with
`--locked -- -D warnings`. Rustfmt, diff checks, local documentation links/anchors
and table shapes passed; eighteen owner families/seven gates are unchanged.
Runners reused validated service bundles, rebuilt kernels and enforced assembly
permissions (249 x86 native entries, one Arm). Guests ran sequentially after the
last Rust change. The host harness was not rerun for kernel-only changes. Arm
used the authorized runner because its local forwarding ports are sandbox-blocked.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance dma-close-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance dma-close-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18709 CATTEN_DEPLOY_HOST_PORT=18009 scripts/run-aarch64.sh --security-test --instance dma-close-arm-20261009 --fresh-storage --timeout 180
```

## Limits and remaining work

This corrects DMA close's allocating rejection and missing public claim. General
device grant metadata allocation remains infallible, and confirmed-success
`BTreeMap`/capability-node removal still has its existing serialized destruction
context. Those G4 boundaries require admitted storage and post-guard disposal;
this change does not declare all device registry operations allocation-free.
MMIO/interrupt close retain their existing separate physical/route contracts.

Wider syscall/fixture masks and implicit field destruction remain G1 work.
Serialized nested probes do not establish cross-LP races or interruption at every
instruction, actual timeouts, failed physical release on a live device, outstanding
I/O, pressure/progress guarantees or physical-device quiescence. No administrative
retry/custody or supervisor reconciliation is added. Historical intermittent
user-stack root-lease timeouts remain causally unresolved; passing guests do not
close them or SEC-18.
