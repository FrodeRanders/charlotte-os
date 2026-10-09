# Hardware quiescence and recovery

Backing becomes reusable only after the relevant physical observer has completed
invalidation. Timeout, delivery failure, logical close and a removed translation
entry are insufficient on their own. Recover only a retained owner through a
new confirmed completion; abandoned/quarantined owners have no reclamation API.

## CPU completion

x86 uses a serialized monotonic epoch and one atomic acknowledgement per LP.
The handler captures the requested epoch before flushing global/non-global
translations and paging-structure caches. Acknowledgements never regress;
late or duplicate completion cannot satisfy a newer request. Coordinator
admission yields without an owner and has a 500 ms budget; after admission,
acknowledgement has a separate 100 ms budget. No failed send is counted.

Post-bootstrap initiators must have IRQs enabled and hold no masking guard.
Only local identity capture/flush is briefly masked. An unsupported masked
context returns `InterruptsMasked`; legacy mandatory wrappers panic. Owned
mapping, loan, MMIO, namespace and final-root cleanup retain backing on failure.
Final-root invalidation tries three fresh epochs. `RetiredKernelRange` retains
its receipt for invalidation retry before physical release starts; `RetiredAddressSpace::release_retry_with` returns
its owner before any physical teardown on invalidation rejection. Once physical
release starts, a partial failure cannot be retried or refunded.

Pinned, scheduled x86 reapers perform physical stack cleanup on the original LP
with IRQs enabled, sleeping one millisecond between batches. IRQ tails stage
retirement rather than synchronously waiting on a preempted shootdown initiator.
Root-close callers must allow stack-retirement leases to finish. Arm retains
its broadcast TLBI/DSB boundary and uses the root's owned hardware tag at final
teardown; a missing live root in range invalidation now rejects.

## DMA completion

- VT-d requires advertised read/write draining and requests both at global
  IOTLB invalidation before data/table reuse. Retirement clears Context Present;
  a present link to physical zero is forbidden. Root/context secondary metadata
  precedes valid-link publication. AMD DTE publication uses this same order.
- AMD-Vi queues a strict Completion Wait with a coherent store to Unit-owned
  memory. Each submission has a nonzero checked epoch; only its exact stored
  value confirms completion. Command-head movement alone is insufficient.
- SMMUv3 protects command-ring phase/fullness after timeout. Domain destruction
  installs an aborting STE, completes configuration invalidation, then completes
  the original ASID's TLBI/SYNC before data-pin release. Abort publication does
  not first modify secondary words of the formerly live STE.

Each backend marks a retiring domain before hardware detachment. Map and unmap
reject on that domain. Destroy may retry hardware completion before physical
release starts; a partially consumed physical release is terminal. Failed completion retains its
table backing, mappings/pins and requester ownership. Queue storage and AMD's
completion cell survive timeout and late commands.

[Hardware-table admission](iommu-table-admission.md) carries charges through
this boundary. Complete physical table release is also required before removing
the admitted owner-slot/source fence. Failed map prefixes require the same data-pin
completion boundary even when table admission rejects a later page.

Explicit destruction publishes the rejecting descriptor under its registry,
then moves the complete domain and actual command engine into `Maintenance`.
The empty engine fences ordinary backend mutation/reset. Configuration/drain
waits run outside backend serialization. Hardware rejection restores the exact
engine state and retiring domain together; abandonment retains both and fences
the unit permanently. AMD producer/epoch and SMMU producer state are never
reconstructed, reset or rewound. Intel refuses a new invalidation write until an
older busy register clears.

Confirmed maintenance returns the engine before unlocked physical table release.
The original empty domain cell/nonzero requester fence persists. Physical
rejection restores the frozen owner without allocation; abandonment retains its
claim. Physical finalization uses the registered-state hold even if an unrelated
domain owns the engine. Initial domain creation/map/unmap waits remain serialized; boot unit
initialization and private cancellation now use an unlocked slot claim. See the [command-maintenance evidence](../reports/audits/2026-10-09-security-iommu-command-maintenance.md).

