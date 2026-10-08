# Final address-space retirement

An address-space handle identifies one software slot generation. Closing that
lifetime now separates logical resource cleanup from final translation-tree
destruction. `RetiredAddressSpace` owns the detached private hierarchy, its
heap/image accounts and a lease on the software slot until invalidation and
physical destruction have completed.

## Owned cleanup and root retirement

Under lifecycle serialization, close validates the exact handle and checks its
publication-prepared completion metadata before retiring any subsystem. Failure
here leaves
the namespace, table entry and backing admission active. Both immediate and staged
close own a `ClosingSlot` before irreversible cleanup. It then fences backing
and capability admission and detaches devices under lifecycle/device
serialization. `PreparedNamespaceDevices` borrows the closing root through
MMIO invalidation, scratch completion and DMA destruction after those guards
leave. Only its exact completion receipt permits further retirement. IPC loan
cleanup then borrows that closing owner and retains exact peer roots while releasing
lifecycle/IPC for physical revocation. After IPC removal, memory cleanup borrows
that owner and retains every mapped peer in existing mapping nodes before moving
records and invalidating outside lifecycle. A preparing backing pin keeps the
records visible until peer admission completes. Unmapped authority also waits for
live revocation/transfer and DMA/copy fences. Only confirmed memory completion and
zero leases permit cleanup sealing. High-water accounting and remaining
namespace metadata removal retain lifecycle. The address space is detached from the
table into the owning receipt, without returning its ID to the free-slot list.

Device cleanup can reject detachment, invalidation, scratch completion or DMA
teardown with `DeviceCleanupFailed`. It retains unfinished device records and
authority, scratch and original backing; subsequent polls cannot skip that phase.
IPC cleanup can reject uncertain loan revocation with `IpcCleanupFailed` before
the root is detached. The closing owner then retains its fence, original root,
slot, tag and backing accounts. No object/root destruction or slot refund follows
that rejection. Memory admission/physical cleanup errors likewise return
`MemoryCleanupFailed`. Unstarted admission returns its peer leases and pin;
physical failure retains every affected mapped root, backing and scratch claim.
Earlier subsystem retirement and confirmed loans/objects are not rolled back. The retained namespace rejects fresh operation leases and competing close;
there is no retry or force-clear recovery API. The supervisor propagates/caches
this terminal teardown error rather than treating it as successful reclamation.

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
destruction and its heap/image/table accounts are made nonrefundable. The bounded
private-table walk and tracked data-frame releases continue after an allocator
rejection. Only success for **every** release allows the later account-field
destructors to return charges for released backing. Earlier provisional
quarantine remains nonrefundable even after a successful walk. A failure conservatively retains all
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
all physical-release callers. Private translation preparation retains its
[owning admission account](translation-admission.md) through publication or
confirmed release; shared kernel tables remain outside that pool.

## Slot ownership and failure

The generic `IdTable` retirement primitive retains an inline `RetiredEntry<T>`
with a table-identity/slot-generation token. `ManuallyDrop` prevents resource
destruction when an unfinished receipt is dropped. Explicit `release_value_with` completion reports the owning physical walk
before payload destruction and returns a linear slot-completion token; it cannot be cloned. A token from another
table or generation cannot make a slot reusable. The former destructive
`remove_element` helper is removed; scheduler extraction still uses `take_element`
where a different owner takes over immediately.

New-slot publication fallibly prepares all three metadata vectors: payload,
generation and enough free-ID capacity for every slot. Reusing an available
slot does not allocate. Failed publication returns the unconsumed payload before
any slot/generation mutation. Runtime thread and address-space registration use
this fallible path; rejected owners are destroyed after serialization leaves.
Address-space slot preparation failure returns `TableAllocationFailed` and
releases its unpublished root and hardware tag outside lifecycle/table guards.

