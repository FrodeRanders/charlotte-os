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
its receipt for explicit retry; `RetiredAddressSpace::release_retry_with` returns
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
reject on that domain, while destroy may retry. Failed completion retains its
table backing, mappings/pins and requester ownership. Queue storage and AMD's
completion cell survive timeout and late commands.

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
coverage, controller recovery for abandoned owners and complete metadata/table
admission remain open. See the
[audit record](../reports/audits/2026-10-06-security-staged-quiescence.md).
