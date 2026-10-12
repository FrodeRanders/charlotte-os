# DMA startup preparation and typed failure evidence — 2026-10-12

Scope: SEC-18 / C01, C13, C15, C18 / G1, G3, G4, G7. Baseline: `cdb1f3b3`.
This follows the unresolved Intel NIC startup failure retained in
[IPC claim readiness](2026-10-12-ipc-close-readiness.md). Current contracts:
[cleanup/recovery](../../reference/cleanup-recovery.md),
[IOMMU admission](../../reference/iommu-table-admission.md), and
[hardware quiescence](../../reference/hardware-quiescence.md).

## Finding and implemented boundary

The historical Intel guest stopped during NIC DMA grant creation with
DmaUnavailable, kernel
`0ab5e61c44b2834ae9feabf75e8807547fca74848f1d4091788a5e2890b9f63b`.
An identical-artifact repeat passed. That adapter collapsed stream resolution,
backend creation and reset errors; the capture cannot identify the underlying
cause. That verification limitation remains, rather than retrospectively
attributing the failure to a particular backend state.

Source review identifies a reachable boot race: storage started before entropy
and network grants were prepared. Its active driver could own the shared IOMMU
unit/command engine across unlocked mapping or maintenance. Domain creation
correctly rejects an unavailable unit/engine before hardware work, but mandatory
boot grant creation then panicked with the collapsed error. This reachable
contention class is verified directly below; it is not proof of the historical
Intel interleaving.

Boot now prepares storage, entropy and network hardware grants before starting
any of those driver threads. `PreparedDriver` consumes one LoadedDomain and an
exact AddressSpaceOperation lease, retained together behind ManuallyDrop through
bootstrap delegation, MMIO/IRQ/DMA grants and thread publication. All driver
roots have no live threads when the batch preparation diagnostic is emitted.
Consuming start completes its root lease after publication. Fallback retains all
fields/counts without callbacks, allocator, root or hardware cleanup. Existing
root accounts, capability records, device descriptors and their charges remain
with those exact roots. This is an instance of existing launch/publication and
live-root ownership, not another custody registry or cleanup family.

This removes sibling-driver contention from the boot grant window without
polling, retries or replay. Independent/dynamic creation still rejects busy
admission. No IRQ masks are removed or force-enabled, and each hardware grant
still follows its existing reset, configuration, capability publication and
activation rules. Mandatory startup failures retain their existing terminal
policy; general dynamic unit readiness/progress is not implemented here.

Creation errors now return a `DmaCreationFailed(Failure)` with Stream, Backend,
ResetAdmission or Reset stage and original backend cause or ResetRejected.
Explicit consuming grant cancellation records Confirmed or Retained while
preserving the initial cause. Retained means the original complete preparation
and charges remain behind their existing fallback; it is not a retry owner.
NotAttempted is the internal pre-cancellation value. No global last-error slot
or diagnostic registry can overwrite another caller's result. IRQ-driver supervisor panic
diagnostics also include the captured root identity and requester. Existing
device status code 13 still represents unavailable DMA on the register ABI;
the richer creation error belongs to the kernel grant interface. Other
admission/publication/activation errors keep their existing representations.

## Direct ownership and failure evidence

Every real complete-unit creation checkpoint now submits a second full grant
against its captured requester. It must return:

```text
DmaCreationFailed(Failure { stage: Backend, cause: Backend(OperationInFlight), cleanup: Confirmed })
```

Unified authority usage must remain unchanged. The existing claim/readiness
preflight rejects before reset or payload work, and ordinary cancellation
completes the temporary root/admission owner. The enclosing exact-root and unit
ownership probes remain active, including available local guards and original
IRQ-state preservation. These are reentrant deterministic checkpoints, not a
scheduled stress reproduction of the historical startup.

Real registry allocation/private-prefix rejection and injected configuration
failure now assert Backend(MapFailed) or Backend(HardwareTimeout) with confirmed
cancellation. Old MMIO authority produces ResetAdmission/ResetRejected, rather
than losing its cause to Unsupported. Original table/authority accounting,
post-guard release and final root checks remain enabled. The synthetic registered
obligation test verifies HardwareTimeout survives both confirmed destruction and
rejected destruction; the latter preserves the same root/reservation and does
not revisit the backend. It adds no new abandoned root to the existing fixture.

Before consuming driver start, batch assertions require no live threads and
matching root/lease identity. Validation features additionally require exact
root close to reject with OperationsInFlight. All successful final guests emit:

```text
[launch] driver grant batch prepared before threads: storage=true entropy=true network=true
```

On the x86 configurations entropy is false because their discovery path did not
select a protected VirtIO RNG; storage and network are true. ARM exercises all
three prepared owners. Existing UART restart, device retirement, staged close,
IPC readiness, NVMe/object-store/Raft and node shutdown tests remain enabled.
Structural complete fallback retention is not an executed kernel-unwinding or
abandoned-driver recovery proof. No physical retry boundary changes.

## Diagnostic-stage and intermediate results

The diagnostic-only Intel guest passed 18/0/0, bitmap `0x123bbff`, at
18.160133 s, kernel
`557fc6ec9fb5b7c4c620d2f921c0ef29c8b5e0a209c44e5bc73f1958e52fa171`.
It exercised precise rejection/cancellation assertions while retaining the old
launch order. No startup error was captured in that run; it does not identify
the historical DmaUnavailable cause.

The first staged-driver Intel and AMD guests passed 18/0/0 at 17.793260 s and
24.207898 s, both bitmap `0x123bbff`, using kernel
`f474efd6c527e301adec63b585f480dc28b2590a3ab4f4ed524fa5b9e056754a`.
All three runners exited 0. These precede the ARM dependency correction and the
final stable-source matrix below.

