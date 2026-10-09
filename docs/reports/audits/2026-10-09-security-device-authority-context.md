# Device authority preparation and retirement contexts

This continues the [unified record-storage correction](2026-10-09-security-capability-record-storage.md)
within C12/C13/C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md). SEC-07 and
SEC-18 remain partial: **20 corrected, six partial and four open** findings,
eighteen owner families and seven acceptance gates. No new registry or custody
interface is introduced.

## Defect and correction

Unified record storage was fallible and detached outside `CAPABILITIES`, but
device grants still prepared it under lifecycle. MMIO close destroyed its
authority node under lifecycle/device serialization before mapping invalidation;
successful DMA close destroyed it under the device guard. MMIO invalidation or
scratch failure therefore retained its root/descriptor but had already refunded
the original authority metadata charge.

Shared [`PreparedReservation`](../../../crates/catten/src/capability.rs) now owns
fallible record storage, optional unused kernel namespace preparation and its
captured namespace identity. Device
[`GrantAdmission`](../../../crates/catten/src/device/publication.rs) prepares this
beside its existing device nodes and exact root before local publication guards.
Under lifecycle it revalidates captured generation, closing state, memory-budget
admission and capability admission before charge/serial mutation. It never
acquires a fresh root by numeric ASID during reservation. Rejection leaves
unused storage and any original charge with the preparation; explicit disposal
leaves local guards before completing the root. Grant abandonment retains all
fields through its existing `ManuallyDrop` owner. Other captured authority callers
use the same machinery but still require their own outer-context qualification.

Non-DMA [`PreparedClose`](../../../crates/catten/src/device/close.rs) now contains
its root operation, descriptor, detached payload node and `RetiredRecord` together.
Claim detaches authority under lifecycle/device serialization without destroying
the node or charge. MMIO retains its busy reset-visible descriptor through
unlocked detach/invalidation/scratch completion. IRQ route removal remains under
device serialization; its detached metadata stays in the containing owner.
Cleanup starts once, and explicit finish requires either an unstarted admission
failure or confirmed cleanup. Only then are payload and authority metadata
disposed outside local guards and the exact root operation completed last.
Failure or abandonment retains the whole operation without allocator, registry,
hardware or logging work in Drop. There is no started-close retry.

DMA's existing [`DmaOperation`](../../../crates/catten/src/device/mapping.rs)
continues to preserve live authority and its original payload claim on backend
rejection. After confirmed destruction it stores both detached metadata owners
in that same operation, leaves the device guard, disposes them explicitly and
then completes the root. Whole-domain
[`PreparedNamespaceDevices`](../../../crates/catten/src/device/retirement.rs)
similarly retains current authority retirement while consuming its existing
admitted payload work list outside local guards. Hardware failure still leaves
unfinished authority live; no forced completion or acknowledgement is added.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Device or unified node preparation rejects | Ordinary explicit disposal/root completion precedes local publication guards and hardware. No capability charge/serial or device payload publishes. |
| Captured policy/generation/closing rejects | Preparation remains owned; ordinary finish follows guard release. No backend work/publication. |
| Serial exhausted after provisional charge | Charge stays in the preparation until post-guard explicit finish. |
| Grant publication | Relinks prepared device and unified storage. DMA keeps its complete hardware/reset obligation until explicit activation. |
| MMIO/IRQ close claim | Authority is inaccessible; original metadata charge and exact root remain owned. MMIO descriptor stays busy/reset-visible. |
| MMIO detach/invalidation/scratch rejects | Terminal retention of original charge, descriptor, root, scratch and unfinished payload. No restoration, replay or forced refund. |
| DMA backend close rejects | Original payload cell/authority remains; ordinary public claim/root completion follows existing backend-owner policy. Physical uncertainty does not authorize retry. |
| Confirmed close | Payload and authority nodes detach without allocation/destruction under local guards; explicit post-guard disposal precedes exact-root completion. |
| Grant/close/namespace owner abandonment | Complete unfinished dependencies stay retained; inert metadata fallback performs no hardware or heap cleanup. No custody ticket is published. |

## Execution evidence

The device fixture injects first/second device-node and unified-node rejection
for MMIO, IRQ and DMA. All return `ResourceLimit`, retain no published device
namespace/authority and never reach fake hardware callbacks. Preprepared grants
reserve authority and publish while the **actual heap is held**, including fresh
device namespace linkage and existing namespace reuse. Serial exhaustion under
heap/lifecycle holds retains one provisional charge, then explicit post-guard
completion refunds it. Existing quota, retired memory-budget, namespace retirement,
captured generation and closing-root tests still execute.

Scratch MMIO, direct MMIO and IRQ close claim while the actual heap is held,
verifying logical authority removal keeps its original charge. Their staged root
close remains Pending through physical completion and metadata retirement;
explicit owner finish refunds metadata and lets that exact closing owner complete.
Unknown-capability rejection separately confirms ordinary root-lease completion.

