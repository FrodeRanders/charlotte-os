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
`Tables::prepare_unpublished` remains a private-fixture helper. Production unit
preparation owns its complete typed payload and existing backend-slot claim;
allocation, waits and private cancellation leave local backend serialization first.
Production domain constructors instead return an owning typed rejection with
all root/CD/MSI prefixes and metadata, without cancelling under backend guards.
`DmaCreation` retains that `PrivateDomain` inside the complete grant alongside
its exact root operation and authority reservation. VT-d context-table rejection
retains the still-private domain through the same path. After local lifecycle,
device, backend and config guards leave, private table cancellation verifies
unpublished state, freezes before physical release, then refunds confirmed
backing and disposes metadata. Only afterward may the grant refund authority and
complete its root. Failure returns the complete frozen payload; Drop retains
every field, original charge, root and reservation without cleanup. There is no
hardware completion to retry for never hardware-published backing. Domain allocation/metadata preparation remains separate.

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

Creation borrows `PreparedDmaDomain`'s `DmaCreation` and records the admitted
registered domain before publishing a reachable descriptor. An armed owner
rejects before initialization/reset/allocation. Error leaves that obligation
armed: backend serialization only marks retiring and publishes abort. The
enclosing preparation invokes the ordinary unlocked destruction boundary after
local lifecycle/device/config guards leave. Confirmed destruction refunds the
capability reservation and completes the exact root; rejected cleanup retains
both alongside the registered retiring domain. No success-only scalar return
or backend-internal published rollback can lose the containing obligation.

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
The admitted domain slot and source fence remain until every table frame releases
successfully; pins and capability cleanup cannot treat rejected release as a
successful close.

Release enters a terminal frozen state before touching the allocator. Partial
failure or interruption retains the whole charge, including capacity for frames
already freed; retry never walks freed entries. Unknown provisional handoff also
retains admission and backing. Repeating successful release is harmless, but
frozen release always rejects. A successor cannot refund that original charge.
Hardware timeout before physical release may still be retried against the same
retained owner by an existing owning caller, as described in
[hardware quiescence](hardware-quiescence.md). Failed creation's publication
preparation terminally retains its root/reservation/obligation; the backend's
retiring domain alone is not authority to retry that containing preparation.

Explicit domain destruction marks retiring and publishes a rejecting descriptor
under its registry guard, then moves the complete domain and actual command
engine into `Maintenance`. Empty engine admission returns `OperationInFlight`
before ordinary backend mutation/reset. Its existing admitted domain cell remains
empty and its requester entry nonzero. Domain IDs are monotonic and never reused.
No reinsertion node or teardown snapshot is allocated.

Configuration/TLB maintenance and drain execute outside the backend guard using
the moved engine's existing queue/epoch state. Ordinary rejection restores the
actual engine and exact retiring domain together under one hold. Abandonment
retains every field and permanently fences the unit: the engine is never
reconstructed from a snapshot, restored by Drop, or borrowed through a raw
pointer after extraction. Unit table/queue/completion backing remains installed
and unit-owned for its kernel lifetime. Confirmed maintenance returns the engine
before physical cleanup; the domain/source claim remains. Absent domains retain
idempotent close when the engine is available; a claimed domain cell rejects.

Physical table release and successful ledger disposal occur outside backend
serialization. Ordinary physical rejection restores the exact frozen owner into
its existing slot; pins and charges remain. Abandonment never invokes implicit
field destructors: it retains mapping/quarantine storage, pins, table ledger and
charges, while the slot and requester fence remain. This terminal state is not
an operator retry owner. Successful completion removes the empty cell, changes
the requester to its existing reset-required tombstone, and consumes mapping/pin
collections outside the backend guard.

Boot unit initialization uses `unit_initialization::UnitState` in its existing
typed slot. A short hold claims `Vacant` before unlocked preparation. Ordinary
lookup returns `Unsupported` for vacant and `OperationInFlight` for claimed;
there is no lazy initialization under a DMA caller. Private preparation rejects
into a complete `DetachedDomain` and cancels before clearing its slot claim.
Any started hardware control, published error, failed physical cancellation or
abandonment retains the complete payload and claim without retry. Success checks
published backing and installs the complete unit once; installed maintenance
still fences initialization when its actual engine is absent. No unit shutdown,
recovery registry or administrative force-clear is introduced.

Ordinary map/unmap now moves the complete domain, actual command engine and
detached pending pin into `MappingMaintenance`. Its admitted domain cell stays
empty, requester nonzero and engine absent through unlocked sparse walking,
leaf detachment and maintenance. Public `DmaOperation` retains the exact root
lease and capability claim; close and competing operations reject that claim.
Restore exact domain/engine under one original hold before confirmed post-guard
unpin and public completion. Rejected unmap retains its pin in pre-admitted
quarantine on all three backends, without allocating exceptional reinsertion.
Abandonment retains all fields, claims and charges without cleanup. See the
[mapping-maintenance evidence](../reports/audits/2026-10-09-security-dma-mapping-maintenance.md).
Its synthetic containing-owner fixture adds one domain-table charge/frame;
table-only totals are VT-d 36/27, AMD-Vi 32/23 and SMMUv3 38/29 charges/frames.
Its two retained data frames and user root have independent accounts.

