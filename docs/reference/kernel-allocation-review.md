# Kernel allocation producer review

This is Q3's current producer/factory assignment, reviewed 2026-11-05. The
[closure plan](security-closure-plan.md) records package acceptance. This review
does not supply aggregate/original-principal admission (Q4), the complete
caller/IRQ/destructor inventory (Q5), recovery (Q6), progress (Q7) or combined
qualification (Q8).

The [version-controlled review](../reports/audits/evidence/2026-11-05-q3-producer-review.json)
reconciles 738 non-drop records from both current headless production profiles:
144 allocator/growth/copy/factory operations, 505 owned-result operations and
89 empty constructors/reference clones. It includes all source/body origins,
614 kernel source identities and named dependency/profile supplements. Grouping
expresses a manual review disposition, not an automatic allocation/reachability
proof. These are review groups, not new pools or cleanup owner families.
The 2028 explicit/implicit Drop candidates remain Q5 work.

An owned return can borrow, rewrap or transfer existing storage. A collection
push can use previously reserved capacity. Preserve accepted Q1/Q2 and existing
admission owners instead of adding a second charge to their spelling. Conversely,
a per-request count/byte limit alone cannot establish concurrent node/principal
coverage. The required Q4 rows below remain actual unfinished work.

## Current assignments

| Review group | Disposition | Trigger, bound and owner/lifetime |
| --- | --- | --- |
| `boot_memory` (1 records) | Bounded boot exclusion | Mandatory BSP validation allocates one aligned 8192-byte scratch layout and tests one private frame; no runtime entry. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `boot_lp` (63 records) | Bounded boot exclusion | Validated 1..256 LP factories, schedulers, AP stacks/tables, APIC-ID map and shootdown acknowledgements; boot lifetime, no hotplug. PerLp/boxed_slice concrete callers are fixed boot storage or inactive helpers. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `boot_timer` (16 records) | Bounded boot exclusion | BSP preallocates queues and one timer Arc per validated LP; owning LP initializes hardware once before the barrier. Runtime get cannot construct storage. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `boot_firmware` (40 records) | Bounded boot exclusion | BSP ACPI catalog has at most 8187 XSDT addresses; IORT has at most 4093 nodes and one cached immutable config. MCFG/PCI has 32 windows, 256 bus identities/window and global 16384 device/function limits. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `boot_tags` (5 records) | Bounded boot exclusion | Hardware-width ASID return storage reserves 256 or 65536 u16 slots before user tag publication. Return cannot grow; original root/invalidation rules remain authoritative. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `boot_log` (9 records) | Bounded boot exclusion | CPU vendor/model strings come from fixed 12/48-byte CPUID data; firmware diagnostic output is bounded by the catalog and emitted once by finish_boot. Unknown table signatures format borrowed bytes. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `boot_ingress` (7 records) | Bounded boot exclusion | Compiled cluster configuration and encoding reject more than protocol MAX_SERVICES=16 before collection allocation. Temporary fixed-count bindings/encoded launch bytes are used by trusted boot composition only. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `inactive` (33 records) | No current production caller | ShardLocal, ShardMailboxSet, generic closure IPI, unused VecQueue, uncharged IdTable insertion/default snapshot helpers and MADT indexing have fixture-only or no callers. InterruptRoutingManager mutation is todo and its empty/default/clone bodies have no operational caller. New production use requires review. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `deferred_stub` (1 records) | No constructible production work | DeferredTask is uninhabited and no worker/submission exists; pinned unbounded queue construction allocates no blocks. Adding a task variant or production caller requires bounded admission and progress review. See [boot-allocation-boundaries](boot-allocation-boundaries.md). |
| `fixture_profiles` (11 records) | No current production caller | Kernel S3/Kafka/Kafka-step profile-building helpers are called only by optional self-tests. Encoders validate protocol bounds; operational connector launch consumes the admitted encoded plaintext instead. See [service-launch-ownership](service-launch-ownership.md). |
| `completion` (105 records) | Existing admitted owner | Completion records/nodes/accounts, CQ ring/backlog bytes and watch/observer references have separate existing admission and complete owners. Borrow/upgrade/return operations do not create another independent allocation charge. See [completion-record-budgets](completion-record-budgets.md). |
| `ipc` (139 records) | Existing admitted owner | Namespace/payload/endpoint nodes, queue backing, staging/loan/descriptor/revocation/attachment vectors and returned work remain with captured exact-generation owners and independent byte/count reservations. Prepared capacity precedes publication/retirement. See [ipc-node-admission](ipc-node-admission.md). |
| `capability` (12 records) | Existing admitted owner | Capability/account/node publication prepares backing against the exact unpublished/captured root; original class survives transfer, retirement and weak retention. See [capability-admission](capability-admission.md). |
| `thread` (142 records) | Existing admitted owner | Thread node plus ThreadContext bytes are admitted together before allocation; stacks/root tables and waiter/timer accounts retain their own owners. Scheduler lookup/guard results borrow existing boot scheduler Boxes. See [thread-metadata-admission](thread-metadata-admission.md). |
| `tables` (16 records) | Existing admitted owner | Runtime root/thread IdTables use charged candidates for payload/generation/return-ID arrays, including old/candidate overlap. Retirement reuses prepared capacity; uncharged generic insertion has no production caller. See [root-table-admission](root-table-admission.md). |
| `iommu_ledger` (5 records) | Existing admitted owner | DMA ledger vectors reserve exact capacity through the containing table/domain owner and existing iommu_nodes backing; pushes do not grow published storage. See [iommu-table-admission](iommu-table-admission.md). |
| `observer_timer` (43 records) | Existing admitted owner | Observer entries/handles/list/account backing and anonymous timer node/cancellation ownership use existing typed fallible admission. Reference/delivery transfers retain the original allocation owner. See [observer-ownership](observer-ownership.md). |
| `shared_allocator` (14 records) | Shared admitted mechanism | ByteAllocator and ChargedAllocator are backend adapters, not new resource families. Concrete callers supply typed layout/class reservations; allocation holders retain admission through final weak/deallocation. See [metadata-admission](metadata-admission.md). |
| `memory_objects` (13 records) | Existing admitted owner | Object frame catalogs and snapshots retain their existing prepared byte reservations and captured backing classification; actual frames keep physical admission. Bare default snapshot wrapper is fixture-only. See [memory-capability-node-admission](memory-capability-node-admission.md). |
| `inputs` (14 records) | Accepted Q1 ownership | Primitive image/descriptor/pickup/plaintext buffers capture actor class and retain byte admission through deallocation; sensitive spare capacity is wiped. See [snapshot-buffer-admission](snapshot-buffer-admission.md). |
| `launch_cache` (21 records) | Accepted Q2 ownership | Complete launch/handoff/upgrade parents retain source/target roots, profiles, transfers and admitted vector prefixes. Fixed image cache reserves independent bytes/count before I/O and immutable publication. See [service-launch-ownership](service-launch-ownership.md). |
| `telemetry_q4` (17 records) | Required Q4 fix | Diagnostic snapshots/encoding use fallible prepared buffers and one 1 MiB per-request working bound. They still require aggregate and original-principal backing admission; per-request bounds are not accepted as that proof. See [telemetry-snapshots](telemetry-snapshots.md). |
| `frame_tracking_q4` (11 records) | Required Q4 fix | Each heap/image frame reserves physical admission before fallible owned_frames tracking growth; registration pushes only into prepared capacity. Count follows admitted backing, but vector capacity/old-candidate overlap still needs original-principal/aggregate kernel-byte coverage. See [page-table-lifetime](page-table-lifetime.md). |

