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
using its captured translation identity, tears down the private tree and
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

## Physical release and charges

Both architecture destructors now use `FrameRelease`. Before invalidation or
physical teardown can fail, the owning root is disarmed against repeated
destruction and its heap/image accounts are made nonrefundable. The bounded
private-table walk and tracked data-frame releases continue after an allocator
rejection. Only success for **every** release allows the later account-field
destructors to return charges for released backing. Earlier provisional
quarantine remains nonrefundable even after a successful walk. A failure conservatively retains both
accounts in full, even when other frames were successfully released. There is
no per-failed-frame allocation, retry queue or automatic charge recovery.

An allocator rejection is logged without further changing its allocation
bitmap. This does not repair pre-existing corruption or make a frame reported
as already free allocated again. Fault-injection tests reject before the real
allocator and verify that those frames remain allocated. Kernel tables and
foreign leaf backing never enter this owning release path.

This failure differs from failed **invalidation**: once quiescence has been
established, detached physical backing cannot be reached through the old root.
Software slots and hardware tags may complete their normal retirement even
when physical release rejected a frame. Charges remain consumed at the node,
independent of any successor generation. Failed invalidation instead retains
the entire root, tag and slot. A premature exit from the physical walk leaves
accounts nonrefundable; kernel panic/unwind recovery is not tested here.

Heap/image provisional rollback now uses
[joint frame-and-charge preparation](kernel-backing-preparation.md), whose
retained pages are excluded from successful root refunds. This does not cover
all physical-release callers. Uncharged translation preparation logs failures;
translation frames themselves still lack admission accounts.

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

Live operations have a separate linear slot lease. Retirement/extraction reject
nonzero counts; root close returns `OperationsInFlight` before subsystem
mutation. Explicit completion releases only the original table/generation's
count. Abandonment retains a live root, not a detached one. An owned staged close
can instead fence new lease admission and return its request owner while old
leases drain. Timeout or abandonment retains that closing root and fence; it
does not authorize destructive cleanup. Public memory-object/MMIO mapping,
explicit device close and direct loan revocation now acquire live leases, and
the supervisor retains a staged teardown owner across pending polls. All existing
borrowed-memory IPC replies also own both roots and a reply claim through
post-IPC invalidation; abandonment prevents root close. Returned authority
stays hidden until publication. Explicit call/reply cancellation also owns both
roots through unlocked revocation. Bulk endpoint/domain IPC and whole-domain
device cleanup still need
their own completion owners before releasing outer serialization. See
[live address-space operations](live-address-space-operations.md).

Failed final invalidation or abandonment retains the whole hierarchy, physical
backing, hardware tag, software slot and backing accounts. There is no automatic
recovery or administrative reclamation API. Quarantine can reduce capacity.
`QuiescenceFailed` is a kernel-side fault-adapter result; the current real x86
delivery failure stops the initiator rather than returning a retryable error.
Missing acknowledgements still stall.

## Remaining SEC-18 work

This corrects the **final root** boundary: its own lifecycle/table guards no
longer surround the last rendezvous or `AddressSpace::drop`. Earlier
whole-domain memory/device cleanup and bulk IPC cleanup retain lifecycle/IPC
serialization across some x86 invalidations. Public live mapping, direct loan
revocation, all existing borrowed-memory replies and explicit call/reply cancellation
now supply their own leases and completion owners. Remaining paths need the
same composition before guards can be released. The final-root lease does not
provide such a lease for an arbitrary still-live mapping operation.

Thus complete x86 teardown progress, recoverable shootdown failure and full
hardware-walk quiescence remain open. See
[memory-object retirement](memory-object-retirement.md),
[kernel frame retirement](kernel-frame-retirement.md) and
[page-table lifetime](page-table-lifetime.md).

## Verification

The host test runner now compiles the kernel's generic slot owner as a standalone
Rust test crate. Eighteen tests cover detach-before-destroy, delayed reuse,
destructor ownership, abandonment, table identity, stale generations, failed
preflight and interleaved completion without allocation, including a corrupted
completion-capacity fixture. These are serialized state/interleaving tests.
Five live-lease tests additionally cover overlapping counts, pre-allocation
rejection, completion identity, counter limits, vector growth and abandoned
lease/table destruction.
Six staged-close tests cover fencing, exact close authority, preparation
rollback, old-lease completion, abandonment and allocation-free final detachment
after completion-capacity refresh.

Single-mutator guest fixtures check failed preflight before namespace/backing
retirement, guard availability during final invalidation, exact physical and
heap-charge release, registration while another slot is detached, hardware-tag
non-reuse on ARM and stale-handle rejection after successful reuse. Failed-barrier
and abandonment probes permanently retain **two private roots**, each with one
charged heap data page plus its translation frames, software slot and ARM tag.
Their physical frame counts are logged; no test bypass frees them. They are
additional to earlier memory-object/kernel-range quarantine probes.

Live-operation fixtures verify busy-close non-mutation, retained tag/backing,
continued admission, release without lifecycle re-entry and stale/detached
acquisition rejection. Abandonment retains one additional live root with one
charged heap page and private tables. Its namespace stays present and close
remains busy, unlike the two detached-root probes above. Staged-close fixtures
add pending-owner return, new-lease rejection, post-guard cleanup and zero-budget
wait success/timeout. The timeout probe retains one further closing root, one
heap-page charge and its private tables. Its lease subsequently completes, but
the abandoned close fence remains; another caller cannot reclaim it.

Architecture-shared destructor fault adapters exercise normal release, each of
four table/root and two heap/image release positions, and rejection of all six.
They check continued release after an error, no repeated teardown, account
retention through field destruction, borrowed-root protection and preservation
of a foreign leaf. The seven failing roots leave **12 physical frames, seven
heap-page charges and seven image-page charges** permanently retained. Some
charges intentionally exceed the remaining physical backing. These are
unpublished roots, not concurrent running-domain teardown tests.

AArch64 executes these probes and the security regression. x86 compilation does
not execute the rendezvous or establish recipient progress. Real physical OOM,
concurrent hardware walks and unresponsive-LP recovery are not tested here.
