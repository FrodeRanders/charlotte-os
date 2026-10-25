# Security audit: DMA-pin snapshot preparation and retention

Date: 2026-10-25. Reviewed baseline: `9ded45ff`.

DMA pin snapshots now prepare fallibly outside `MEMORY_OBJECTS`, before either
pin counter changes. Final captured source/access validation precedes snapshot
fill and both shared/exclusive counter publications under one hold. Bare DMA-pin
fallback retains snapshot metadata and backing fences; explicit snapshot disposal
precedes unpin.

This advances SEC-07 R7-2 and C14/C15/C18, preserving C05/C06's independent
cleanup pins. The ledger remains **29 corrected / 6 partial / 3 open**;
SEC-07/18 remain partial and production trust disabled. The
[tracked evidence](evidence/2026-10-25-security-dma-snapshot-storage.json) records
source/model/artifact identities and selected execution diagnostics.

## Preparation and lifetime

The old DMA pin path incremented shared/exclusive counters before an infallible
frame-vector clone under memory serialization. That allocation could not return
ordinary failure. The new path first validates nonempty direction, capability
read/write rights, source loan/retirement state, copy/DMA exclusion, CPU mappings
and checked counters. An inert owner then reserves vector capacity outside the
registry, carrying no data pin, borrowed frame or copied frame identity.

Final admission rechecks the full captured namespace handle, object ID, size
and every predicate before allocation-free snapshot fill and both counter updates.
Ordinary preparation/captured-validation rejection disposes its unused capacity
after memory unlock. The memory adapter returns `FrameAllocFailed` on vector
allocation rejection; backend callers retain their existing `Error::Memory`
conversion. No public ABI or diagnostic stage/cause contract changes.

Copy and DMA now share vector preparation, capacity checks, transfer and explicit
cancellation. Their admission, pin types and completion proofs remain separate.
Bare `DmaPin` fields retain snapshot capacity and the original backing fences
without allocator/registry entry. Explicit unpin frees the actual snapshot first,
then decrements both original counters under memory serialization, then permits
existing deferred backing release after unlock. A missing pinned object now
rejects as an internal invariant, matching copy unpin, instead of silently
returning. Absence supplies no hardware completion proof.

VT-d, AMD-Vi and SMMU map entries perform this preparation before their domain/
engine claim. The public `DmaOperation` retains its existing exact root and DMA
capability; the completed pin enters its existing `PendingPin` before unit
readiness admission. Hardware descriptors, maintenance, rollback, quarantined
pins and typed backend completion remain owned by the same operations. There is
no new root lease, admission pool, registry, recovery family, readiness signal or
map/unmap replay permission.

## Verification

Serialized memory-pin controls exercise:

- **128 preparation-prefix rejections**, alternating shared/exclusive access:
  64 before and 64 after actual vector capacity preparation. Memory/heap/table/
  frame guards are available at the factory boundaries; both counters remain
  zero and a source write remains allowed. Original backing/authority/frame
  counts return. These are stage adapters, not global allocator OOM.
- Empty direction rejects before the factory. Actual vector-capacity overflow
  and per-call factory failure return `FrameAllocFailed` without a pin.
- Source close, an exclusive CPU mapping, a competing DMA pin, a copy pin before
  device-write admission and a new read loan are all rechecked after preparation.
  Rejection preserves each competing owner's existing state and publishes no
  new DMA pin. Accepted peers close through their original owners.
- Actual source-root retirement and ASID reuse in the pin-free serialized
  adapter rejects the captured generation after successor authority publication.
  The successor keeps zero DMA counts and remains writable. This fixture does
  not bypass a public operation's already retained root.
- **64 heap-held snapshot fills/dual-counter publications**, alternating shared
  and exclusive access on a 35-page source, compare the exact frame vector.
  Post-snapshot disposal observes the original counters still armed and authority
  close rejected; only then does explicit unpin restore both counts. Local guard
  availability and the original IRQ state are checked.
- Guarded bare exclusive-pin abandonment retains its snapshot, **one data page**,
  complete object metadata and original charge with both counters at one.
  This memory-only pin was never handed to hardware. Its unmapped source root
  can retire and its ASID can be reused, while backing remains charged to the old
  generation. A successor's actual pin names a different object; independent
  cleanup preserves the old counters/charge. This adds no retained root/lease,
  hardware claim or physical retry. These are selected retention counts, not
  total kernel quarantine. Shared preparation's existing copy controls also
  retain their unused snapshot under heap/memory guards.

The memory-pin controls install no IOMMU descriptor or command. Actual backend
map/rollback/unmap/destruction, unit readiness, cancellation, device/loan/peer
close and returned-capability controls remain enabled in the native suites.
Strict base-kernel Clippy passes on both architectures. Kernel-coupled preparation
is exercised directly; no alternate host implementation is introduced.

All first native executions pass, with **42 HTTP checks** and both security-service
generations on each backend. Results and observed transport diagnostics stay separate:

| Backend | Deferred tests (passed/failed/pending) | Healthy beside stalled peers | Trickle rejection | Cancellation requests |
| --- | --- | --- | --- | --- |
| Arm SMMUv3 | 20/0/0 | 0.229–0.398 s | 30.09 s | 19,368 |
| Intel VT-d | 18/0/0 | 0.212–0.312 s | 30.16 s | 17,948 |
| AMD-Vi | 18/0/0 | 0.264–0.410 s | 30.24 s | 18,304 |

The evidence includes all host retry diagnostics and selected guest backend/
ownership markers. No deadline, attempt bound or admission limit is widened.
No native failure or guest repeat occurs in this batch. These passing episodes
do not explain historical AMD startup or other recorded failures.

Arm's **first metrics attempt** records one host `TimeoutError; retry 1/2`,
followed by `PASS: initial metrics request` within its original retry bound.
Intel/AMD initial metrics pass without a preceding retry diagnostic. Each run
also records four `ConnectionResetError` retries around subsequent peer-close/
idle recovery. Surrounding host output and Arm's failed first attempt are
retained separately from guest completion and the passing repeat in the evidence.
The timeout's cause is not established; a passing retry supplies no historical
failure attribution.

Workspace formatting, whitespace and local references pass; native assembly
section checks remain enabled. Bundled services are reused; userspace sources
and the tracked manual PDF are unchanged. Historical failed executions retain
their separate records; these passes do not attribute their causes.

## Formal qualification and remaining work

All **23 focused harness cases** pass: three safe configurations and **20 expected
counterexamples**. DMA operation explores **29,175 distinct / 62,003 generated
states**, depth **24**; IPC transaction explores **26 / 71**, depth **11**;
root retirement explores **2,978 / 3,733**, depth **33**. Required safe actions
execute; negative configurations violate their named invariant through the
intended unsafe action.

No model input changes. Existing DMA backend/public claims, confirmation,
readiness, abandonment and CPU-remap exclusions, IPC loan/rollback/non-terminal
timeouts and root original-charge projections remain relevant. These models do
not implement concrete vector allocation, the pin-free source-interleaving
interval, snapshot disposal or allocator/destructor/IRQ contexts. Those are
native/source qualification, without executable refinement or sustained-pressure
fairness proof.

Per-object frame bounds limit individual snapshots. Overlapping candidates and
retained vectors still lack independent byte/count and aggregate signed-principal
admission. Source-transfer/scoped cleanup, other ordinary backing rollback and
broader caller/IRQ/destructor contexts remain open. Concurrent hardware I/O,
pressure/essential-service latency and the full recovery matrix remain separate
SEC-07/18 work. Historical mapping panics, root-close timeouts and initial probe
failures remain separately unattributed.
