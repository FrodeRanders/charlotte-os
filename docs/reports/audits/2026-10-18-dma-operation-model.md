# DMA operation and readiness model — 18 October 2026

## Scope

This follows the [IPC publication model](2026-10-18-ipc-publication-model.md)
and [root/wait synchronization](2026-10-18-security-model-sync.md), reviewed
against implementation `1c6963a2`. It adds
[`CharlotteDmaOperation`](../../tla/CharlotteDmaOperation.tla), a safe bounded
configuration and thirteen required counterexamples. Source comparison also
found a fresh CPU-remap gap with coherent DMA pins. The accompanying
[correction and native regression](2026-10-18-coherent-dma-remap-fence.md)
close that path.

The earlier DMA model checks stream ownership, pinning, CPU/loan exclusion and
acknowledged revocation before pin release. It abstracts backend admission and
runtime error ownership. The new model follows one map, its fixed-unit wait,
and possible unmap/CPU teardown through those additional boundaries.

## Source comparison

Reviewed implementation anchors:

- [`device/mapping.rs`](../../../crates/catten/src/device/mapping.rs):
  `DmaOperation`, `MappingMaintenance`, typed `UnmapError`, map diagnostics and
  public claim/root completion.
- [`domain_creation/readiness.rs`](../../../crates/catten/src/device/domain_creation/readiness.rs):
  borrowed original owners, fallible source preparation, exact source episodes,
  rechecking the fixed unit and one consuming claim callback.
- [`mapping_storage.rs`](../../../crates/catten/src/device/mapping_storage.rs)
  and [`detached_domain.rs`](../../../crates/catten/src/device/detached_domain.rs):
  pending pins, admitted mapping/quarantine records, actual domain/command
  ownership and inert fallback.
- The [SMMUv3](../../../crates/catten/src/device/smmu.rs),
  [VT-d](../../../crates/catten/src/device/vt_d.rs) and
  [AMD-Vi](../../../crates/catten/src/device/amd_vi.rs) map/unmap adapters.
  Their backend-specific hardware acknowledgement contracts remain assumptions.
- [`owned/memory.rs`](../../../crates/catten-rt/src/owned/memory.rs):
  `DmaUnmap`, `DmaTransfer`, `SharedDmaMemory` and `FailedSharedDmaMemory`.

