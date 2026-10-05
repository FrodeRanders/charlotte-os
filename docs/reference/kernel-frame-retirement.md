# Kernel data-frame retirement

Kernel-range cleanup now separates mapping removal from physical release.
`RetiredKernelRange` owns removed backing through invalidation. Kernel-stack
teardown and failed range preparation use this owner; the previous
`unmap_and_deallocate_range` helper, which freed data before unmapping, is removed.

## Owning preparation and cleanup

The caller creates a receipt outside its arena guard. A provisional frame owner
releases an allocation that fails before its leaf is installed. Once installed,
the kernel mapping owns that frame. On partial preparation failure, only the
successfully installed prefix is detached into the receipt. A failing
`AlreadyMapped` operation cannot steal a pre-existing leaf.

Stack teardown retains arena serialization while removing leaves and updating
guard-page references. Physical release happens afterward, with the arena and
page-table guards gone. The receipt completes architecture invalidation before
returning any data to the allocator. New stacks may reuse the virtual region
after detachment; its old physical frames remain unavailable until invalidation
finishes. The allocator cannot assign those old frames to the new stack early.

An incomplete detachment or failed invalidation preserves the receipt. Explicit
release can retry; already returned frames cannot be released twice. Dropping
unfinished cleanup permanently quarantines its remaining physical backing and
increments `QUARANTINED_KERNEL_PAGES`. Drop neither initiates a rendezvous nor
guesses that backing is safe to reuse. There is currently no administrative
quarantine-recovery API or external telemetry field for that counter.

This kernel boundary uses a mutable owning receipt for retry, rather than an
allocated error owner: the same helper initializes the kernel heap before a
Rust allocator exists. Its inline metadata has 256 frame slots **per operation**.
The current boot heap needs at most 132 large frames; kernel stacks use sixteen
4 KiB frames. Oversized, unaligned or overflowing operations fail before
mapping mutation. This is a bounded preparation-record policy, not a node or
domain memory quota. Future larger operations must use a planned/chunked owner
or deliberately extend the bound; unbounded rollback allocation is unsuitable.

The helper assumes caller-owned kernel data mappings and serialized range
management. It is not an arbitrary MMIO/direct-map/foreign-backing destructor.
Intermediate tables remain linked for reuse under the
[page-table lifetime policy](page-table-lifetime.md).

## x86 delivery failure

The synchronous IPI sender no longer subtracts failed deliveries from its
acknowledgement barrier. A failed send logs a fatal diagnostic and stops the
initiating LP with ownership/barrier latched. The caller cannot resume and
reuse backing; other shootdowns cannot overtake the incomplete operation.
Other LPs are not automatically stopped. This is fail-stop protection, not a
recoverable timeout/retry protocol or an availability guarantee.

A recipient that never acknowledges still stalls the existing rendezvous.
Recovery would require exact participant/epoch tracking and a way to prove
that an unresponsive LP cannot resume using old translations. Merely expiring
a timer and crediting its acknowledgement would recreate the original hazard.

## Remaining SEC-18 scope

Kernel-range callers now release their arena/page-table guards before physical
cleanup. Several user-memory, device and address-space lifecycle paths still
invoke x86 invalidation while retaining other interrupt-masking guards. Those
need explicit retirement phases that preserve captured generation, mapping
state, loans/pins and charges while releasing locks before the rendezvous.
Complete user/domain teardown quiescence and recoverable x86 failure handling
remain open. Kernel-stack/range byte admission and general metadata budgets
also remain part of SEC-07.

## Verification

Single-mutator boot fixtures verify detach-before-release, retained data after
an injected failed barrier, successful retry and repeated release, allocation
and mapping failure after an installed prefix, real `AlreadyMapped` protection
of foreign backing, and pre-mutation metadata/range rejection. A Drop fixture
deliberately quarantines **one 4 KiB page** for the lifetime of the test guest;
its free-frame count must not increase and its diagnostic count must increase.

The x86-only fake-sender fixture checks that failed delivery leaves the barrier
unchanged and excludes the initiating LP. It does not execute the fatal halt
branch. AArch64 executes the kernel ownership fixtures and security regression;
x86 guest execution, real failed/unresponsive recipients, concurrent virtual
reuse and full physical-pressure testing remain outstanding.
