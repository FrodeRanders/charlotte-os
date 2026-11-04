# Supervisor launch, handoff and service-image ownership

## Unstarted launch

`DomainLaunchTransaction` owns the loaded namespace, a still-kernel-owned
profile, the exact target-root operation and up to three captured source-root
operations. Naming/controller, old-service and state-source identities are
retained before delegation or transfer. The immortal kernel namespace needs no
root lease. Duplicate sources reuse the existing lease. A stale source rejects;
an ASID lookup cannot substitute its successor.

Profile and scoped-deployment paths keep this same owner through initial-thread
publication. `PreparingObserver` contains it too; ordinary grant rejection calls
explicit cancellation after helper guards leave. Successful thread publication
hands target lifetime to its stack/root owner before completing preparation
leases. `PreparedDriver` keeps its independent boot-composition lease through
that handoff.

`cancel` runs only in the qualified ordinary context, outside unrelated
lifecycle, IPC, registry, heap and physical-allocator guards. It closes a
kernel-side profile once, completes its own target operation, closes the
unstarted target once, then completes source operations. Connections and profile
authority already delegated to the target belong to that namespace. A failure
stops immediately and retains all remaining fields/charges; there is no
best-effort second pass or scalar reconstruction. Busy is not confirmed cleanup.
`Drop` is inert terminal retention, including implicit destruction inside
observer/handoff parents. This is not a recoverable owner or retry API.

The earlier loader's `PreparingDomain` failures occur before this owner exists.
Their active cleanup and every broader enclosing caller/IRQ/destructor context
remain part of Q5. These local contracts do not qualify all loader or scheduler
rollback paths, forced kernel-operation termination, or typed recovery.

## Upgrade handoff

The live upgrade syscall uses `try_spawn_upgrade_from` and the same complete
`PreparingUpgrade` as the multi-state mechanism. The IPC lookup captures the old
endpoint's root generation under IPC; it does not return an ASID for later
classification. The replacement inherits that captured old service's trusted
class before loader backing/translation admission. An ordinary service cannot
promote itself by replacement. State authority comes from the separately
captured authorized manager/source namespace.

`UpgradeGrant` borrows its source-cap ID slice. Both prepared-transfer and target
ID vectors use `ByteAllocator<IpcNodes>`, with original source classification,
requested bytes and zero node slots. Reserve both complete arrays before any
source escrow/delegation. Keep every transfer, both arrays and the launch owner
in one retaining parent. On ordinary failure, explicitly dispose transfers and
actual vector backing before cancelling the target/root owner. Commit is atomic;
write only the resulting destination IDs into the bootstrap page. After commit,
ending transfer staging is not reverse movement or permission to replay.

The launch capability vector has 32 entries including bootstrap authority.
At most 31 state entries fit without an endpoint, or 30 with one. On both
supported targets the 328-byte transfer plus 8-byte ID requires at most 10416
array bytes. The existing 12/16 MiB IPC byte limits suffice; no limit increase or
single-state special implementation is needed. Empty handoff storage allocates
nothing. For authorized live-service targets, the public register ABI, source-image
consumption, failure result and
replacement's local registration yield remain unchanged. Connections without
a captured service-root identity reject before preparation.

## Store-sourced images

The existing cache is a fixed table with 64 slots by default, bounded through
`service_images`: 16 MiB/64 total, 12 MiB/48 ordinary. All current producers are
kernel bootstrap/lookup code with compiled static artifact names, so they use
the immortal kernel namespace's trusted class. Artifact roles or user-supplied
names cannot select it. The ordinary dimension remains part of the shared pool
contract; it is not a per-application cache allowance.

Reserve a cache count and fixed publication slot before store I/O. No cache
mutex spans IPC, allocation or signature verification. Image copies use the
same checked memory-object snapshot boundary and retain
`ByteAllocator<ServiceImages>` through private preparation and publication.
Seeding's temporary verification copies contribute bytes and zero cache counts.
Reject failures fallibly. Cancel a private copy outside guards, free its actual
buffer, clear the preparing slot, then refund its count. Abandonment retains the
complete private prefix/count and the preparing fence.

Verify the signed artifact name before allocation-free publication. Published
slots own the vectors and admission for the kernel lifetime; they are immutable
and never evicted or replaced. Returned slices borrow the cache, not its mutex
guard. The production cache is static, so its slices last the boot. Its unsafe
slice construction relies on this no-removal/no-mutation contract. Do not add
eviction without changing the borrow API and ownership proof. There is no
leaked plain vector or growable publication table.

This separate pool prevents permanent cached images from consuming Q1's
temporary snapshot allowance. Both reuse shared admission/allocator mechanics;
physical completion and publication stay local. It permits at most 16 MiB
image retention alongside the 16 MiB snapshot ceiling within a minimum configured
72 MiB heap envelope. The fixed table is kernel storage, not a heap-node charge.
Allocator overhead, all-family/principal capacity and workload progress remain
Q4/Q7/Q8. No heap reserve or protocol maximum is changed.

Store repair requires an explicit not-found response or a completely returned
empty/oversized/invalidly signed image. Quota, snapshot, I/O, IPC and malformed
attachment failures stop seeding; they cannot authorize replacing a possibly
newer installed artifact. Persistent object formats and signatures are unchanged.
The existing single-flight cache gate still yields while another reader owns
it; it has no producer notification or abandoned-reader recovery contract.
The existing object-store lookup/IPC-cap helpers retain their ordinary caller
cleanup policy. Broader waiting, teardown and termination qualification remains
Q5/Q7; this batch adds no saturated-progress guarantee.

The 24-pool catalog is statistics payload version 11. Header and record widths,
syscall registers and runtime ABI 3 remain unchanged. Retired payload versions
reject. Kernel, SDK, observe, HTTP and signed test/minimal bundles on both
architectures must agree on this catalog.

[Q2 qualification](../reports/audits/2026-11-04-supervisor-launch-cache.md) records
the combined host/strict/native evidence and exact source variants. Its local
acceptance does not close SEC-07/18 or the Q3–Q8 aggregate gates.