Before a reachable descriptor is published, each backend records its admitted
domain in `PreparedDmaDomain`'s borrowed `DmaCreation`, alongside the capability
reservation and exact user-root operation. Creation error retains that obligation;
it only marks retiring and publishes abort under the backend guard. Published
creation or capability-publication rejection invokes one-shot explicit destroy
after the grant's local lifecycle/device/config guards leave. Confirmed destruction precedes
reservation refund/root completion. Rejected cleanup or abandonment retains all
three without destructor cleanup; no retry/custody interface exists for this
publication owner. Reusing an armed creation owner rejects before initialization,
allocation or reset. Never hardware-published constructor errors now retain
complete typed private payloads in that same grant; private physical/metadata
cancellation leaves local guards before release, refund and root completion.
Physical rejection/abandonment preserves the whole grant and cannot retry freed
addresses. This needs no hardware maintenance receipt. Initial creation/reset
waits, domain allocation/preparation and reset fallback remain separate
serialized boundaries. See the
[grant rollback evidence](../reports/audits/2026-10-09-security-dma-grant-rollback.md)
and [published-creation follow-up](../reports/audits/2026-10-09-security-dma-creation-rollback.md).

Boot unit initialization now claims the existing typed slot before unlocked
allocation, preparation, control and waits. Ordinary backend lookup never lazily
initializes hardware. Private preparation rejection cancels the whole payload
before clearing its claim. Once a hardware control write starts, any error
retains the complete unit and claim, including disable uncertainty before new
backing publication. Partial release and abandonment also stay fenced; no
hardware initialization replay, shutdown or custody controller is supplied.
The boot caller's outer IRQ policy remains unchanged. See the
[unit initialization evidence](../reports/audits/2026-10-09-security-iommu-unit-initialization.md).

## Requester reset and reassignment

Successful domain destruction leaves a zero-valued requester tombstone in its
existing registry entry. It blocks a new domain until a kernel-controlled
reset completes. A source is not reset merely because a DMA capability closed.
Failed creation rollback retains this same fence after hardware detachment.

The implemented adapter supports QEMU NVMe (`1b36:0010`, class `01:08:02`) on
one segment zero. It bounds the controller BAR to 16 KiB and other memory BARs
to 8 KiB, verifying sizes through restored BAR probes with memory decode and
bus mastering disabled. Unsupported models/topologies/BAR layouts reject.
Old overlapping MMIO capability authority rejects reset; only the recipient's
unmapped, unclaimed launch grants may overlap. Lifecycle and device serialization
cover this check, and the exact endpoint's config guard survives reset and
new-domain creation.

The adapter clears `CC.EN`, waits at most 100 ms for `CSTS.RDY=0`, and retains
disabled bus mastering on failure. Only successful new-domain creation consumes
the reset owner and enables memory decode/bus mastering. Supported NVMe also
resets at its first grant, before new translations can expose old firmware queue
state. Other fresh devices retain their driver-specific initialization policy;
their later reassignment remains fenced without a supported reset adapter.

Uncertain public MMIO cleanup retains its in-flight claim, exact root lease and
mapping/scratch record. Explicit close detaches capability authority before
unlocking but keeps a claimed descriptor visible to reset throughout invalidation;
failure retains that descriptor and root without allocating a reinsertion.
Reset cannot treat an unconfirmed invalidation as removed register authority.

## Evidence and limits

Host epoch/command tests and the QEMU Intel VT-d, AMD-Vi and Arm SMMUv3 guests
exercise these paths. The reset fixture enables a real NVMe controller with DMA
admin queues, rejects one teardown completion, verifies pin retention and denied
map/reassignment, performs real teardown retry, removes old MMIO, then confirms
reset before a new domain. Normal NVMe/object-store/Raft tests run afterward.
The CPU fixture omits an actual remote IPI and separately its acknowledgement,
then retries against all real LPs without fake acknowledgements.

This is QEMU evidence, not a physical-device or worst-case latency proof. The
NVMe fixture submits no outstanding I/O command. Generic PCI function/bus reset,
VirtIO/AHCI/NIC reset adapters, stalled physical CPUs, interrupt-remapping/ATS
coverage, controller recovery for abandoned owners and complete kernel metadata/table
admission remain open. See the
[audit record](../reports/audits/2026-10-06-security-staged-quiescence.md).

Private-construction evidence: [typed grant rollback](../reports/audits/2026-10-09-security-dma-private-rollback.md).
It adds no reset target, hardware acknowledgement, queue reconstruction or
published-domain recovery mechanism.