## Dependency and optional-profile boundaries

Operational connector input/plaintext remains in the captured Q1 owners. S3's
kernel decoder borrows its bounded frame. Kafka's `validate_encoded` shares the
same parser and field validation as owned decode, using 3840 bytes of fixed list
scratch (compile-time maximum 4096) instead of heap vectors. The original wire
format, integrity/authentication rules and profile limits stay unchanged. The
userspace decoder materializes its three vectors after validation, within the
service's admitted address-space backing. Kernel S3/Kafka/Kafka-step builders
have only optional fixture callers; production consumes the admitted encoded
profile. New production calls require review.

The launch library's current descriptor/policy parsers borrow their frames;
cryptographic opening writes into caller-owned plaintext. Digest/signature and
AEAD work uses fixed storage in these paths. This does not qualify forced-abort
erasure, authenticated security time or external TLS negotiation. The pinned
concurrent-queue bounded buffer is allocated at boot construction; its push/pop
paths do not grow it. DWM has no constructible work. Locks/Once and reference
clones supply no independent backing allocation. Keep the pinned dependency
review with any future feature/producer change.

The optional C framebuffer backend retains its checked six-allocation boot plan,
complete opaque owner and inert fallback. Its write/parser path does not
allocate. The [display deadline episode](../reports/audits/2026-11-04-framebuffer-bounds.md)
remains unresolved and is not closed by a headless producer review. Device-tree
power/discovery/IOMMU integration and RISC-V remain nonoperational; HVF omits
protected PCI/DMA checks and supplies no production-isolation qualification.
Inactive MADT/VecQueue/interrupt-routing helpers are not qualified implementations
merely because no current application reaches them.

## Next aggregate work

Q4 must add aggregate/original-principal coverage to the diagnostic snapshot
working buffers and heap/image frame-tracking vector capacity, including growth
overlap and actual deallocation. It must also reconcile existing per-family
admission with signed-principal coverage and the common heap envelope, accounting
for allocator/control backing, boot reservations, weak/private holders, restart,
retention and quarantine. Existing global ordinary/trusted pool admission alone
must not be reported as completed principal accounting.

Changing a limit requires a capacity/latency tradeoff, not silent clamping of
legal input. The hardware-ASID return reserve costs at most 128 KiB once at boot
and removes allocator work from retirement. The Kafka validator's fixed list
scratch removes three kernel allocations while accepting the full 64 KiB frame
and existing 64/31/64 list ceilings; it does not reduce those protocol limits.
Native scheduled-stack controls qualify that executed boundary, not every kernel
caller's stack/IRQ context. Read [the Q3 checkpoint](../reports/audits/2026-11-05-q3-producer-review.md)
for final campaigns, failed preflights and acceptance limits.
