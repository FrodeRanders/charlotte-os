# Joint user-backing preparation

`PreparingUserBacking` is the kernel owner for one provisional heap or
ELF/runtime data page. It retains a physical-frame owner, a node reservation
and an exclusive borrow of the original `AddressSpace`. The caller keeps the
address-space table guard throughout the operation, after checking its exact
generation. Cleanup never resolves a reusable numeric ASID.

## Admission and publication

Creation reserves the appropriate heap/image pool, prepares a frame-tracking
slot fallibly, then allocates and captures the frame before zeroing the page.
Admission, tracking and
allocation failures are distinct. Failure before allocation refunds only the
unused reservation. `fill` exposes an exclusive bounded page borrow, not frame
ownership. Heap commitment maps already-zeroed backing without zeroing it twice.
Tracking/allocation rejection performs that refund explicitly before returning;
it does not depend on a reservation-releasing preparation destructor.

The consuming `map_with` marks publication as uncertain before entering the
mapper. A normal rejection must confirm that no leaf was installed; both
production walkers return errors before leaf publication. The callback must
only perform that mapping, not alter backing policy or frame tracking.
Success commits the reservation to the original account and transfers the
frame into the preflighted root registry without allocating. Callers no longer
commit charges separately. The raw reservation/commit API is confined to the
memory implementation; owning-frame insertion asserts preflighted capacity.

Confirmed mapper rejection invokes ordinary rollback explicitly. A caller
cancelling a definitely unpublished preparation uses consuming
`cancel_unpublished`, rather than Drop. The physical rollback still executes in
the owner's borrowed table context; moving that ordinary path outside masking
serialization remains G1/G2 work in the [cross-category map](cleanup-recovery.md).

## Abandonment is not ordinary rollback

`PreparingUserBacking::drop` now only transfers the active reservation into the
captured account's nonrefundable counts and disarms its frame owner. It takes no
table, allocator or admission-pool lock, invokes no callback, frees no backing
and enters no logger. An inert committed charge is not counted twice. A frame
whose ownership has already fully transferred into the root remains root-owned.

This rule also applies to a reservation abandoned before tracking/allocation:
its domain ceiling and original pool remain consumed even though no physical
frame exists. Ordinary constructor errors still refund unused reservations.
Diagnostic charge counts therefore need not equal physically retained frames.
After abandonment there is no owner to authorize cancellation or retry; neither
root destruction nor a successor generation can recover that reservation.

Interrupted publication or ownership transfer cannot authorize deallocation.
The owner's fallback retains the frame and its charge, including when account
commit already happened but its inert reservation token is still present. It
does not count that reservation twice. A potentially installed leaf remains backed until
normal root retirement; the data frame is conservatively retained afterward.
This is quarantine, not a deferred mapping-recovery or invalidation service.
The fixtures simulate these states; they do not execute panic unwinding.

## Rollback and retained charges

For confirmed unpublished backing, cleanup first arms `ProvisionalRelease` in
the borrowed account. Its page is counted against the original domain ceiling
and marked nonrefundable **before** physical release. The physical owner is
consumed before calling the allocator, so error or interruption cannot trigger
a second release through Drop. Only confirmed release removes the provisional
account counts and refunds the captured node reservation.

Rejection, or an abandoned charge-release receipt, retains both counts. The
account stores a bounded scalar `quarantined_pages`, not a growing per-frame
ledger. Normal root destruction refunds `pages - quarantined_pages`; earlier
quarantine survives root destruction and software-slot reuse. Failed root
release still retains the whole account. No successor generation or trusted
platform promotion can return or reclassify the retained charge. Ordinary and
platform pool identity is preserved. After the original account disappears,
the retained reservation continues to consume the node pool.

This deliberately sacrifices capacity rather than declaring unconfirmed backing
free. There is no automatic retry or reclamation API. Allocator errors are
logged on the explicit rollback path, not repaired: a frame already reported
free is not made allocated again. Fault adapters reject before the real allocator and check that their
frames remain allocated.

Uncharged `PreparingUserFrame` remains inside the data/table owners and for
foreign-backing fixtures. Translation publication uses the dedicated
[PreparingTable owner](page-table-preparation.md). Failed unpublished release
is logged and not retried. Private tables retain their independent admission
account through confirmed release; they never refund a heap/image reservation.
Shared kernel tables, aggregate kernel metadata, stacks and kernel heap remain
separate SEC-07 work.
This owner retains the table guard; it does not fix live mapping/IPC/MMIO
masking-guard or x86 shootdown progress issues.

## Verification

Shared boot fixtures cover both heap/image kinds: admission rejection before
tracking, tracking rejection before allocation, rejected allocation, explicit
unused-preparation cancellation, mapping rejection, zero/fill preservation and
successful publication/destruction. Synthetic release failures exercise domain-ceiling
enforcement, ordinary/platform pools, mixed owned/quarantined teardown, software
generation reuse and fresh-frame non-reuse. Real installed leaves exercise
abandonment before commit, with an inert token after commit, and after removal
of that token before frame transfer; no deallocator is called for
uncertain publication. The three interrupted publication states now exercise
actual owner Drop while the original table, physical allocator and backing pool
guards are held. Further allocated/unallocated abandonment probes hold the same
guards, reject fresh admission at the retained domain ceiling and check charges
after root destruction and exact software-slot reuse. Abandoned release receipts
retain counts without a recovery bypass.

The failing/abandoned preparation fixtures permanently retain **fourteen physical
frames, eight heap-page charges and eight image-page charges**, additional to
earlier root/memory-object/kernel-range fixtures. Two charges have no physical
backing because abandonment occurred before allocation. These are serialized
boot probes, not physical-exhaustion stress, allocator corruption or real unwind
recovery. See the [abandonment report](../reports/audits/2026-10-09-security-preparation-abandonment.md)
for QEMU evidence and the remaining stack-retirement progress concern.
