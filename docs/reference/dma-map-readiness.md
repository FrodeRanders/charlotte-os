# DMA-map unit readiness

DMA map/map-exclusive syscalls share the fixed IOMMU unit's existing owned
availability subscription with grant preparation and consuming grant
cancellation. This adds readiness **before** mapping work, within one syscall;
it adds no public map retry or compatibility fallback.

## Complete preparation and one claim

`DmaOperation` retains the exact address-space generation and public DMA
capability claim. Before backend access it captures ordinary/platform policy
from that retained root. The existing `PendingPin` moves into this operation
before readiness admission; the original memory object cannot close or change
ownership while that pin is held. Source/disposal storage is fallibly prepared
outside lifecycle/device/backend guards, with the captured classification.

The common admission helper checks the same fixed slot, links or borrows its
existing source, rechecks after preparation and parks on the independent
subscription. Producer restoration closes the source through deferred delivery
after its local guards; no callbacks or source disposal run under backend
serialization. An available command engine alone is insufficient: all domain
cells must also be present. A permanently abandoned claim remains fenced and
does not generate readiness.

After wake, admission rechecks the same slot and takes the recorded nonretiring
domain, actual command engine and original pending pin together under one
backend hold. It returns `MappingMaintenance` before that guard leaves. The
claim callback runs once; no walker, descriptor publication, cache maintenance
or physical cleanup is replayed. Existing backend-specific mapping and typed
completion/rollback retain their original policy.

## Caller context and rejection

Waiting requires an already interrupt-enabled scheduler-owned caller. No map
adapter enables IRQs. x86 userspace SYSCALL dispatch enters this qualified
context after saving its trusted register/return frame; ARM's currently masked
SVC dispatch can still take the immediate ready path, but busy wait admission
rejects explicitly. The interrupt-enabled kernel fixture separately qualifies
scheduled admission on both backend builds. This does not qualify waiting
beneath an arbitrary kernel caller's outer guards.

Kernel `device::dma_map` and direct backend fixture adapters retain rejecting
admission. Public syscall dispatch explicitly uses `dma_map_scheduled` for
both ordinary and exclusive mapping; the former exclusive-only wrapper is
removed. Register layouts, syscall numbers and runtime ABI 3 are unchanged.

Source preparation, waiter rejection or other failure before the actual
domain/engine claim releases the original unstarted pin outside backend guards.
The public operation then explicitly disposes its transient interest outside
the device guard and finishes its exact root last. A completion that still
owns pending preparation cannot release the public claim. Abandonment retains
the root, claim, pin, interest and original charges without implicit cleanup.

Per-call status/phase/cause remain diagnostic. Backend map errors include both
admission and started maintenance and authorize no resubmission. A failed
userspace shared-DMA preparation retains its existing restricted CPU-mapping
owner. Hardware uncertainty, pin quarantine, unmap's typed admission versus
completion distinction and confirmed physical release remain unchanged.
Readiness is neither DMA completion nor authority to destroy or reuse backing.

## Evidence and limits

Private-slot controls use the real public operation and two-page memory pin
through source/wait rejection, masked rejection, producer-before-registration
readiness, detached-cell fencing, one claim, ordinary cleanup and retaining
pre-claim Drop under guards. A scheduled fixture checks actual blocked thread
state, exact-generation exit observation and one wake/claim. Its bounded
one-millisecond registration observation is fixture-only; the tested map wait
parks on its source.

A pre-driver real-unit probe retains the complete actual unit through the
existing preparation owner, with its caller retaining the existing real domain
root. Workers enter ordinary and exclusive scheduled mapping, register and
block with their original public claim/pin, then resume after exact unit
restoration. Actual backend unmap completion precedes memory release. The
probe allocates no duplicate domain or engine, mutates no descriptor and uses
no invented hardware acknowledgement. It separately qualifies real VT-d and
SMMUv3 admission; it does not identify the historical contending owner.

Native linked execution and artifact/failure details are recorded in
[the follow-up audit](../reports/audits/2026-10-14-dma-map-readiness.md).
AMD-Vi is compile-qualified unless separately executed. Masked SVC contention,
unmap/close readiness, wider metadata/guard contexts, indefinite abandonment
and historical causal attribution remain separate work. The existing owner
families are C13/C14/C18 in [cleanup/recovery](cleanup-recovery.md); no registry,
new custody family or physical retry owner is introduced.
