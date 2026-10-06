# Kernel data-frame retirement

Kernel-range cleanup now separates mapping removal from physical release.
`RetiredKernelRange` owns removed backing through invalidation. Kernel-stack
teardown and failed range preparation use this owner; the previous
`unmap_and_deallocate_range` helper, which freed data before unmapping, is removed.

## Owning preparation and cleanup

The caller creates a receipt outside its arena guard. Once installed, the
kernel mapping owns each frame. On partial preparation failure, the unpublished
frame rejected by mapping joins the same receipt as the successfully installed
prefix. Its physical release failure is reported by explicit post-guard cleanup,
not hidden by a provisional frame destructor. Only this operation's installed
prefix is detached; a failing `AlreadyMapped` cannot steal a pre-existing leaf.

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

The fallible synchronous sender now uses one monotonic request epoch and one
acknowledgement per LP. Each acknowledgement follows a flush that includes
global translations and paging-structure caches. Delivery rejection and missing
acknowledgements return errors without crediting the failed participant. A new
attempt cannot count stale or duplicate acknowledgements. Coordinator admission
is bounded to 500 ms, followed by a separate 100 ms acknowledgement budget.
Initiators must have IRQs enabled and hold no masking guard; the coordinator
wait yields before acquiring ownership. Legacy mandatory callers panic on
rejection, while owning retirement receipts retain backing for explicit retry.

The scheduled, pinned reaper on each x86 LP releases dead-thread stacks with
IRQs enabled. IRQ tails only stage retirement; they do not wait synchronously
on an initiator they may have interrupted. A timeout leaves backing unavailable
and does not prove that an unresponsive LP cannot resume. Retry requires a
fresh successful rendezvous. See [hardware quiescence](hardware-quiescence.md).

## Remaining SEC-18 scope

Kernel-range callers now release their arena/page-table guards before physical
cleanup. Several user-memory, device and address-space lifecycle paths still
invoke x86 invalidation while retaining other interrupt-masking guards. Those
need explicit retirement phases that preserve captured generation, mapping
state, loans/pins and charges while releasing locks before the rendezvous.
Abandoned-root recovery and complete platform/device quiescence remain open.
Runtime kernel/user stack pairs now have [capacity admission](stack-admission.md)
that survives uncertain range retirement. General kernel-range/heap and metadata
budgets remain part of SEC-07; inherited boot stacks remain outside runtime admission.

[Memory-object retirement](memory-object-retirement.md) now separately retains
backing through mapping invalidation even when the final DMA/copy pin releases
concurrently. Partial detach and failed rollback quarantine rather than recycle
uncertain backing. This is an ownership prerequisite for the remaining lock-safe
phase work, not a correction of the outer x86 lifecycle/IPC locking gap.

The [final user-root boundary](address-space-retirement.md) now leases its
software slot while retaining the detached hierarchy through post-guard
invalidation/destruction. This removes lifecycle/table guards from that final
step; earlier memory-object/IPC/MMIO invalidations still need their own phases.

## Verification

Single-mutator boot fixtures verify detach-before-release, retained data after
an injected failed barrier, successful retry and repeated release, allocation
and mapping failure after an installed prefix, real `AlreadyMapped` protection
of foreign backing, and pre-mutation metadata/range rejection. A Drop fixture
deliberately quarantines **one 4 KiB page** for the lifetime of the test guest;
its free-frame count must not increase and its diagnostic count must increase.

The host epoch tests cover stale, duplicate and non-regressing acknowledgements,
exclusive coordinator ownership and identity exhaustion. The boot fake sender
checks rejected delivery. A running four-LP x86 fixture omits one actual IPI,
then separately omits one acknowledgement, retaining its exact root, slot and
charge before a fresh real rendezvous releases them. This does not stop a CPU
or prove physical-platform failure recovery. See the
[staging/quiescence audit record](../reports/audits/2026-10-06-security-staged-quiescence.md).