Initial domain creation now claims the existing installed-unit slot and owns the
complete unit through reset, constructor allocation, registry preparation and
initial configuration outside local lifecycle/backend guards. Admission rejects
when its actual command engine is absent or any domain cell is detached, including
the post-drain physical-finalization interval. Ordinary success/error restores
the exact unit before grant rollback; abandonment retains it and its permanent
slot fence. No registry, queue snapshot or scalar replay is introduced.
Explicit destruction and published creation rejection use unlocked maintenance/
physical cleanup; physical finalization uses the registered-state hold even if
another domain owns the command engine. Complete outer-context qualification,
registry metadata admission/destruction and abandoned-owner recovery remain
separate work. No quota override or partial-release retry is introduced.

QEMU reset now retains the exact endpoint config/BAR/ECAM claim inside the same
grant's `DmaCreation`, through configuration and busy capability publication.
Controller polling also leaves lifecycle/backend serialization through the
original grant and complete installed-unit claim. Reset uncertainty or abandonment retains the
grant and claim without destructor hardware writes. Confirmed post-guard backing
rollback precedes explicit reset cancellation; successful publication precedes
verified activation. The synthetic reset fixture retains two additional exact
roots/reservations and RAM metadata, with no new IOMMU table or DMA data-frame
charge. See the [reset-claim evidence](../reports/audits/2026-10-09-security-pci-reset-claim.md).

The [complete-unit creation evidence](../reports/audits/2026-10-09-security-dma-creation-phases.md)
qualifies claim/construction/configuration/restoration boundaries and rejection
when the retained root begins closing before publication. Its guarded synthetic
owner adds one unit and one domain table charge/frame, two independently charged
data frames and an exact root/reservation. Table-only intentional totals are
VT-d **38/29**, AMD-Vi **34/25**, SMMUv3 **40/31** charges/frames. General metadata
admission, inner fallback and all wider caller contexts remain separate work.

Explicit DMA close now keeps its admitted device record claimed through backend
retirement, with the same exact-root `DmaOperation` as map/unmap. Ordinary failure
preserves authority and completes that public claim without metadata extraction/
reinsertion; confirmed backend success precedes authority/payload removal and
root completion. Abandonment retains the claim/root, while the backend's own
typed complete owner retains unfinished backing. Partial physical release remains
terminal. The [close-claim evidence](../reports/audits/2026-10-09-security-dma-close-claim.md)
adds no intentional table/data/root retention. General grant metadata admission
and confirmed-success registry-node destruction remain separate work.

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

The [detached-domain follow-up](../reports/audits/2026-10-09-security-iommu-detached-release.md)
adds complete-payload abandonment under the same held guards, exact metadata
identity after partial rejection/restoration and terminal callback exclusion.
These private fixtures retain three additional domain charges and two frames,
bringing the table retention fixtures to **22 original charges and 15 frames**.
Real NVMe maintenance-boundary probes check guard availability, caller IRQ state,
unchanged pins/charges and competing create/map/unmap/destroy/reset exclusion.
They are reentrant boot probes, not concurrent multi-LP or outstanding-I/O tests.

The [command-maintenance follow-up](../reports/audits/2026-10-09-security-iommu-command-maintenance.md)
adds pre-maintenance guard/IRQ checks and unit-wide mutation/reset exclusion.
Private RAM tests preserve Intel busy registers, AMD timeout producer/strict
completion epochs and SMMU timeout producer/full or malformed consumer state.
They cancel private backing explicitly and restore frame/charge baselines.
The guarded containing-owner probe now also retains a real command metadata
allocation. Existing retained table counts remain 22 charges and 15 frames.
No outstanding-I/O, actual panic unwinding or physical-device claim is added.

The [published-creation follow-up](../reports/audits/2026-10-09-security-dma-creation-rollback.md)
records the backend obligation in its enclosing preparation before hardware
publication. Actual QEMU NVMe configuration followed by injected rejection
checks real post-guard maintenance/physical release with disabled bus mastering,
available PCI config serialization, table-charge/capability refund and root close.
Fake rejected cleanup retains one additional root/reservation without new IOMMU
charges/frames; all five grant-retention fixtures preserve their original roots
and authority charges. This does not test a real withheld hardware completion.

The [private-construction follow-up](../reports/audits/2026-10-09-security-dma-private-rollback.md)
adds real rejected root/CD/MSI prefixes and complete metadata preparation, with
post-guard physical/metadata probes and exact charge/authority refund/root close.
Guarded complete-grant abandonment and rejected second-frame release retain two
additional exact roots/reservations. Table-only fixture retention adds **eight
charges/seven frames on VT-d**, **four/three on AMD-Vi**, and **ten/nine on
SMMUv3**, beyond the earlier 22 charges/15 frames; root backing is accounted
independently. No failed private owner can retry a returned address. At that
checkpoint unit initialization was still serialized; the follow-up below replaces
that path. Domain creation/reset is extended by the complete-unit claim below;
general metadata admission and wider contexts remain open.

The [unit-initialization follow-up](../reports/audits/2026-10-09-security-iommu-unit-initialization.md)
checks all real private allocation-region prefixes and complete preparation,
then actual successful initialization waits/publication outside backend/lifecycle/
device/table guards. Repeat initialization does not rerun preparation. Synthetic
claim/payload abandonment, partial release and uncertain control/published
rejection retain five additional original unit charges/four frames, with no new
domain charge. Table-only intentional retention totals **35/26 on VT-d**, **31/22
on AMD-Vi** and **37/28 on SMMUv3**; independent user-root backing is separate.
Private state alone never permits replay of a started hardware control write.
These boot probes preserve the outer IRQ mask and do not qualify concurrent
initializer stress, outstanding I/O, real timeouts or platform MMIO rollback.