The [action map](../../tla/CONFORMANCE.md#dma-operation-and-readiness-projection)
records correspondence and omissions. Existing concrete evidence remains in
the [unmap phase report](2026-10-12-dma-unmap-phases.md),
[map-failure owner report](2026-10-12-dma-map-failure-ownership.md) and
[map-readiness report](2026-10-14-dma-map-readiness.md). Those historical tests
were read for comparison. The new remap regression
records its own failing and passing native execution separately.

## Modeled contract

The model retains separate public root/claim, pending pin, wait storage and
interest fields. The fixed unit has an actual engine and recorded domain cell;
a present engine with a detached cell cannot admit work. The external holder
can restore before registration, or compete again before a closed-source wake
is processed. An abandoned holder stays fenced. Busy waiting admits only a
qualified scheduled caller; a ready unit may be claimed immediately by the
other modeled caller contexts. No action enables interrupts.

Map work runs once after claim. Ordinary backend return restores its complete
registered unit before public completion. A confirmed rollback can release its
pin; uncertainty retains the pin and original backing charge. Ordinary public
completion may release the root while that quarantined pin remains. Rejected
public completion retains its root/claim even after backend success and exposes
no usable map success. The map-submission bound is a caller protocol contract,
not a proof that Rust's returned `OwnedMemory` cannot be passed to a new API
call. Map error diagnostics provide no cleanup receipt or replay authority.

The runtime records unmap as retained before kernel entry. Only typed
pre-claim busy rejection can rearm it. A busy-looking error after maintenance
or final public completion is terminal for this attempt. Drop does not submit
an uncertain attempt again. Backend acknowledgement and successful public
confirmation are distinct, so a failure at the latter boundary is not converted
into caller-visible completion.

Shared CPU teardown can fail after DMA completion. The successful DMA state
persists, and a later finish performs CPU teardown only. A failed shared map
keeps its existing mapping behind a restricted owner. Explicit CPU unmap and
best-effort Drop can request local close without discharging an uncertain DMA
pin or original backing charge. Confirmed final release is an input to the
abstract backing-refund action. Consuming failed shared CPU teardown may return
an unmapped memory owner; a fresh mapping requires all DMA pins to be gone.
Existing coherent CPU mappings precede pinning and keep their volatile protocol.

## Finite bounds and counterexamples

The instance has one public map operation, one memory object and one fixed unit,
with three initial layouts: ready, busy, and engine-present/domain-cell-detached.
It explores shared/exclusive memory and scheduled, masked, nonscheduled or
rejecting caller contexts. Three availability epochs permit restoration and
competing claims. At most two unmap submissions allow a pre-claim busy rejection
followed by one admitted attempt. Source, pin and sponsor labels retain one
captured root generation; this is an input identity, not a new ABA proof.

Each unsafe configuration enables one weakened action alongside the safe
actions. Twelve represent regression obligations without a newly reproduced
source defect. The thirteenth, `UnsafeCoherentRemap`, records the fresh-remap
gap reproduced in the kernel before its correction.

| Unsafe action | Required violated invariant |
|---|---|
| `UnsafeClaimDetachedCell` | `ClaimNeedsCompleteUnit` |
| `UnsafeReadinessCompletes` | `ReadinessIsNotCompletion` |
| `UnsafeRejectDropsPreparation` | `PreclaimRetainsPreparation` |
| `UnsafeReplayMap` | `MapSubmittedOnce` |
| `UnsafeRearmStartedUnmap` | `StartedFailureNotRearmed` |
| `UnsafeDropResubmits` | `UncertainAttemptNotResubmitted` |
| `UnsafeForgetDmaConfirmation` | `ConfirmationSticky` |
| `UnsafeFailedSharedAccess` | `FailedSharedRestricted` |
| `UnsafeCpuUnmapDischargesDma` | `PossibleDmaRetainsBacking` |
| `UnsafeAbandonReleases` | `AbandonmentRetainsOwner` |
| `UnsafeWaitMasked` | `WaitNeedsQualifiedContext` |
| `UnsafeFinalBusyRearms` | `PublicRejectionRetainsAttempt` |
| `UnsafeCoherentRemap` | `CpuOwnerNeedsNoDma` |

## Validation

Results, action coverage, counterexample sequences and checked-input hashes
are retained in the [evidence manifest](evidence/2026-10-18-dma-operation-model.json).

The complete harness passed all **28 safe configurations** and all **64
expected counterexamples**, with every required action exercised. Safe runs
summed to **112,427,454 generated / 17,718,230 distinct states**, with no queued
states remaining. The new DMA operation instance explored **62,003 generated /
29,175 distinct states**, depth 24. The earlier DMA pinning model, now requiring
zero pins for a fresh CPU mapping, explored **17,802,315 / 2,092,882**, depth 26.
The manifest hashes all 121 checked module/configuration/harness inputs and the
reviewed source files. Separate repeats retain all thirteen DMA counterexample
action sequences; the full harness independently checked their named failures.

The remap control reaches `CpuOwnerNeedsNoDma` through `BeginMap`, `ClaimMap`,
`FinishMap`, `RejectMapPublicCompletion`, `FailedSharedCpuUnmap`, and
`UnsafeCoherentRemap`. No fresh mapping is enabled on that retained-pin path in
the safe specification.

The manual passed two final LaTeX passes in a temporary output directory and
rendered review of the changed pages. Chapter 21 has no overfull boxes; existing
warnings elsewhere remain. Its inventory matches the harness, all 592 local
Markdown destinations in changed documents resolve, and whitespace checks pass.
The user's existing chapter 6 edits and tracked PDF were preserved.

The checker is the CI-pinned TLA+ Tools v1.7.4 JAR, reporting TLC 2.19,
revision `5a47802`, SHA-256
`936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`.
Local Java is Azul OpenJDK 25.0.3; CI uses Java 17. TLC runs outside the
filesystem sandbox because worker coordination needs a local socket.

The first safe draft passed. Before the complete suite, its running map phase
was named explicitly, owner/acknowledgement invariants were strengthened and a
counterexample for rejected final public completion was added. That preliminary
complete suite passed 28 safe/63 negative configurations,
but its projection omitted fresh CPU remapping. Source review then found the
kernel's exclusive-only gate. The final model adds `FreshCpuMap` and a remap
counterexample, and tightens the older pinning model's `CpuMap` guard. Its graph
therefore differs from the preliminary result. The manifest retains preliminary
results separately with this limitation. Earlier draft input files were not
separately archived; only final checked inputs are hashed.

## Limits and remaining work

These are finite safety checks. Root/sponsor capture, qualified outer caller
contexts, genuine backend acknowledgement and confirmed physical release are
boundary assumptions. No hardware register, cache, interrupt-mask or allocator
protocol is proved. The model does not count metadata bytes, platform headroom,
per-principal budgets, multiple simultaneous public operations, grant
creation/cancellation or reset-based recovery of quarantine. It introduces no
unmap readiness, polling, custody registry or cleanup owner.

Typed frame-walk and CPU teardown failures are collapsed to outcomes; actual
partial physical release and field-destructor contexts stay separate. The
`Owned` CPU state means an unmapped local owner. A distinct `Rust` state follows
a successful fresh CPU mapping and requires no retained DMA pin.
The runtime's restricted failure wrapper remains a distinct state even when
ordinary kernel rollback happened to succeed. No liveness or complete Rust
refinement proof is claimed, and the earlier pinning, root and wait projections
have not been composed into a single proof.

The [cleanup map](../../reference/cleanup-recovery.md) records selected
C13/C14/C18 coverage; SEC-07/18 remain partial. The accompanying kernel
correction passes 517 host tests, strict Clippy on
both kernel targets and the new regression in ARM/SMMUv3 and Intel/VT-d boots.
Those direct pin controls do not inject uncertain hardware completion or execute
the whole failed-wrapper path from EL0. No multi-node or real-hardware
qualification is claimed. The next formal gaps remain
NS publication classes/death-watch admission, boot policy installation ordering,
and rejected application snapshot restoration before durable progress changes.