## Failed ARM dependency runs retained separately

The first ARM security/HTTP/DHCP and shutdown guests reached:

```text
[launch] virtio-rng at BAR4=0x8000008000 requester=0x20
```

They emitted no grant-batch preparation marker, authoritative deferred result,
shutdown drain or power-off. Neither captured a kernel panic. Both runners
exited 1: `authoritative self-test result was not produced within 180s`.
Last timestamps were 6.895874 s and 6.462506 s respectively. Their kernels were:

| Failed artifact | SHA-256 |
| --- | --- |
| ARM initial combined kernel | `fce9848a938305cbe937edfe095c412909d3fec168aae703bdd630d08910ed5f` |
| ARM initial shutdown kernel | `68555c8bcf4dcb709244b95bd0e9c263ddfc26806425b1c78829e14c08d29207` |

Source review establishes a dependency deadlock introduced by that intermediate
batch: ARM fetched RNG and NIC images through the object store, while its own
prepared storage driver was not started until the entire hardware batch finished.
The last RNG log precedes service_elf lookup. The available captures have no
thread-stack snapshot, but the unstarted storage dependency is explicit in the
composition and registry code; no OOM or hardware failure is inferred.

Correction: include signed RNG and NIC images in ARM's minimum bootstrap set.
All boot hardware images are then available before storage startup. Other logical
services retain their store-backed loading path after storage starts. This adds
193,432 signed ELF bytes (net 102,224; rng 91,208) to the ARM embedded image set,
before linker/container overhead. It changes no trust verification, device
admission or deployment signing policy. The complete final matrix is rebuilt
after that correction; passing x86 runs do not substitute for ARM evidence.

## Final validation and artifacts

Both signed bundles were rebuilt before guests with
`scripts/build-catten-services.sh --embed` and
`scripts/build-catten-services-x86_64.sh`. Host suite: **384 passed, 0 failed,
3 ignored**. Formatting, diff and changed Markdown-link checks passed. Strict
`-D warnings` Clippy passed for bundled services, final-source ARM
security/HTTP/DHCP kernel and final-source x86 HTTP/DHCP kernel.

Final stable-source fresh-storage guests used a 180-second runner deadline:

| Profile | Passed / failed / pending | Final bitmap | Guest completion time |
| --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 56.805756 s |
| x86 VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 18.901630 s |
| x86 AMD-Vi, same kernel/features | 18 / 0 / 0 | `0x123bbff` | 22.600957 s |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.566528 s |
| x86 VT-d exact-artifact cold repeat 1 | 18 / 0 / 0 | `0x123bbff` | 23.179652 s |
| x86 VT-d exact-artifact cold repeat 2 | 18 / 0 / 0 | `0x123bbff` | 22.834951 s |

All six runners exited 0 with zero failed/pending bitmaps. Additional synchronous
assertions are outside these deferred counts. Every final guest emitted its
assertion-backed pre-thread grant-batch marker and passed precise creation cause,
consuming cleanup, root retention and existing hardware/IPC ownership checks.
Shutdown drained children at 13.922200 s and requested PSCI power-off at
14.010830 s. The two Intel repeats ran as separate concurrent guests with fresh
storage; these limited samples are not a production reliability or pressure-soak
claim, and no historical failure is overwritten by their passes.

Commands used `CATTEN_SKIP_EMBED_BUILD=1` after both bundle builds:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance dma-start-arm-final
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-start-intel-final
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance dma-start-amd-final
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance dma-start-shutdown-final
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-start-intel-repeat1
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-start-intel-repeat2
```

HTTP/deployment ports were 19325/18625, 19326/18626, 19327/18627,
19328/18628, 19329/18629 and 19330/18630. AMD and both Intel repeats used
`CATTEN_SKIP_KERNEL_BUILD=1` only after matching the current ELF hash against the
immutable final Intel boot hash. Per-boot hashes precede later profile rebuilds:

| Artifact | SHA-256 |
| --- | --- |
| ARM final combined kernel | `bf8b932231e6ae8915d508c47deed43cf04d4adc984115a19c9dc1fab3944b80` |
| Intel/AMD/repeat final kernel | `3a94acbc9a46ca255f804bbdb843767a4a8263d8ba0b8dda8f95b2f670a39bbe` |
| ARM final shutdown kernel | `e17d5d802d7b93f12ad5ac34c66e0dd9605d0eca25a4acc85cf844cd5f893c99` |
| ARM bundled service ELFs | `e8bf681a4f01d71c30cecfe9d13f1362e3b7e9f8c79a04e078d3befbcd915621` |
| x86 bundled service ELFs | `2bf773dfb18cbd457ddbe31a7c2d81816e5d8d147d9e57a6425f11f4ee5bc999` |

Bundle hashes use sorted newline-terminated `basename:ELF-sha256` lines hashed
with SHA-256. Evidence is inline; temporary logs and ignored build artifacts are
not dependencies of this published report.

## Remaining limits

The boot ordering excludes one identified sibling-contention window and creation
errors now preserve evidence. The older Intel failure remains causally unresolved.
Dynamic unit/engine readiness, indefinite admission pressure, general launch
rollback/Drop/outer IRQ contexts and a production custody/controller policy
remain open. Diagnostics, confirmed unstarted cancellation and passing repeats
do not authorize replay of uncertain or partially released creation. SEC-18 and
SEC-07 remain partial, with existing production enrollment restrictions intact.