Ordinary extraction, close preflights and final slot completion do not allocate,
even when slots are added during an unlocked close or detached owners finish in
between. Preflights check capacity rather than growing it. A missing-capacity
invariant rejects before ordinary extraction or close fencing; an already-closing
owner retains its root/fence on rejection. No cleanup path repairs that invariant
by allocating. This slot primitive does not add a namespace or kernel-heap budget;
high-water vector backing remains retained for table lifetime. Private translation
frames have separate admission. Evidence:
[slot return storage audit](../reports/audits/2026-10-07-security-slot-return-storage.md).

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
roots through unlocked revocation. Explicit endpoint close additionally borrows
its server-root owner across each queued call's unlocked cleanup, including when
a staged server close is already pending. Whole-domain IPC loans now compose
closing ownership and peer cleanup leases, including already-closing peers;
ordinary admission stays fenced and cleanup admission seals before backing
teardown. Whole-domain devices now borrow the closing root through unlocked
physical cleanup. Whole-domain memory now retains all mapped roots through its
own receipts outside lifecycle. Published move/copy/result attachment cleanup
and unpublished copy/vector rollback now release backing outside IPC. See
[live address-space operations](live-address-space-operations.md).

Failed final invalidation retains the whole hierarchy, physical backing,
hardware tag, software slot and backing accounts. Production final invalidation
makes up to three fresh x86 rendezvous attempts, then transfers its complete
owner into a [bounded final-root recovery registry](root-recovery.md). Capacity
rejection returns ownership and quarantines outside the registry hold. A trusted
kernel controller can retry that registered invalidation receipt; no automatic
worker or userspace mutation API is added. Abandoned owners and physical-phase
failures cannot be adopted or retried. Successful physical release remains
consuming. Quarantine can reduce capacity. See
[hardware quiescence](hardware-quiescence.md).

## Remaining SEC-18 work

This corrects the **final root** boundary: its own lifecycle/table guards no
longer surround the last rendezvous or `AddressSpace::drop`. Published IPC
move/copy/result attachment cleanup now detaches unmapped backing and its original
charge under IPC, then releases outside serialization. Unpublished copy/vector
rollback now also runs outside IPC. General allocator/metadata work under IPC
remains open. Whole-domain IPC loan
revocation now runs outside those guards through borrowed closing ownership and
exact peer cleanup leases. Whole-domain device cleanup now runs outside lifecycle
with its own receipt; uncertain DMA completion retains the root. Loan revocation
rejects DMA-pinned loans before claiming them. Whole-domain memory cleanup now
owns backing, scratch, authority and mapped peer leases before releasing lifecycle.
Unmapped revocation peers stay Pending until owned prior state returns. Public
live mapping, direct loan
revocation, all existing borrowed-memory replies and explicit call/reply cancellation
now supply their own leases and completion owners. Remaining paths need the
same composition before guards can be released. The final-root lease does not
provide such a lease for an arbitrary still-live mapping operation.

Complete platform/device quiescence and recovery of abandoned owners remain
open despite bounded shootdown retry and tested QEMU NVMe reset. See
[memory-object retirement](memory-object-retirement.md),
[kernel frame retirement](kernel-frame-retirement.md) and
[page-table lifetime](page-table-lifetime.md).

## Verification

The host test runner now compiles the kernel's generic slot owner as a standalone
Rust test crate. Twenty-seven tests cover detach-before-destroy, delayed reuse,
destructor ownership, abandonment, table identity, stale generations, failed
preflight and interleaved completion without allocation, including a corrupted
completion-capacity fixture. These are serialized state/interleaving tests.
Live-lease cases cover overlapping counts, pre-allocation
rejection, completion identity, counter limits, vector growth and abandoned
lease/table destruction.
Staged-close cases cover fencing, exact close authority, preparation
rollback, old-lease completion, abandonment and allocation-free final detachment
after publication-prepared capacity validation. Four return-storage cases check
publication rollback with the payload returned, mixed extraction/retirement,
allocation-free reuse and rejection without repair on corrupted capacity.

Guest registration fixtures reject slot publication 64 times after real root,
namespace-metadata and hardware-tag preparation. They check lifecycle/table/kernel
mapping/physical-allocator guard availability before destroying the returned
root, restoration of free frames and table charges, no namespace/limit
publication, preservation of an unrelated live namespace and exact slot/generation
recovery. The fixture
substitutes only publication and asserts at the ordinary rejection-release
boundary; it does not force physical OOM or a real allocator corruption.

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

Device ownership and DMA-pinned loan regression evidence is recorded in the
[device retirement remediation](../reports/audits/2026-10-06-security-device-retirement.md).

Mapped-peer memory ownership and unmapped reader retention are documented in
the [memory cleanup remediation](../reports/audits/2026-10-06-security-memory-retirement.md).
