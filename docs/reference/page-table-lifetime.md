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
currently no private-table quota or kernel-table pool; heap/image admission
does not charge these frames. This policy is a lifetime correction, not an
aggregate memory-exhaustion solution. Full table admission must count actual
roots and intermediate frames, including retained branches and partial
preparation, until they are physically released.

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

## Verification

The synchronous boot fixture builds two private, never-installed roots mapping
the same physical data through three sparse leaf tables each. It checks fourteen
table frames in total, duplicate rejection, sixteen unmap/remap rounds with
stable physical counts, failed repeated unmap, independent alias survival,
rejection of block promotion over retained tables, and complete private-tree
release. It also returns the backing frame and checks the original free count.
The existing higher-half VM fixture now invalidates the final kernel page
before returning its data frame, covering the exclusive-end arithmetic edge.

This fixture executes on AArch64; x86 execution requires an x86 guest. It is not
a concurrent hardware-walk race reproduction, forced allocator-OOM test,
failed-IPI test, or proof of quiescent teardown. See the
[investigation](../reports/investigations/2026-10-05-page-tables-locality-and-admission.md)
for the translation-memory and NUMA implications.
