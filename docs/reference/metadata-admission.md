# Shared metadata admission

[Resource policy](resource-policy.md) selects numerical limits. The kernel's
[`klib::admission`](../../crates/catten/src/klib/admission.rs) supplies four
family-typed owners: byte/count node charges, fixed-bundle charges, multidimensional
reservations, and observer actual-layout charges. A single
[catalog](../../crates/catten-syscall/src/metadata_pools.rs) names all 24 pools,
their dimensions and units. Every pool keeps its own static counter lock; the
pool inventory adds no allocated registry, common pool-counter lock or centralized
resource owner. Separate principal funding uses the existing history lock.

Family code still captures trusted sponsorship, chooses the actual allocation
layout, translates errors and retains the charge inside its complete operation.
It decides when completion permits deallocation. Node deallocation precedes
refund, detached or quarantined owners retain their original charges, and root
retirement or generation reuse cannot reclassify them. The shared layer cannot
close resources or prove CPU, thread or device completion. Principal-sponsored
thread/context/table owners release their node reservation before their original
principal charge; each counter guard leaves before the next field drops.
Observer allocation rollback refunds layout bytes while retaining its holder
bundle until the holder is actually freed. Simple node, fixed-bundle,
two-dimensional reservation and actual-layout charge sizes remain respectively
16, 1, 24 and 16 bytes on supported 64-bit targets. Existing logical resource limits
remain unchanged; the expanded IPC payload pool has its own byte/count policy.

`ByteAllocator<F>` adds allocation-lifetime byte admission to standard vectors
without another owner or registry. Its immutable captured class follows allocator
clones and moves; cloning reserves nothing. Allocation reserves requested bytes
and zero node slots before `Global`, and actual deallocation precedes refund.
Default growth admits the full candidate while retaining the old allocation's
charge. Pop/clear never release capacity admission. The containing operation
still decides when element cleanup and buffer disposal are safe; the allocator
supplies no physical-completion or recovery proof.

`SponsoredAllocator<S>` supplies the same requested-layout byte funding without
another family counter when a separate typed owner already bounds allocations.
`ByteAllocator` composes its family/original-principal policy through that adapter.
`ChargedAllocator<C, P, A>` can select it for both actual Arc allocations: account
payload and private charge holder, including headers and alignment. The default
backing allocator is still `Global`; unmigrated owners retain their representation
and policy. Family slots survive until both allocations are gone; principal bytes
refund per allocation only after actual free. This qualifies allocation mechanics,
not arbitrary active parent destruction or hardware completion.