Three mapped-MMIO probes inject detach, invalidation-result and scratch-release
rejection. Real invalidation remains attempted; injection never manufactures a
hardware acknowledgement. Two further probes abandon claimed MMIO and IRQ close.
Dropping the owner under backend/lifecycle/table/physical/heap/device guards
retains **five original authority charges, five exact root-operation leases and
four scratch pages**, with their architecture-specific root/private-table backing
and metadata. These are additional to the preceding record-storage fixture's two
retained ordinary metadata charges. No new device data frames are allocated.
Root close returns `OperationsInFlight`, MMIO close remains busy and scratch
cannot reuse the retained reservation. The abandoned IRQ metadata cannot touch a
fresh grant of the same INTID; the fresh root closes normally.

Selected close diagnostic:

```text
[device authority close] heap-held MMIO/IRQ claim retains original charge; scratch/direct/IRQ success disposes metadata before staged root completion; detach/invalidation/scratch rejection and MMIO/IRQ guarded abandonment retain five root leases, five authority charges and four scratch pages
```

Authority preparation/disposal entry probes require local lifecycle/device/
backend/capability, CPU-table and physical/heap guard availability with unchanged
entry IRQ state. Record allocation/release also uses the existing capability/
heap and per-operation IRQ probes through real NVMe recovery. Source inspection
and heap-held admission/detach verify no allocator work occurs in the claimed
publication phases. The admission fixture records 7 preparation/10 disposal entries, and composed
close records 9/22 on each target. The real-sequence counts below are selected
phase visits, not whole-boot allocation balances; inactive reservation/account
disposal is included.

| Target | Registered tests | Real NVMe authority preparation/disposal entries |
| --- | ---: | ---: |
| Intel VT-d | 15 passed, zero failed/pending | 19 / 42 |
| AMD-Vi | 15 passed, zero failed/pending | 16 / 36 |
| Arm SMMUv3 | 19 passed, zero failed/pending | 20 / 44 |

Final x86 passed bitmap is `0x1bbff`; Arm is `0x2003ffff`, with scoped
security checks `0xffff` at publication generations 1 and 2. Existing real NVMe
drain, reset exclusion, creation rollback,
map/unmap/pin and physical-retirement fixtures pass with these metadata probes.
Existing IOMMU table-only retention remains VT-d **38 charges/29 frames**,
AMD-Vi **34/25**, SMMUv3 **40/31**, independent of the new CPU-root/authority
retention. Hardware timeout is injected as an error in the synthetic close suite;
no real timeout or cross-LP race proof is claimed.

Final fresh-storage guests run sequentially with previously validated embedded
service bundles, because this change affects only the kernel:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance device-authority-intel-final-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance device-authority-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18749 CATTEN_DEPLOY_HOST_PORT=18049 scripts/run-aarch64.sh --security-test --instance device-authority-arm-20261009 --fresh-storage --timeout 180
```

Kernel SHA-256:

- Final Intel/AMD x86: `a2f152b15954722eaa892de2e44db4a11fab709a3f184e471083f7201aa1ee57`.
- Arm security guest: `99bfd0433a91f6d0bfef4350f3dff324edea409eba6d375a9a421f2643281d32`.

Strict locked default-feature Clippy passes on x86 and Arm (`-D warnings`).
Formatting and diff checks pass; boot runners verify native assembly
section/load permissions (249 x86 entries, one Arm entry). The shared admitted-map
source/host tests are unchanged; their
preceding eight-test/full-host-harness result remains in the
[namespace report](2026-10-09-security-capability-namespace-storage.md).
That harness is not rerun in this kernel-only pass; the new ownership probes
execute inside all three guests.

## Failed initial run and preserved policy check

The initial Intel artifact
`092aad23ed77da242d551e9120fd7b21a7d73552633285705840132938b2d41f`
passed the new close/admission fixtures but failed the existing memory-budget
retirement test. It produced no completed self-test summary and the runner exited
1 after its 180-second capture window. Relevant diagnostic:

```text
device/admission_tests.rs:118: assertion left == right failed
left: Ok(4100); right: Err(NamespaceRetired)
```

The intermediate preparation path retained capability-budget and closing checks
but omitted `memory::budget::accepting` from the old lifecycle reservation helper.
Memory-budget retirement alone therefore admitted a new grant. The corrected
shared lifecycle preparation checks that policy against its **stored exact root**
before capability charge/serial mutation. The existing test requires rejection
for all three device categories, including refusal to reach DMA hardware.
Final validation above uses the corrected artifact; this failed run is recorded
separately. It is a deterministic implementation regression caught before commit,
and gives no causal evidence for the historical intermittent user-stack lease
timeouts.

## Remaining work

Other unified authority callers and active token destructors still need full
outer-context inventory; IOMMU backend registry storage and other metadata need
admission/destruction qualification. Enclosing syscall masks, concurrent pressure,
real OOM, physical-device recovery, aggregate byte/principal limits and complete
custody/reconciliation remain open. Historical intermittent user-stack lease
timeouts remain causally unresolved. Passing guests do not close SEC-07/18.
