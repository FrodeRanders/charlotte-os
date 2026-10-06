# Owned translation-frame preparation

Both architecture walkers prepare new table frames through `PreparingTable`,
including AArch64 lazy roots and x86-64 initial PML4 construction. A raw
allocation no longer sits between allocator success and parent/root ownership.
The owner captures backing before zero initialization and releases unpublished
preparation on rejection. Zeroed child contents precede a valid parent link;
x86 permission/cache bits are assembled before its aligned entry publication.

## Scope and physical progress

Ordinary private/lower-half translation requests preserve the existing one-eighth
free-frame floor. The predicate and allocation run under the same frame-allocator
guard. Each actual root/intermediate allocation checks the floor; empty linked
branches reuse existing frames without a new request. A failure may therefore
leave a partial tree, but those published tables remain owned until retry or
quiescent destruction. Heap/image page rollback does not free the tree's tables.

Trusted platform private requests and shared higher-half kernel requests may
consume that reserve for kernel progress.
They remain subject to real allocator exhaustion and owned preparation. The
architecture derives `TableScope` from its mapping context; user-accessible
mapping addresses are validated before entering the walker. Applications cannot
select a kernel-scope allocation policy. Private requests also retain their
exact [table admission account](translation-admission.md), reserved before
physical allocation and borrowed through publication or rollback. Shared kernel
requests reserve an owning charge in a separate runtime-table node pool before
allocation. Other unbudgeted consumers can still exhaust physical memory.

## Publication and root identity

All fallible preparation precedes the consuming `publish` boundary. Its callback
adopts the frame exactly once into a parent link or owning root. The owner is
disarmed before invoking publication: interruption cannot drop a frame that may
already be hardware-reachable. Unconfirmed publication sacrifices capacity;
there is no retry or administrative recovery API. This boundary does not prove
hardware-walk quiescence or make published tables safe to free early.

ARM prepares a lazy root before acquiring its hardware tag. Rejected frame
allocation consumes no tag. Rejected tag admission drops the still-unpublished
root without changing TTBR0. A successful root remains in the original address
space and retains its tag through normal retirement. x86 constructs the zeroed
PML4 and copies shared kernel entries before adopting the root; its old scalar
root-transfer helper is removed. Kernel preparation failures still follow each
caller's existing fatal or fallible policy.

Private trees retain linked empty tables and release them only through the
existing [lifetime contract](page-table-lifetime.md) and
[final root retirement](address-space-retirement.md). Mapping, IPC-loan and MMIO
ownership is composed through exact root leases and receipts. Table charges
follow retained/published and quarantined lifetimes; complete physical-platform
quiescence and broader recovery remain SEC-18 work.

## Verification

Architecture-shared boot probes inject a local, per-walker allocation gate, not
a global allocator fault mode. They reject every possible construction prefix,
check absent leaves and exact retained-table counts, retry into the same root,
remap cached empty branches with zero fresh-allocation allowance, and reject
then complete a sparse second branch. Aliases preserve their foreign data,
active hardware roots remain unchanged, and private destruction returns every
table before the separately owned data page is released. Heap/image pools stay
unchanged because tables have an independent pool, whose counts are also checked.

Separate checks cover both scope/floor predicates, zeroed unpublished-owner Drop
and rejected ARM hardware-tag admission. The existing x86 root-allocation
fixture executes in x86 QEMU. These prefix fixtures leave no additional retained
frames. Separate admission fixtures retain two rejected/abandoned provisional
frames and verify that their original charges survive root teardown. These are
serialized boot probes and QEMU regressions, not real physical OOM, publication
unwinding or concurrent hardware-walk proofs.

Shared-table admission fixtures additionally test real higher-half mapping,
partial-prefix retention, cached reuse at the ceiling, and no duplicate charge
or refund when user roots borrow the shared hierarchy. Two rejected/abandoned
shared preparations retain two more frames/charges. Unpublished success refunds
only after confirmed physical release; linked kernel tables stay charged for
the kernel lifetime. Bootloader-inherited tables remain outside runtime admission.
