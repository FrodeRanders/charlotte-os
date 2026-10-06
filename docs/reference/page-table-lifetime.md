# Page-table lifetime

User address spaces own their private translation hierarchies. AArch64 uses
TTBR0; x86-64 owns the lower-half PML4 subtrees. Kernel translation tables are
shared, and borrowed `get_current` snapshots do not own them.

## Dynamic unmap and reuse

Removing a 4 KiB mapping clears its leaf and returns its data-frame address.
It leaves intermediate tables linked, even when they become empty. The same
virtual region can be mapped again without allocating fresh table frames.
Address-space teardown walks and releases the complete private hierarchy,
including retained empty branches, after that lifetime's threads and
translations have retired. Inactive, unpublished preparation roots can be
destroyed directly; borrowed snapshots never release a tree.

This replaces the previous empty-table pruning: both walkers returned table
frames to the physical allocator before removing their parents and completing
invalidation. Hardware walks are not serialized by the software table mutex.
Keeping the tree owned removes that premature reuse path without an allocation,
retirement queue or cross-core rendezvous inside dynamic unmap.

The memory cost follows the hierarchy's high-water footprint. Repeated use of
the same regions reuses cached tables; mapping additional sparse regions can
grow it. Kernel trees retain empty branches for the kernel lifetime. There is
now a separate [private translation budget](translation-admission.md); heap/image
admission does not charge these frames. Roots, partial linked trees and retained
empty branches remain charged until confirmed physical teardown. Shared kernel
tables remain outside this budget; there is no complete kernel-table pool or
physical-memory ledger.

An empty linked subtree also prevents installing a large/huge leaf over that
subtree. Automatic page-size promotion and live tree compaction are not
implemented. A future compactor must detach into an owning transaction, establish
walk quiescence, and only then recycle/refund frames. Failed invalidation must
retain or quarantine ownership.

## Publication and data-frame release

New child tables are zeroed before their parent link becomes valid. A release
fence orders initialization before publication. x86-64 assembles permissions
and cache selection privately and publishes each new entry with one aligned
volatile store. Its walker no longer has a second root-construction path that
could load an uninitialized CR3 or clear an existing root. Runtime root
construction belongs to `try_new_user`.

Both walkers now retain [owned table preparation](page-table-preparation.md)
through the final publication boundary. Private/lower-half table allocations
also reserve their exact owning root's table account. Ordinary roots preserve
the physical progress floor; trusted platform roots and shared higher-half
kernel tables may consume its reserve. Partial linked trees remain owned and
charged on mapping failure. Admission does not change invalidation obligations.

The zeroing `map_page` path initializes data before exposing its leaf on both
architectures. `map_existing_page` preserves data already initialized and owned
by its caller.

Range invalidation iterates checked page addresses rather than constructing an
exclusive end. This permits flushing the final virtual page, whose exclusive
end is not representable; genuinely overflowing page offsets remain rejected.

Retaining intermediate tables does not make returned data frames immediately
reusable. AArch64 leaf unmap completes broadcast invalidation; x86-64 leaf
unmap invalidates locally and requires a completed cross-LP operation before
physical reuse. Such a rendezvous must not hold interrupt-masking locks that
can prevent a recipient from servicing its IPI.

SEC-18 now has an owning [kernel data-retirement boundary](kernel-frame-retirement.md):
range cleanup detaches before post-guard invalidation/release, and x86 failed IPI
delivery cannot count as acknowledgement. Several x86 user/device/lifecycle
invalidation callers still retain other masking guards. Their retirement phases
and complete cross-LP teardown safety remain open.

Memory-object cleanup also has a [retirement pin](memory-object-retirement.md)
independent of DMA/copy retention. Final unpin cannot bypass its invalidation
fence, and failed detach/rollback retains charged backing. Its composed root
leases permit physical completion outside lifecycle/IPC serialization.

[Final root retirement](address-space-retirement.md) now detaches the private
hierarchy into a software-slot-leasing owner. Its final invalidation/destruction
runs after lifecycle/table guards are gone; ARM invalidates the captured hardware
tag rather than a detached table entry. The slot is reusable only after physical
destruction. Failure/abandonment retains hierarchy, charges and slot. Earlier
mapping/IPC/device lock-held invalidations and full quiescence remain open.

## Verification

The synchronous boot fixture builds two private, never-installed roots mapping
the same physical data through three sparse leaf tables each. It checks fourteen
table frames in total, duplicate rejection, sixteen unmap/remap rounds with
stable physical counts, failed repeated unmap, independent alias survival,
rejection of block promotion over retained tables, and complete private-tree
release. It also returns the backing frame and checks the original free count.
The existing higher-half VM fixture now invalidates the final kernel page
before returning its data frame, covering the exclusive-end arithmetic edge.

This fixture executes on AArch64 and x86 QEMU. It is not
a concurrent hardware-walk race reproduction, forced allocator-OOM test,
failed-IPI test, or proof of quiescent teardown. See the
[investigation](../reports/investigations/2026-10-05-page-tables-locality-and-admission.md)
for the translation-memory and NUMA implications.
