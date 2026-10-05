# Final address-space retirement

An address-space handle identifies one software slot generation. Closing that
lifetime now separates logical resource cleanup from final translation-tree
destruction. `RetiredAddressSpace` owns the detached private hierarchy, its
heap/image accounts and a lease on the software slot until invalidation and
physical destruction have completed.

## Two phases

Under lifecycle serialization, close validates the exact handle and prepares
completion metadata fallibly before retiring any subsystem. Failure here leaves
the namespace, table entry and backing admission active. It then fences backing
and capability admission, drains subsystem resources, records high-water usage,
and removes authority/usage records. The address space is detached from the
table into the owning receipt, without returning its ID to the free-slot list.

The lifecycle and table guards are then gone. The detached owner invalidates
using its captured translation identity, destroys the complete private tree and
backing, forgets the exact object-budget sponsor, and finally completes its
software-slot lease. The slot becomes available only at that last step. A
replacement receives a new software generation. Lookups, heap commitment and
repeated close cannot act on a detached entry; registration cannot reuse its
still-leased ID.

On ARM, invalidation uses the owned root's hardware ASID directly, not a lookup
through the now-empty software table entry. The tag remains owned until root
destruction. Its destructor retains the existing defensive tag invalidation
before release. On x86, the completed no-PCID rendezvous flushes non-global
translations before destruction. The shared kernel tree is not owned by the
retired user root.

Domain supervision must still establish that the lifetime's threads have
quiesced **before** calling close. This change does not itself stop threads or
prove that every possible caller enters without an unrelated masking guard.

This phase refactor retains heap/image accounts through the root destructor.
The existing architecture destructors still ignore physical frame-deallocation
errors; preserving charges when such release fails needs a separate correction.
Normal release is checked by exact frame and charge counts, not by injected
allocator-release failures.

## Slot ownership and failure

The generic `IdTable` retirement primitive retains an inline `RetiredEntry<T>`
with a table-identity/slot-generation token. `ManuallyDrop` prevents resource
destruction when an unfinished receipt is dropped. Explicit value release
returns a linear slot-completion token; it cannot be cloned. A token from another
table or generation cannot make a slot reusable. The former destructive
`remove_element` helper is removed; scheduler extraction still uses `take_element`
where a different owner takes over immediately.

Free-ID storage is reserved before logical mutation for all existing slots.
Completion does not allocate, even when other detached owners finish in between.
An unexpected missing-capacity invariant fails closed rather than allocating in
completion. This does not make initial table growth fallible or add a namespace,
translation-frame or kernel-heap budget; those remain SEC-07 work.

Failed final invalidation or abandonment retains the whole hierarchy, physical
backing, hardware tag, software slot and backing accounts. There is no automatic
recovery or administrative reclamation API. Quarantine can reduce capacity.
`QuiescenceFailed` is a kernel-side fault-adapter result; the current real x86
delivery failure stops the initiator rather than returning a retryable error.
Missing acknowledgements still stall.

## Remaining SEC-18 work

This corrects the **final root** boundary: its own lifecycle/table guards no
longer surround the last rendezvous or `AddressSpace::drop`. Earlier
memory-object, IPC-loan and MMIO/device cleanup still retain lifecycle/IPC
serialization across some x86 invalidations. Live-domain mapping operations need
their own exact-generation leases, scratch and authority fences before those
guards can be released. The final-root lease does not provide such a lease for
an arbitrary still-live mapping operation.

Thus complete x86 teardown progress, recoverable shootdown failure and full
hardware-walk quiescence remain open. See
[memory-object retirement](memory-object-retirement.md),
[kernel frame retirement](kernel-frame-retirement.md) and
[page-table lifetime](page-table-lifetime.md).

## Verification

The host test runner now compiles the kernel's generic slot owner as a standalone
Rust test crate. Seven tests cover detach-before-destroy, delayed reuse,
destructor ownership, abandonment, table identity, stale generations, failed
preflight and interleaved completion without allocation, including a corrupted
completion-capacity fixture. These are serialized state/interleaving tests.

Single-mutator guest fixtures check failed preflight before namespace/backing
retirement, guard availability during final invalidation, exact physical and
heap-charge release, registration while another slot is detached, hardware-tag
non-reuse on ARM and stale-handle rejection after successful reuse. Failed-barrier
and abandonment probes permanently retain **two private roots**, each with one
charged heap data page plus its translation frames, software slot and ARM tag.
Their physical frame counts are logged; no test bypass frees them. They are
additional to earlier memory-object/kernel-range quarantine probes.

AArch64 executes these probes and the security regression. x86 compilation does
not execute the rendezvous or establish recipient progress. Real physical OOM,
concurrent hardware walks and unresponsive-LP recovery are not tested here.
