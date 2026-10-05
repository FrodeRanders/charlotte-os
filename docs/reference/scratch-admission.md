# Scratch reservation admission and completion

Memory objects and MMIO share a 512 MiB virtual scratch window in each user
address-space lifetime. This is virtual space, not a 512 MiB physical allocation.
Reservations become reusable only after leaf removal and translation
invalidation. Lifecycle/IPC serialization still protects the current generation;
this allocator is not a live-operation generation lease.

## Admission before publication

`memory::object::scratch::ScratchWindow` records sorted, disjoint **live**
extents in a vector. First-fit admission scans their implicit gaps and validates
nonzero page-aligned size, window bounds and checked arithmetic. It calls
`try_reserve(1)` before inserting a reservation. Metadata rejection therefore
publishes no virtual range and does not change live extents. The object adapter
reports this as the existing `ResourceLimit`; invalid/exhausted ranges report
`OutOfScratch`. There is no compatibility-only free-tree allocator.

This replaces the bump-pointer/free-tree representation. Splitting a free extent
or returning a non-tail range previously could allocate a tree node after
logical mutation. The new release path only removes an existing vector entry;
adjacent free gaps combine implicitly. No allocation, free-node insertion or
coalescing bookkeeping is required during release.

The trade-off is linear first-fit scanning and vector shifting. The live-entry
count is bounded by the window's page count (131,072), with resource/capability
limits providing additional bounds. Metadata capacity follows its high-water
mark until the window is destroyed. This is not a per-domain or node-wide
metadata budget, nor a performance claim for heavily fragmented windows.
Creation of the outer per-AS window registry entry still uses an infallible
`BTreeMap` insertion. Comprehensive namespace/kernel-heap admission is open.

## Exact completion

Release must name the offset and size of one exact live reservation. Partial,
combined, unknown and already released extents are rejected without mutation;
checks apply in optimized builds too. Generation checks remain in the object
adapter. Exact-range matching is **not** an allocation nonce: a stale internal
scalar pair could match a later reservation at the same address. Current
callers must retain their serialized ownership records; unlocking those paths
requires real generation/reservation owners, not this range check alone.

Ordinary unmap retains its backing pin on failed scratch release. Failed-map
rollback now does the same when it owns a pin. Bulk object retirement waits for
all mapping invalidations, attempts the surviving domains' scratch completions,
and releases its pin/loan restrictions only if every completion succeeds.
Failed completion logs and quarantines the original backing and charge. Ranges
whose individual completion succeeded may be reused because their leaves are
already quiescent; rejected ranges remain reserved. Cleanup of the closing
domain's own ranges is subsumed by destruction of its entire scratch window.

MMIO explicit close reports `UnmapFailed` if scratch completion fails. The
capability is consumed and the rejected range remains reserved for that AS;
there is no unsafe restoration or retry that can revive partially removed MMIO.
MMIO failed-map rollback may retain a range while returning its original error;
it does not recycle an uncertain range. Device/lifecycle lock-held invalidations
remain separate SEC-18 work.

## Verification

The host runner compiles this production allocator directly. Six tests cover
first-fit reuse, implicit gap combination, unchanged capacity on release, exact
release rejection, local metadata-preflight failure, invalid/exhausted/overflow
requests, independent windows and a 6,000-operation bitmap-oracle trace.
These do not force real allocator OOM or test cross-LP races.

A boot fixture retires an object mapped by two borrowers, injects the final
copy/DMA unpin during invalidation, then rejects one scratch completion before
mutating the real allocator. It checks that all barriers precede release,
successful range reuse, rejected range non-reuse, retained loan restrictions
and original charge after all three domains close. That probe deliberately
retains **one additional data page and one object charge**. There is no recovery
or test-only re-adoption API. Existing memory-object quarantine fixtures bring
that group's total to seven data pages and five object charges.

See [memory-object retirement](memory-object-retirement.md) for the larger
backing lifetime and unresolved generation/shootdown boundary.
