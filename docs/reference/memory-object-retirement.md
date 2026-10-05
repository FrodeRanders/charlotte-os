# Memory-object mapping retirement

Memory-object backing and its creating generation's quota charge now survive
mapping removal until invalidation completes, independently of DMA/copy pins.
This repairs a teardown race: the final external unpin could previously destroy
a deferred object after the registry was unlocked but before its shootdown.
Failed bulk unmaps could also lead to physical release with live leaves.

## Owning the gap

`MappingRetirementPin` acquires a registry pin before mapping work or detachment.
Only explicit consuming release after invalidation decrements it. Its Drop
does nothing: an abandoned operation retains the pin, backing and original
charge in the registry. It neither takes a lock nor initiates invalidation.
Object IDs are monotonic and are not restored through reusable ASID lookup.

`RetiredObjectMappings` owns detached mapping records plus that pin. Domain
cleanup processes one object at a time, moving its existing mapping tree rather
than allocating temporary vectors of objects, invalidations or backing. A
monotonic cursor avoids rescanning processed objects. Final frame release
requires owner retirement, no DMA/copy/retirement pins, and no mapped aliases.
It runs after releasing the object registry, with the sponsorship charge still
owned through physical release.

## Lock-separated detachment

Retirement preparation moves the mapping records and acquires the backing pin
under the registry guard. The receipt initially records that physical leaf
detachment is incomplete. Preparation consumes and drops that guard before
returning, so a chained detach call cannot accidentally extend a borrowed
temporary guard's lifetime. After release, the pin copies borrowed
frame identities into a fixed stack batch of 16 entries. Each batch's table walk
runs without the registry held. The batch is not a frame owner, a new object-size
limit, or a teardown heap allocation; larger objects use successive batches.

Ordinary unmap, mapped-loan revocation (including its IPC adapter), and domain
cleanup share this path. It removes the previous registry-to-address-space-table
nesting in unmap and bulk cleanup, matching mapping's existing separation. The
pin keeps the frame list immutable and backing charged across every registry
unlock, including a concurrent final DMA/copy unpin. Prefix lengths are checked
against that pinned list before invoking the walker; each leaf still must match
its expected physical frame.

All batches are attempted, retaining the first error. Any failure leaves the pin
undischarged, even when some leaves have been removed. Domain cleanup moves
records before detachment; ordinary unmap removes its record only after the full
detach succeeds. Invalidations and scratch/authority completion retain their
existing ordering. This separation does **not** release the outer lifecycle/IPC
guard and is not a solution to x86 interrupt-masked rendezvous.

Scratch now records live extents with fallible admission before publication;
release removes an exact existing reservation without allocation. Bulk finish
retains its pin and loan restrictions on any scratch-completion error, rather
than ignoring that result. The outer window-registry insertion and general
metadata admission remain SEC-07 work. See
[scratch admission](scratch-admission.md) for policy, costs and verification.

The pin also protects ordinary unmap and failed-map rollback: a concurrent
capability close cannot recycle backing just because its mapping record has
already been removed. Mapping frame-list preparation reports allocation failure
before publication. Mapping, copy/snapshot, write, DMA, transfer and loan-grant
paths reject new access while retirement is pending. This is an operation fence,
not revocation of an already executing CPU or a previously authorized DMA job;
those still require their own teardown and invalidation.

Borrower cleanup retains loan restrictions until invalidation finishes. Failed
detachment, failed invalidation, failed scratch release during ordinary unmap,
or abandonment keeps the retention pin. Quarantine preserves the original
generation's charge even after its namespace and translation tree disappear.
It has no recovery API. It is bounded by the existing charged backing-object
pool, but can reduce available capacity; it is not a successful cleanup or a
substitute for fixing an underlying failure. A diagnostic records failed bulk
detach/invalidation; there is no external quarantine telemetry field yet.

## Partial installation and foreign leaves

Each mapping records the prefix actually installed. If rollback fails, that
record remains instead of reporting the object as unmapped. Cleanup may only
visit the installed prefix, never the foreign leaf where installation failed.
Leaf detachment verifies its physical identity before removal. Missing or
mismatched leaves conservatively fail detachment; they do not authorize backing
reuse. Incomplete rollback therefore quarantines backing rather than attempting
an unsafe reverse cleanup. Scratch ranges are not returned before all associated
detach/invalidation checks succeed.

## Remaining boundary

These internal operations still require lifecycle or IPC serialization across
the complete operation. The receipts retain backing, **not an address-space
generation lease**. Numeric ASIDs and scratch identities must not be reused
between detach and finish. Dropping lifecycle merely to run the shootdown would
break that condition.

Consequently SEC-18 remains partial: several user/device/domain paths still
perform x86 rendezvous under an outer interrupt-masking lifecycle/IPC guard.
The next phase must preserve captured generation, backing, scratch reservation
and loan authority while releasing those guards. Recoverable shootdown failures
and complete teardown quiescence remain open. See
[kernel frame retirement](kernel-frame-retirement.md) and
[page-table lifetime](page-table-lifetime.md).

The [final address-space root](address-space-retirement.md) now has a detached
slot-leasing owner that finishes after lifecycle/table guards are released.
That separate close boundary does not lease an arbitrary live mapping's ASID;
the mapping operations described here still retain their outer serialization.

A [live-generation lease foundation](live-address-space-operations.md) now
retains roots through explicit completion and rejects busy close before mutation.
Public memory-object map/map-any/unmap calls hold a lease through page-table
changes and TLB invalidation, and release it on ordinary error as well as
success. Their mapping pins still independently retain backing. IPC loan
revocation continues to rely on IPC serialization; MMIO mapping is not migrated.
Compose their backing/scratch/authority owners with leases before releasing those
guards. The staged fence does not cover non-lease paths or revoke their
authority while older operations drain.

## Verification

Single-mutator boot fixtures inject the final copy/DMA unpin between detach and
invalidation, checking actual free-frame counts and exact-generation charges.
They verify loan authority before/after the borrower fence and scratch reuse
only after all mapping invalidations. Real collision leaves exercise clean
rollback, failed rollback, installed-prefix retention and physical-identity
checks. Fault adapters cover partial detach and failed invalidation; an abandoned
receipt exercises the non-releasing Drop policy.

A 35-page fixture exercises three batches (16/16/3), ordinary unmap, mapped-loan
revocation and owner teardown. It checks registry/table guard availability at
each detach callback, rejects an oversized installed prefix before any callback,
injects the final copy unpin between preparation and the first table walk, and
verifies exact data-frame and charge release after invalidation. It adds no
permanently retained frames. These guard checks are serialized fixtures, not a
concurrent lock-order stress test.

The scratch-completion probe checks two borrower mappings, all barriers before
release, last-copy/DMA-unpin retention, successful-range reuse and failed-range
non-reuse. Its rejected completion retains one extra page/charge even after
all involved domains close.

Failure probes intentionally quarantine **seven 4 KiB data pages and five object
charges** for the test guest's lifetime. They never re-adopt the backing. This is
in addition to the kernel-range fixture's one quarantined page. These fixtures
model the dangerous interleaving, not a concurrent hardware-walk stress test.
AArch64 security guests execute them; x86 compilation does not establish x86
IPI progress or execute its failure path.