The existing `snapshot_buffers` pool covers both frame catalogs and bounded
[primitive syscall inputs/plaintext](snapshot-buffer-admission.md#primitive-syscall-inputs).
Primitive vectors contribute requested bytes and zero frame-buffer counts.
The reviewed default is 12/16 MiB ordinary/total; count limits stay 6144/8192.
This temporary-storage extension shares its existing pool. Permanent store
images have separate admission; they cannot consume that temporary allowance.

`ServiceImages` adds a separate purpose: immutable kernel-lifetime service-image
backing and reserved cache entries. Fixed cache publication precedes I/O; copied
bytes use the same `ByteAllocator` and checked memory-object boundary. Temporary
seed verification copies contribute bytes and zero entries. See
[launch/cache ownership](service-launch-ownership.md). This appends the 24th pool;
existing IDs/record widths stay stable, and version 10 rejects rather than
omitting the new entry. Both architectures' test/minimal services use the updated
SDK and signed bundles.

## Principal coverage and remaining reconciliation

The catalog is the finite Q4 pool checklist below. Family admission and original
ordinary/platform classification do not establish an aggregate principal byte
limit. “Pending” means principal coverage has not been qualified here; it does
not discard the family's existing admission or cleanup tests. Other logical or
physical limits may supply a derived bound, but that proof and its contribution
to the common heap plan must be recorded before accepting an exclusion.

| Pool | Qualified principal backing scope | Remaining Q4 boundary |
| --- | --- | --- |
| `root_metadata` | Actual policy/usage/authority and generation-account node layouts | Allocator excess and joint heap fit; exact retired-ledger and root custody lifetimes remain required. |
| `namespace_nodes` | All five actual global namespace-node layouts | Allocator excess, nested resources and joint heap fit; captured principal persists through detachment. |
| `authority_nodes` | Actual node layouts, captured from the recipient authority namespace | Allocator excess, enclosing metadata and joint heap fit; logical grantor remains separate. |
| `mailbox_nodes` | Actual endpoint-node layouts from the captured root principal | Allocator excess, word-ring storage and joint heap fit. |
| `deferred_nodes` | Pending | Original accepted-operation sponsor through producer-LP disposal. |
| `observer_backing` | Pending | Actual layout, private holder and final weak reference. |
| `completion_nodes` | Pending | Payloads, CQ backing and detached backlog; distinguish logical limits. |
| `device_nodes` | Actual namespace and MMIO/IRQ/DMA payload-node layouts from captured grant principal | Backend/IOMMU storage, allocator excess and joint heap fit; hardware completion remains separate. |
| `iommu_nodes` | Pending | Domain/unit nodes, quarantine, cache and ledger capacities. |
| `principal_history` | Pending | Boot-lifetime records and duplicate candidates, including initial-record admission. |
| `thread_metadata` | Context/retirement requested bytes and bundles | Allocator excess/control overhead and joint heap fit. |
| `thread_table` | Retained/candidate table requested bytes and slots | Allocator excess and joint heap fit. |
| `root_table` | Retained/candidate ASID-table requested bytes and slots | Allocator excess and joint heap fit. |
| `waiter_accounts` | Actual waiter-account Arc and private-holder layouts | Allocator excess, enclosing references and joint heap fit. |
| `timer_accounts` | Actual scheduler/completion timer-account Arc and holder layouts | Allocator excess, enclosing references and joint heap fit. |
| `completion_accounts` | Actual record/CQ/watch account Arc and holder layouts | Allocator excess, enclosing references and joint heap fit. |
| `namespace_accounts` | Actual capability/IPC/mailbox Arc and holder layouts | Allocator excess and joint heap fit; external reference fields stay with parent pools and other account families remain separate. |
| `scratch_nodes` | Pending | Extent and namespace backing through retained mapping work. |
| `snapshot_buffers` | All production requested-byte paths | Counts have no principal share guarantee; overhead/joint heap fit. |
| `mapping_nodes` | Pending | Original mapping sponsor through detach and physical uncertainty. |
| `memory_capability_nodes` | Root heap/image catalog bytes | Object catalogs/records, namespace/capability/borrower nodes and overhead. |
| `observer_lists` | Pending | List/control allocation and final weak-only backing. |
| `ipc_nodes` | Descriptor, staging, loan, revocation and attachment-catalog bytes | Payload nodes, endpoint queue backing and overhead; namespace accounts tracked separately. |
| `service_images` | Pending | Immutable boot cache sponsorship versus requesting application work. |

The [backing-principal reference](principal-backing-admission.md) identifies
existing page/stack and requested-byte ceilings; the
[capacity baseline](../reports/audits/2026-11-05-q4-capacity-baseline.md) supplies
independent pool and mapped-heap envelopes. This table tracks coverage, not
weighted readiness. A completed pool row still cannot establish simultaneous
heap fit, bounded IRQ latency, cleanup capacity or service fairness.

## Read-only runtime inventory

`THREAD_STATISTICS` payload **version 11** has a 22-word little-endian header.
Thread and domain records retain their layouts. Header words 20 and 21 hold the
metadata record size in bytes (88) and count. After both existing sections,
validated `SystemObserver` snapshots include the complete catalog, in catalog
order. Self-only snapshots include zero pool records. An invalid nonzero
observer capability rejects the request. The syscall register ABI remains
unchanged; services and both architecture bundles must use the current payload.

Each metadata record contains eleven `u64` words:

| Words | Meaning |
| --- | --- |
| 0–2 | Pool identity, dimension count, state (`0` sampled, `1` busy) |
| 3–4 | Total limits |
| 5–6 | Ordinary limits |
| 7–8 | Total usage |
| 9–10 | Ordinary usage |

Unused dimensions are zero. Busy records retain limits but have no usage sample;
their zero usage words are padding, not a claim that the pool is empty. The
borrowed SDK parser rejects other versions, wrong identities/order/dimensions,
invalid states/limits/usage, partial catalogs, overflow and inexact lengths.
Pass the declared payload length, excluding memory-object page padding. Catalog
identity/order or record changes must update the current payload version and all
writers, readers and fixtures together.

Observation prepares its output before sampling and uses `try_lock` once per
pool. A sample is coherent within that pool; pools are **not** sampled atomically
together. Every guard leaves before encoding or callbacks. No allocation, log,
waiting, retry loop, IRQ enable or cleanup operation is introduced beneath a
pool guard. Contention reports busy rather than stalling the observer.

The existing observe service forwards this owned-memory snapshot. HTTP status
JSON exposes `threads.metadata_pools`, with each pool's name, sampled/busy state
and dimensions: unit, total/ordinary limit and total/ordinary usage. Busy usage
is JSON `null`. Authorization remains the existing observation/status policy;
there is no mutable budget or teardown endpoint. History and cluster formats
retain their own existing fields.

The `ipc_nodes` row admits capability, endpoint, pending-call and reply-token layouts;
descriptor byte snapshots, parsed entries, cap-ID/staged-transfer/token-loan
vectors, reply/cancellation revocation work and attachment retirement catalogs
contribute bytes and zero node slots to that same pool, retaining original
requester class through actual buffer release.
Logical IPC resource counts retain their separate owners. Earlier catalog IDs
and record widths stay unchanged; version 9 rejects rather than silently omitting
the new pool. Catalog/version changes require both architecture service bundles
to be rebuilt and signed; expanding an existing pool's layout coverage changes
no SDK or wire format.

Byte counts are requested allocation layouts, not physical RAM or allocator
bookkeeping. IOMMU metadata bytes include allocated ledger capacity and full
replacement overlap; those buffers contribute zero to its node count. Object
frame catalogs similarly contribute bytes and zero nodes to
`memory_capability_nodes`, retaining original backing class through actual free.
Bundle-only pools do not report bytes. Pools may describe nested
allocations and must not be summed into a machine memory total. Limits reserve
headroom in admission, not preallocated memory. This inventory supports sizing;
it does not close R7's remaining allocation, sponsor, physical heap or concurrent
pressure gaps.

## Qualification

The host suite compiles the production common owners, policy, SDK parser and
observer handles. It exercises ordinary/total rejection without mutation,
original-class refunds, allocator rejection, layout rollback, bundle headroom,
concurrent release, busy observation, callback after guard release, and malformed
wire records. The authenticated HTTP liveness probe validates the complete
catalog, units, limits and sampled/busy usage in real status responses. Existing family fixtures retain their real allocation, detachment,
weak retention, cancellation and deallocation-before-refund checks. Native
telemetry fixtures validate self-only, authorized and forged observer requests,
exact output sizing and heap-held nonblocking inventory capture. Owner-size
checks run natively too. Host mutexes do not qualify IRQ behavior.

The initial [shared-counter evidence](../reports/audits/evidence/2026-11-02-metadata-admission.json)
records source/kernel/service identities and relevant diagnostics. Fresh four-CPU
240-second security campaigns passed ARM 19/19, Intel VT-d 17/17 and AMD-Vi
18/18, with zero failures or pending tests. AMD also passed authenticated HTTP
inventory and management TLS probes. The full host suite, strict kernel checks and native test
builds passed; both architectures' test/minimal service bundles were
rebuilt and signed. The earlier ARM preflight rejection is recorded separately.
These runs do not close broader allocation, concurrency or latency gaps.

The earlier [IPC node qualification](ipc-node-admission.md) records its
23-pool/version-10 scope. Current [launch/cache qualification](service-launch-ownership.md)
covers the 24th pool/version-11 payload and rebuilt/signed test/minimal bundles.
