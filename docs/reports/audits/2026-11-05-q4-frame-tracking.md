# Q4 root frame-tracking admission

Baseline `bbb92379`, after diagnostic buffer admission. This batch corrects
heap/image frame-catalog capacity on both supported architectures. **Q4 remains
in progress**: wider metadata/principal coverage, allocator/control overhead,
joint heap planning and later pressure/progress gates are still required.
The [qualification evidence](evidence/2026-11-05-q4-frame-tracking.json) records
final sources, artifacts, the original failed campaign and passing repeats separately.

## Implemented boundary

The root's former ordinary `Vec<PAddr>` becomes one retaining `FrameTracker`.
It uses the existing `ByteAllocator` to admit requested catalog bytes against
`memory_capability_nodes` (zero node counts) and the same 16 MiB signed-principal
kernel-buffer counter already used by diagnostic scratch. There is no new pool,
principal registry, preallocation or reduction in existing resource/input limits.
The new inline tracker fields are included in the actual `AddressSpace` payload
layout used by existing ASID-table byte admission. Allocator overhead and the
combined multi-pool heap envelope remain separate Q4 work.

Both architectures capture trusted class and the prebound signed principal from
the constructor's translation account. ARM now constructs that account before
its complete root owner, matching the existing x86 order. Empty borrowed
snapshots allocate nothing. The original storage class remains fixed through
later growth and promotion; a later actor, ASID or name supplies no sponsor.

`PreparingUserBacking` still admits physical backing first, then prepares
tracking, then allocates a data frame. A failed byte reservation rejects before
data-frame allocation and explicitly cancels the unused physical reservation.
Growth admits the whole candidate while old capacity remains charged; it does
not charge just the increment. Successful tracking capacity remains owned even
if subsequent data-frame allocation/mapping is cancelled, and can be reused.
Neither removing identities nor an empty length refunds allocated capacity.

Late authority installation prepares funding for all previously unidentified
capacity before mutating any principal binding. The token exclusively borrows
that precise tracker. After the existing combined physical binding succeeds,
commit attaches the already-admitted sponsor without allocating or copying.
Physical-binding rejection explicitly cancels the unused byte reservation;
byte rejection leaves all physical bindings and authority absent. Existing
same-principal funding is not charged twice, and a different principal rejects.
The private allocator cell is never exposed; no allocator/cell clone escapes
the tracker. This permits the one admitted initial binding without raw-parts
adoption, a shared mutable identity object or another control allocation.

The owning physical release walk remains disarmed before its first release and
never retries partially returned addresses. Only a complete successful walk
permits explicit catalog disposal; actual deallocation precedes both byte
refunds. Failed walks and abandonment retain allocation and original charges,
including a catalog whose identities were drained. This retains capacity, not
a recovery owner for consumed physical errors. Existing provisional-frame,
invalidation, root-slot and physical-account contracts stay in force.

## Controls and final qualification

Four focused host controls use the actual tracker and byte allocator with
explicit host identity/counter adapters. They trace full growth overlap at
actual deallocation, rejected growth preserving pointer/capacity/prefix,
bulk-binding admission/commit/cancellation, prevention of duplicate/foreign
binding, and inert tracker/binding abandonment. A private unsafe source copy
that frees the catalog from Drop is rejected by the real deallocation tracer.
The native controls supply actual roots, history records and physical outcomes.
Private test principals enter at the trusted constructor boundary; those
serialized controls do not themselves test artifact-signature verification.

Native serialized controls cover signed unpublished capacity, shared sibling
roots, 256 pre-data-frame denials, rejection through the same principal counter
from diagnostic scratch, independent healthy allocation, promotion without
reclassification, full growth overlap, successful close/ASID reuse, and late
binding rejected independently by byte and physical admission before success.
Counter-only family pressure is a rejection/rollback control, not a heap-pressure
or essential-service fairness campaign.

The existing root-release fixture now additionally reconciles catalog bytes for
success, each of six release failures and all-release failure. All seven failed
catalogs remain charged, including through complete failed-root Drop beneath
history, heap and metadata-pool guards. The executed 64-bit catalogs have four
entries of capacity each: 32 bytes per failed catalog, 224 bytes retained across
these seven cases. Physical failure counts and no-retry expectations stay fixed.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting and full host suite | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19, unchanged 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each, unchanged 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed, unchanged 90-second budgets |

Catalog admission and the named retaining paths are qualified in this scope.
All five final campaigns use the exact previously qualified signed service
bundle identities; no userspace source changed. Only the two native fixture
phase files changed after the original AMD failure. The entire final matrix was
rerun; earlier passing runs remain separate. This accepts neither Q4 as a whole
nor any aggregate security gate.

## Preserved preflights and limits

Initial native compile checks caught a missing test trait import and a test
helper assuming `VectorBudget::set_limit` existed. The helper now reconstructs
only a private fixture limit while preserving every live charge. Initial host
harness checks caught relative test-module resolution, an embedded crate-root
attribute warning and a deprecated atomic method under `-D warnings`. Explicit
paths, the repository-pinned compiler and the current atomic method corrected
that harness. These failures are preserved separately from final qualification.
The original AMD native failure below is preserved separately; no deadline was increased.

## Original AMD device-fixture failure

The first AMD campaign panicked in the unchanged device stale-handle fixture:
its just-closed root had ASID 181, but the next allocation received ASID 178.
The new root-frame controls had already passed. The [original capture](evidence/2026-11-05-q4-frame-tracking-amd-failure.json)
retains kernel/bundle/disk identities, source hashes, result and diagnostics.
Earlier ARM/Intel passing campaigns remain separate from final-source repeats.

The unreserved allocator pops a shared free-slot stack; concurrent release or
allocation can change which available slot the next caller receives. Immediate
same-slot reuse is not an API guarantee during churn. The serial capture shows
an earlier CQ root at 178 and an interleaved abort marker, but records no exact
slot-return actor. An abort marker alone is not quiescence. That correlation is
not presented as a complete scheduler or physical-release attribution; the
fixture's unsupported next-slot assumption follows from the allocator contract.

The same ASID/generation/stale-close and ARM hardware-tag assertions now execute
at the beginning of the synchronous boot suite, before fixtures or operational
services publish competing user roots. Both private roots are closed within
that check. The deferred device probe retains its MMIO/IRQ/DMA tests. This changes
fixture composition, not the allocator, admission policy, deadlines or production
retirement contract. Final qualification reruns the whole combined matrix after
this phase correction; it is not a same-source repeat that erases the failure.

Earlier ARM display, Intel retirement-pin and ARM exit-watch incidents remain
unresolved. Headless results do not close them. General caller/IRQ/implicit-Drop
qualification remains Q5 work; this batch qualifies the named retaining paths,
not arbitrary containing destructors, recovery or physical platforms. Finding
dispositions and production-trust gates remain unchanged.
