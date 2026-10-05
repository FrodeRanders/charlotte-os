# Page tables, locality and resource admission

Date: 2026-10-05. Source review and design implications, not a hardware benchmark.

## Assessment

Fernando Simões's [Page table memory consumption](https://frn.sh/pagetables/)
is relevant to Charlotte's resource accounting and future NUMA placement. It
does not call for replacing our native radix tables, capability isolation,
LP-affine shards or cluster placement model. It reinforces the need to measure
and admit translation metadata separately from data backing.

## Memory cost in Charlotte

An address space owns its user translation tree. Shards/threads in that domain
share the tree; independent protected domains do not share user page tables
merely because they map the same memory object. Sharing or lending physical
data therefore does not eliminate destination page-table cost.

For our 4 KiB tables with 512 eight-byte entries, a dense 1 GiB mapping needs
512 leaf tables, or 2 MiB, plus upper levels. Mapping that same physical region
in 512 independent domains needs roughly 1 GiB of leaf tables even though the
data backing remains 1 GiB. This is geometry, not a Charlotte benchmark or a
claim that such a workload is presently admitted.

Sparse mappings are less favorable: 512 individual data pages placed in 512
different 2 MiB regions can require 512 leaf tables. Leaf metadata alone then
matches the 2 MiB of data, before upper levels. Budgeting tables as a fixed
fraction of backing bytes would miss this. Count actual allocated table frames,
including roots, partial mapping preparation and retained empty branches.

Dense page-table entries also improve cache-line locality during walks. A
64-byte cache line contains eight eight-byte entries; this does **not** establish
that either supported CPU automatically fills eight complete TLB translations
on every miss. Hardware walk caching and TLB fill policy must be measured.

## Affinity and future NUMA

[Mitosis](https://arxiv.org/abs/1910.05398) studies page-table placement on
multi-socket machines: moving a workload or placing its data locally can leave
translation walks accessing remote memory. Its replication/migration approach
demonstrates that translation placement is a separate policy dimension.

[Hydra](https://www.usenix.org/conference/atc24/presentation/gao-bin-scalable)
uses lazy, partial page-table replication and knowledge of sharers to avoid
the coherence and shootdown costs of eagerly copying complete trees. Its
results argue against treating replication as an unconditional optimization.

Our inference: LP affinity helps avoid unnecessary private-cache and execution
state churn, but does not by itself guarantee local page-table memory on a
NUMA machine. Charlotte currently has a global physical allocator rather than
NUMA-aware frame selection. AArch64 uses hardware ASIDs; x86-64 has no PCID
policy and deliberately flushes non-global translations through CR3 reloads.
Affinity therefore must not be advertised as preserving all TLB entries across
context switches on both architectures.

NUMA domains within one computer are not Charlotte cluster members. Cross-host
application placement and Raft do not supply a coherent hardware page-table
tree. The future intra-host policy needs LP-to-NUMA topology, placement of both
data and translation pages, and measurements of walks/misses/remote accesses.

Retain the current constrained, filtered migration policy. Consider table
migration or replication only after locality measurements justify its memory,
update-coherence and invalidation costs. Huge user mappings also need compatible
permissions, backing continuity and correct subrange revocation/splitting;
they cannot be substituted blindly for 4 KiB capability mappings.

## Implementation sequence

Heap and ELF/runtime backing now have independent physical budgets. Translation
roots and intermediate tables remain outside those pools. The first follow-up
makes initial root preparation fallible: runtime x86-64 construction owns and
zeros its fresh PML4, preserves the physical progress floor, and returns
`RootAllocationFailed` before namespace publication rather than panicking.
AArch64 keeps its lazy-root policy. Trusted mandatory fixture constructors may
still panic; the runtime loader uses `try_new_user`.

Before enforcing complete page-table quotas:

1. Dynamic unmap now retains empty tables linked in their owning hierarchy for
   reuse until quiescent address-space teardown. It no longer recycles table
   frames before clearing parents/invalidation. New tables/data are initialized
   before publication; x86 entries are assembled before one publishing store.
   See [the lifetime contract](../../reference/page-table-lifetime.md).
2. Kernel ranges now use [owning retirement](../../reference/kernel-frame-retirement.md),
   and x86 failed IPI delivery fails closed rather than crediting a recipient.
   [Memory-object retirement](../../reference/memory-object-retirement.md) now
   retains backing through the final DMA/copy unpin and mapping invalidation;
   failed detach/rollback quarantines the original charge. Its installed-prefix
   and leaf-identity checks preserve foreign mappings. Those receipts still
   require lifecycle/IPC serialization and do not lease a generation.
   [Final root retirement](../../reference/address-space-retirement.md) now
   separately leases the software slot while a detached hierarchy completes
   post-guard invalidation/destruction. Earlier live mapping and loan/device
   operations still need their own generation-fenced, lock-safe phases.
   Owning-root destruction now retains whole heap/image charges when any
   physical release fails; only complete successful release permits refund.
   [Joint provisional preparation](../../reference/kernel-backing-preparation.md)
   now retains the original domain/node charge on rejected release or uncertain
   publication, including across root destruction; translation frames remain
   uncharged.
   Close the remaining user/domain locking gaps (SEC-18), then enforce
   reliable quiescence at every physical release. Future live compaction must
   own detached tables until invalidation and walk quiescence are established.
   Never hold a registry required by an IPI recipient across a synchronous
   rendezvous. Failed or pending invalidation must retain frames and charges,
   not credit them to a successor generation.
3. Admit each private root/intermediate frame before allocation, with domain and
   node limits and trusted platform headroom. Count shared kernel tables once,
   not once for every user root referencing them. Partial linked trees must
   remain charged until rollback or safe teardown actually releases them.
4. Expose raw current/peak counts, allocation failures and retirement backlog
   for telemetry. Keep hard admission exact; apply low-pass filtering only to
   placement/control feedback, not the underlying measurements or quotas.
5. Test dense/sparse aliases, mapped shared backing across domains, partial
   allocation failure, concurrent unmap/reuse and post-shootdown refunds before
   attempting huge pages, table sharing or NUMA replication.

The dynamic table-unmap correction is tracked as SEC-17 in the remediation
ledger; the broader physical-release gaps are SEC-18. Sparse alias/reuse and
teardown fixtures exercise ownership, not a reproduced hardware race or exploit.
This is implementation work within the existing architecture, not a reason to
discard its isolation or placement design. The research supplies no evidence
that page walks caused the Kafka/QEMU throughput observed earlier; that requires
separate profiling on the relevant execution environment.
