# IOMMU table backing admission

Intel VT-d, AMD-Vi and Arm SMMUv3 use `device::dma_tables::Tables` to own
physical table/queue backing and its admission charge. Admission precedes
physical allocation and hardware publication. The account follows the exact
hardware domain or unit owner; it is never looked up through a reusable ASID,
capability, requester or domain number.

| Scope | Ceiling |
| --- | --- |
| One DMA domain | 1,024 actual 4 KiB pages (4 MiB), including its root and SMMU context descriptor. |
| One remapping unit | 2,048 pages (8 MiB), including shared tables, queues and completion storage. |
| Node total | One thirty-second of usable RAM, rounded down to pages, minimum one. |
| All DMA domains | Three quarters of the node total; the final quarter is shared-unit headroom. |

This is initial kernel policy. Every domain, including platform-driver domains,
uses the domain subpool. Shared-unit scope is selected by backend initialization,
never an application flag. The unit reserve is not a platform-domain privilege.
Domain admission is per hardware requester lifetime, not a signed userspace
entitlement or aggregate per-launch sponsorship policy. Physical allocation can
still fail inside these limits; independent admission pools do not comprise a
complete RAM ledger or guarantee progress under physical exhaustion.

VT-d counts its root and cached per-bus context tables. AMD counts its contiguous
device table, command/event pages and completion cell. SMMUv3 counts its contiguous
stream table and command/event queues. Firmware/boot-inherited structures are
outside the pool. Published shared structures remain charged for unit lifetime;
there is no controller shutdown/reclamation API.

## Publication and rollback

A provisional region holds an exclusive account borrow, the reserved count and
physical backing through zeroing and transfer into the fallibly prepared ledger.
The full contiguous extent is admitted before allocation. No allocation is
required between recording physical ownership and publishing a table link.
Children are initialized before valid-link publication. Ledger metadata and
physical allocator rejection explicitly cancel only the unused region admission.
`Tables::prepare_unpublished` rolls back known-private construction prefixes;
`cancel_unpublished` consumes a never hardware-published table owner. All three
backend constructors use these boundaries, including VT-d MSI/context rejection
and SMMUv3 root/CD/MSI preparation. Unused reservations and confirmed private
release refund exactly once.

The backend marks domain/unit tables published before the first hardware-visible
context, DTE, STE or base-register write. Publication also rejects uncertain
preparation. Every table/region fallback retains backing and its original charge,
including unpublished and reservation-only abandonment. Region fallback marks
its exclusively borrowed parent uncertain; later allocation/refund rejects. Table
fallback retains the ledger allocation as well, avoiding implicit `Vec` heap
deallocation. Neither fallback enters an allocator, pool, guard, callback or
logger. Ordinary successful release disposes that ledger explicitly. Failed creation retains its registered retiring owner and
requester fence unless hardware detachment, maintenance/drain and complete
physical release all succeed. SMMUv3 creation rollback also completes TLBI/SYNC
for the original ASID after installing its aborting STE.

Partial sparse walks keep their linked intermediate tables charged. Unmap removes
leaves; empty branches remain cached and reusable at the ceiling until domain
retirement. A failed branch returns a mapping error rather than bypassing quota.

A failed multi-page map clears only its installed data-leaf prefix. If any data
leaf was installed, the backend must confirm existing IOTLB drain/ASID maintenance
before releasing the data pin. Rejected completion retains the pin in the exact
domain; duplicate maps of that object reject. Quarantine capacity is prepared
fallibly before any data leaf publication. A later confirmed domain retirement
releases both ordinary mappings and retained pins outside backend serialization.
This also covers table-admission failure in the middle of a buffer.

## Physical completion

Domain retirement first marks the owner retiring, detaches hardware authority and
confirms the backend's existing configuration/TLB maintenance and transaction
completion. Only then does `Tables::release` consume the physical release walk.
The registered domain and source fence remain until every table frame releases
successfully; pins and capability cleanup cannot treat rejected release as a
successful close.

Release enters a terminal frozen state before touching the allocator. Partial
failure or interruption retains the whole charge, including capacity for frames
already freed; retry never walks freed entries. Unknown provisional handoff also
retains admission and backing. Repeating successful release is harmless, but
frozen release always rejects. A successor cannot refund that original charge.
Hardware timeout before physical release may still be retried against the same
owner, as described in [hardware quiescence](hardware-quiescence.md).

Normal destruction moves the owning domain out of its registry and consumes its
mapping/pin collections directly. It does not allocate a teardown snapshot.
Physical allocation, ordinary private cancellation, ledger/registry preparation
and published hardware maintenance/table release still occur under backend
serialization. Pure fallback retention does not qualify those ordinary outer
masks or move hardware waits outside the backend guard. General kernel metadata and callback admission, moving all
allocation out of masking guards, and abandoned-owner recovery remain separate
work. No quota override, partial-release retry or administrative force-clear is
introduced.

## Evidence and limits

Serialized boot fixtures check node/subpool accounting, rejection before an
allocator callback, physical allocation failure, contiguous zeroed backing,
private rollback, cached-table reuse, retained sparse prefixes and successful
retry. Each active architecture walker runs against a private unpublished root.
A published abandonment and an injected partial physical-release failure retain
five charged pages and three actual frames for the guest lifetime. Rejection
and abandonment are injected owner states; no actual panic unwinding or physical
allocator corruption is performed.

Additional probes cover domain and unit scope. Metadata rejection never reaches
physical allocation, allocator rejection refunds unused admission, reservation-
only/allocated-region cancellation restores counts, and ordinary constructor
prefix rejection at one, two and three frames refunds exactly once. Successor
owners cannot refund earlier retention.

Guarded abandonment holds backend registries, lifecycle, both CPU tables, heap
and physical allocators and the original pool. Unpublished table Drop and rejected
published cancellation retain their actual ledger allocations. Region probes
cover reservation-only, allocated, armed transfer and partial physical rejection;
uncertain parents reject allocation and physical retry. These retain fourteen
additional original charges (seven domain) and ten frames. Alongside the original
release fixtures, the injected retention probes account for nineteen charges and
thirteen frames, separate from live hardware-unit/domain backing. Simulated
interruption is not actual panic unwinding or a hardware timeout. See the
[preparation follow-up](../reports/audits/2026-10-09-security-iommu-preparation-abandonment.md).

The QEMU NVMe recovery fixture checks unchanged charges after rejected hardware
completion, refund after real retirement, capability refund under domain pressure,
and both successful and rejected cleanup of a two-page map crossing a cached
leaf-table boundary. Memory close rejects while the failed cleanup pin survives;
real domain retirement permits it. The fixture submits no I/O command and does
not suppress a physical hardware acknowledgement. These are QEMU checks, not
physical-device recovery or whole-node exhaustion proofs. See the
[audit record](../reports/audits/2026-10-07-security-iommu-admission.md).
