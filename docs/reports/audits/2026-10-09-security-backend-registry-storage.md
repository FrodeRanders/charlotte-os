# Admitted IOMMU backend registry storage

This continues the [device authority-context correction](2026-10-09-security-device-authority-context.md)
within C14/C15/C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md). SEC-07 and
SEC-18 remain partial: **20 corrected, six partial and four open** findings,
eighteen owner families and seven acceptance gates. No new registry or custody
interface is introduced.

## Defect and correction

Complete-unit creation already left lifecycle/backend guards, but domain cells
and requester fences still used infallible `BTreeMap` insertion after reset and
physical domain construction. VT-d context metadata insertion followed hardware
root-link publication. Domain destruction removed its empty map cell below backend
serialization and cleared the requester before finishing data unpins.

All three backends now use shared `retirement_list::AdmittedMap` nodes for domain
cells and requester/stream fences; VT-d uses them for context metadata too.
[`backend_registry::Preparing`](../../../crates/catten/src/device/backend_registry.rs)
keeps typed fallible node preparation inside the existing grant's `DmaCreation`.
The existing complete-unit claim excludes competing registry mutation while
preparation runs. All required nodes precede reset, domain-ID mutation, physical
domain backing and descriptor publication. Partial allocation rejection remains
owned until ordinary post-guard grant cancellation. No scalar cleanup ladder or
new allocation/custody registry is added.

Publication only relinks admitted nodes. VT-d records the context frame before
publishing its hardware root link. Confirmed reset updates the original zero
requester fence in place rather than replacing/deallocating it. Successful
destruction keeps its empty domain cell and nonzero requester through physical
table release and all data unpins, then detaches the domain node under backend
serialization for explicit disposal after unlock. Requester/context nodes remain
unit-owned. Existing typed drain, maintenance, private rollback and frozen
physical-release proofs are preserved.

The original grant retains partial/unused backend nodes beside its exact root,
authority reservation, reset claim and private/registered hardware obligation.
`Nodes` fallback retains its `ManuallyDrop` fields without allocator, registry,
hardware or logging work. Metadata makes creation armed even before hardware;
attempting reuse rejects before initialization/reset/allocation. Explicit unused
node disposal requires confirmed grant rollback or publication, after local
guards and before exact-root completion. Retained metadata is not a retry owner.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Domain/source/context node rejects | Unstarted backend work; partial nodes remain in the original grant. Ordinary cancellation disposes after guards and refunds authority before root completion. |
| Reset/private construction rejects | Existing complete grant retains metadata and hardware dependencies. Dispose only after confirmed one-shot rollback; uncertainty is terminal retention. |
| Domain/context publication | Relinks prepared nodes without allocation. Context metadata precedes hardware linkage; domain obligation precedes reachable descriptor publication. |
| Maintenance rejects before physical release | Existing exact typed owner/engine restoration policy applies. No new metadata or generic retry permission. |
| Physical release rejects or is abandoned | Original domain cell/source claim and frozen complete owner remain retained. No partially freed walk is retried. |
| Confirmed table/pin completion | Detach original domain node, preserve the existing zero reset fence, release metadata outside backend guards. |
| Grant/preparation abandonment | Exact root, original authority reservation and unfinished nodes/obligations remain owned indefinitely. No forced claim clearing or custody ticket. |

## Execution evidence

The [metadata fixture](../../../crates/catten/src/device/backend_registry/tests.rs)
rejects each preparation stage, then explicitly disposes partial unused nodes.
Heap-held publication, claimed-cell extraction/restoration and node detach use
the actual allocator lock. Disposal happens afterward; payload Drop counters
verify no destruction under that hold. A second domain reuses the exact requester
value address, proving in-place fence reuse rather than replacement. The x86
adapter fixture covers all three node types on both Intel and AMD guests; Arm
covers domain/source nodes.

A typed complete-grant fixture abandons metadata before any hardware/backing
work while backend, lifecycle, device, both CPU-table, physical/heap and original
IOMMU-pool guards are held. It retains **one additional exact root-operation lease
and one original capability reservation/charge**, unused device/unified storage
and three backend nodes on VT-d or two on AMD-Vi/SMMUv3. Architecture-specific
CPU-root backing and existing namespace/account metadata remain retained too.
Exact root close returns `OperationsInFlight`; IOMMU table charges and actual
physical free-frame count do not change during the guarded fallback. No new
IOMMU table or DMA data frames are allocated for this probe. This is controlled
abandonment, not actual panic unwinding.

The [real NVMe fixture](../../../crates/catten/src/device/recovery_tests.rs)
rejects every required node on its fresh requester. It verifies unchanged next
domain ID, source/domain/context registry snapshot, table charges and original
authority usage, with available PCI config and disabled bus mastering. Normal
creation, reset/reassignment, private/published rollback, map/unmap, pin retention
and acknowledged destruction then execute against the same backend.

Selected diagnostics, common to the passing guests:

```text
[backend registry rejection] every required node rejected before reset/domain IDs/backing; original requester/registry snapshot, table charges and authority preserved
[backend registry ownership] node rejection, heap-held publication/claimed-cell restoration/detach and exact zero-fence reuse passed; guarded complete-grant abandonment retains one root/reservation and unused backend nodes without new IOMMU table/data charge
```

Preparation and disposal **entry** probes require local backend/lifecycle/device/
capability, CPU-table and physical/heap guard availability and preserved entry
IRQ state. Allocation/deallocation necessarily enters the allocator afterward;
these counts are phase visits, including rejected/empty preparation disposal,
not whole-boot allocation balances. Source inspection establishes ordering
around reset/context linkage and all data unpins; serialized probes do not prove
every enclosing caller or cross-LP interruption.

| Target | Registered tests | Adapter preparation/disposal entries | Real NVMe preparation/disposal entries |
| --- | ---: | ---: | ---: |
| Intel VT-d | 15 passed, zero failed/pending | 10 / 9 | 21 / 21 |
| AMD-Vi | 15 passed, zero failed/pending | 10 / 9 | 14 / 17 |
| Arm SMMUv3 | 19 passed, zero failed/pending | 6 / 7 | 18 / 21 |

Final x86 passed bitmap is `0x1bbff`. Arm is `0x2003ffff`, with scoped security checks `0xffff` at publication
generations 1 and 2.
Existing IOMMU table-only intentional retention stays VT-d **38 charges/29
frames**, AMD-Vi **34/25**, SMMUv3 **40/31**, independent of this additional
root/authority/metadata fixture. No real hardware timeout or outstanding I/O is
injected. Earlier intermittent user-stack lease timeouts remain causally
unresolved; these passing runs do not close C16/G1/G2/G7 or SEC-18. The
[current ledger](../../reference/security-remediation.md) preserves that limitation
and links the earlier failed runs separately from their passing repeats.

Fresh-storage guests run sequentially with previously validated embedded service
bundles because this change affects only the kernel:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance backend-registry-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance backend-registry-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18759 CATTEN_DEPLOY_HOST_PORT=18059 scripts/run-aarch64.sh --security-test --instance backend-registry-arm-20261009 --fresh-storage --timeout 180
```

Kernel SHA-256:

- Intel/AMD x86: `99d3f7dccf6af610db4021dc052468c49d10c572e052753df72c852d3eaf83c2`.
- Arm security guest: `9ab497363e6efa24d8fdd4095f8c8fe02acaf2dfbe34cf0d434a7529d81cb885`.

Strict locked default-feature Clippy passes on x86 and Arm (`-D warnings`).
Formatting and diff checks pass. Boot runners verify native assembly section/load
permissions (249 x86 entries, one Arm entry). The shared admitted-map source
and host tests are unchanged; the preceding eight-test/full-host-harness result
remains in the [namespace report](2026-10-09-security-capability-namespace-storage.md).
That harness is not rerun in this kernel-only pass; the new ownership probes run
inside all three guests.

## Remaining boundary

Per-domain mapping `BTreeMap`s, SMMU L3 walker metadata, quarantine/ledger
collections and all enclosing allocation/destruction contexts still require
qualification. This correction supplies fallible registry storage and explicit
ownership, without a general heap-byte/principal ceiling or essential-progress
guarantee. Shared admitted-map lookup/insertion is linear. Real OOM, concurrent
pressure/interruption, actual outstanding device I/O, unsupported reset targets,
abandoned-owner custody and physical-platform recovery remain open. SEC-07 and
SEC-18 retain their original acceptance criteria.
