# Shared capability-record admission

The kernel's unified namespace now accounts for every capability kind:
IPC, memory, completion, device, mailbox and system-observer authority.
Mailbox opens and capability-backed completion submissions (including timers,
event watches and workers), and all memory-object destinations enforce shared
admission in addition to their existing family limits. Every IPC capability
publication enforces this policy, including call-side pending/delegated authority
and returned connections. MMIO, interrupt, DMA-domain and system-observer grants
also enforce it. All six kinds are bounded; the unconverted allocator and its
budget bypass have been removed. SEC-07 remains partial for broader accounting.

| Shared admission scope | Record limit |
| --- | ---: |
| One namespace | 4,096 |
| Node total | 65,536 |
| Ordinary records on the node | 49,152 |

The remaining 16,384 records are shared platform headroom, not a per-service
allowance. Kernel platform launch policy
sets the namespace's class against an exact address-space handle. A later
promotion affects future records; existing charges retain their original
class. Applications cannot select the class or override these limits.
All allocation paths check the same limits, including kernel/platform grants.
These are not complete aggregate kernel-memory protections.

## Record ownership and publication

Every entry owns its domain/node charge. All three states count:

| State | Public authority? | Release or transition |
| --- | --- | --- |
| Staged | No | `Reservation::publish` makes it live; token Drop cancels it. |
| Live | Yes, with matching owner and kind | Typed close or namespace teardown releases the entry. |
| Escrow | No | Owning move cancellation restores the original source; committed moves revoke it. |

A rejected reservation consumes no serial. A successfully staged identity
remains consumed after cancellation; reusing it could revive stale authority.
Cancelled admission returns capacity immediately. Numeric handles are never
enough for token cleanup: tokens capture the exact namespace budget object.
Their publish, restore and Drop operations cannot alter a replacement's entry
even when both ASID and capability number are reused. Namespace teardown
releases entry charges, although retained tokens may keep the old, now empty,
budget control block alive.

`PreparedTransfer` owns every memory attachment's preparation:

- A move retains source payload ownership, source escrow and a backing pin.
- A copy owns private charged frames; no receiver payload or usable capability exists
  yet. Its source pin is released after the snapshot finishes.
- A loan retains source escrow and a backing pin, without creating borrower
  state. Read-loan preparation permits existing read-only mappings/borrowers;
  write-loan preparation requires exclusive, unmapped backing.

Every destination remains hidden until commit. Move/loan source capabilities
are hidden while preparing; a pre-existing read-only mapping can still read.
Drop restores original source authority without fresh quota, even at the
namespace ceiling, cancels destination admission and releases private
frames/pins. Source escrow restoration, pin release and clearing the source's
transfer fence share one memory-registry hold: close cannot observe restored
authority with an unfinished transfer pin. Public memory close waits outside
the registry while that fence is owned; serialized IPC cleanup instead uses a
nonwaiting busy check without consuming authority. Close rejection leaves the
payload in place, without remove-and-reinsert allocation. A committed loan keeps
the transfer fence until its preparation owner drops, preventing a new read
preparation from stealing the fence during that interval. Teardown may remove
the source; late Drop uses its immutable object identity and captured escrow
namespace, never a successor's reused scalar handle. It never reverses a
receiver-controlled mapping or live loan.
The scalar rollback APIs and the vector alias-cleanup assertion have been
removed.

`commit_transfers` validates every source and destination before publishing any of
the mixed-mode batch. `commit_transfers_with_authority` also includes fresh
reservations from IPC in that same publication. It holds the memory registry across atomic capability
publication and the remaining payload updates. Moves revoke their source slot;
loans restore it while installing borrower state; copies install their private
backing. Drop the committed owners before allowing writable access, because
their retention pins are still active until Drop. IPC vectors use this owner for
all four modes; reply memory is prepared before loan revocation and committed
afterward. `PreparedCall` owns pending-call/reply metadata, its call reservation,
optional `PreparedConnection`, memory transfers and every loan pair. Borrowed-memory
replies use `PreparedReply` to own returned-memory escrow, hidden connection
authority, loan receipts and both live-root leases through unlocked revocation.
Publication
validates all IPC and memory identities before making any live, then installs
IPC payloads/enqueues with no remaining fallible admission under the IPC guard.
A scalar-only transaction needs no memory-registry lock. A failed call restores
original source authority and refunds private backing/family/shared reservations
without creating a pending call, queued message or live delegated connection.
The
multi-state kernel upgrade helper also owns a prepared batch, but the actual
userspace upgrade syscall still accepts one state object.

If a namespace has been retired but its payload is not yet drained, cancellation
may restore its *existing* source authority for teardown; this admits no new
record. A removed or replaced namespace cannot be restored. If source teardown
has already removed its payload, the pin retains the frames until cancellation,
and the original sponsorship charge is released without debiting a successor.
Copy backing admission also checks the captured sponsor generation, so delayed
allocation cannot charge a replacement ASID. Reply tokens retain every scalar
or vector loan, with bounded tracking storage prepared before publication.
Reply and cancellation revoke all loans; queued cancellation also closes copied
and moved attachments. Successful individual revocations are removed from the
token immediately, so later failure does not cause duplicate revocation.
Committed mapped-loan revocation can still fail during unmap/shootdown; that
is distinct from private preparation cancellation, which needs no unmap.

## Lifecycle and locks

Device grants own lifecycle before taking a device/backend registry and borrow
that guard for shared reservation. MMIO/interrupt payload publication is under
`DEVICES`; an IRQ grant remains serialized with the existing uniqueness check.
DMA admission precedes stream lookup/hardware domain creation. Creation runs
without `DEVICES`, but with lifecycle, so normal retirement cannot interleave.
`PreparedDmaDomain` owns created hardware until capability/payload installation.
Failure destroys it outside `DEVICES`; backend destroy failure quarantines
reachable backing/stream state rather than freeing it. Count admission neither
accounts nor eliminates retained/quarantined IOMMU resources. Device grant errors
are `ResourceLimit` or `NamespaceRetired` (status constants 17/18).

The observer grant verifies its exact `AddressSpaceHandle` under lifecycle before
reserving/publishing. `try_start_observability_service` owns a cancellable atomic
startup claim and `PreparingObserver`/`DomainLaunchTransaction`; failed grants
close the unstarted domain and delegated bootstrap authority best-effort. No
observer registry guard spans loader/lifecycle work. The boot-critical wrapper
still treats launch failure as fatal after cleanup; this adds no automatic
restart policy or allocator-failure recovery guarantee.

IPC `reserve_cap` uses the generation retained by its registry. Endpoint
creation and direct mint/delegation prepare family-owned resources before
publishing the shared identity; rejection refunds those charges. A recipient
needs a shared slot even when the grantor sponsors its connection metadata.

`PreparedReceive` reserves and installs speculative reply authority while
exclusively borrowing IPC, before dequeuing or writing a vector result page.
That authority cannot be used by another IPC caller until the guard is released.
Its Drop removes only the speculative capability on result-write failure,
leaving the internal token, caller-sponsored record, queue and attachments
untouched. Admission failure leaves both queue and result bytes unchanged.
Commit finishes result writing and dequeues without another fallible step.
One-way messages require no reply slot and can drain a full namespace. Queued
owning memory and connections remain inaccessible after authority publication
until this commit. Result-page writing failure leaves their pending delivery
state intact. Loans retain their separate owned revocation contract.
An owned receiver should release resources or apply backpressure on resource
errors rather than spin on a queue whose reply cannot yet be admitted.

A connection-bearing reply reserves shared and family authority before revoking
any loan. Quota rejection leaves the token and all loans live. Publication then
composes its returned connection and optional memory after successful revocations.
Completed unobserved calls own returned memory/connection authority until
cancellation. Ordinary lookup rejects those pending destinations even when their
IDs are guessed. The first successful reply poll publishes both families under
IPC before marking observation; readiness waiting alone does not publish them.
Observed authority remains with the caller. Repeat polling returns the stored
result without republishing consumed grants. Direct connection mint/delegation
and non-IPC memory transfers retain immediate visibility. Individual loan
revocations are still fallible, so unmap, retirement or publication-scratch allocation failure after
a successful revocation may leave a subset revoked; it
does not publish fresh returned authority. This is not atomic TLB-shootdown or
loan-revocation rollback.

User domain creation stages a fallibly allocated budget control block before
allocating its ASID, then publishes an empty namespace with the real generation.
Teardown retires capability admission before draining subsystem payloads and
removes the namespace after draining them. This avoids an address-space lookup
inside capability allocation under another subsystem's registry guard.

Generic `reserve` owns lifecycle briefly for identity capture/admission; the
returned token does not retain that global guard. Mailbox open already owns
lifecycle and uses the guard-borrowing helper to avoid recursive acquisition.
Completion admission uses `reserve_captured` under its own registry, supplying
the generation stored in that registry. This helper rejects missing or
replacement user namespaces without looking up an ASID under `CAPABILITIES`.
Memory captures both address-space handles before taking its registry, then
uses the captured helpers under `MEMORY_OBJECTS`. Prepared owners must be
dropped outside that memory guard: releasing their pins reenters the registry.
The permanent kernel namespace and kernel-only pseudo-domain fixtures use
`None`; that is not an application-selectable identity.

Counter ordering is namespace registry → domain counter → node counter.
Counter guards never enter a subsystem or allocate. Token Drop enters the
capability registry and therefore must not run while that registry is already
owned. Do not acquire lifecycle while holding a subsystem registry. See
[lock ordering](locking.md).

## Completed allocation cutover

There is no requirement to retain old internal APIs or wire formats. The old
generic allocator/restorer names and the temporary `allocate_unmigrated` bridge
have been removed. Domain/node counters enforce their actual limits directly;
there is no boolean that exempts an allocation from policy or retirement.

These count limits do not charge allocator bytes, empty namespace/control
blocks, page tables, stack backing, kernel heap or arbitrary callback captures.
Demand-backed user heaps and ELF/runtime pages now have separate
[heap](heap-admission.md) and [image](loader-admission.md) physical admission.
`Arc` control-block preparation is fallible, but BTreeMap allocation remains
infallible. Count admission is not physical out-of-memory handling.

## Verification

Device fixtures fill a real namespace, reject MMIO/IRQ/DMA grants and check that
over-quota DMA requests never call the backend. Freeing a slot permits real
MMIO mapping/close and IRQ grant/close. Fake DMA backends check creation failure,
retirement before publication and exactly-once destroy, including a simulated
destroy error; they do not validate hardware quarantine or ACK-timeout behavior.
Staged device authority fails after exact ASID/capability reuse without affecting
the successor's usable MMIO. Observer fixtures check quota recovery and exact
generation rejection. An isolated startup claim checks cancellation/duplicate
exclusion without resetting the real observer. Pre-bootstrap failures at both
connection and observer admission reclaim an unstarted fixture namespace and
its delegated records, leaving live observer registration unchanged.

Real-domain IPC fixtures fill receiver namespaces while keeping family budgets
below their ceilings. Endpoint/direct-grant rejection refunds family metadata;
freeing one slot allows recovery. Scalar/vector receive rejection preserves
the queued call, loan, result-page sentinel and caller-sponsored records across
retries. Successful retry delivers once; caller cancellation releases the
queued loan. One-way receive succeeds at the ceiling. Invalid and read-loaned
result pages return speculative reply slots without consuming queued work.
A paused shared reservation rejects retirement before publication; queued work
and result bytes stay intact. These are deterministic kernel fixtures, not an
exhaustive concurrent proof or a new EL0 quota probe.

Call fixtures reject every scalar/vector variant at the caller ceiling, and
reject delegated connections or memory at the receiver ceiling without leaking
staged call metadata. A connection+copy with one receiver slot cannot publish
the connection alone; two slots allow enqueue, with a third needed for receive
reply authority. Paused copy-only and four-mode calls reject caller/receiver
retirement without publishing either IPC or memory authority. Original source
caps/backing/charges recover. Mapped-source and invalid-copy rejection return
call/grant reservations. A stale prepared call survives sponsor teardown,
then fails/drops after exact ASID/capability reuse without changing successor
authority, backing or records. Returned-connection quota rejection preserves a
live loan; successful retry revokes it and leaves observed authority independent
of the pending call. These are deterministic kernel fixtures, not allocator
failure injection, exhaustive scheduling exploration or a new EL0 quota probe.

Kernel fixtures fill 4,096 actual records across all six kinds, test hidden
staging/cancellation, source rollback at capacity, committed revocation and
capacity reuse. A staged batch cancels every entry after rejection; this is
not an atomic vector-admission API. Exact-number replacement fixtures preserve
same-state successor entries when old tokens fail and drop. Real address-space
teardown/reuse checks generation fencing.

Real-domain memory fixtures fill spare namespace slots with kernel-only dummy
records. Allocation/copy/read-loan/write-loan quota rejection leaves backing
charges and source access unchanged. Prepared moves cancel at a full source
namespace, commit a two-object batch, reject a retired destination without
publishing either object, and cancel back to a retiring source's original slots.
Source teardown retains its frames until the move owner drops. Exact ASID and
numeric-capability reuse checks both late source and late destination failure
without changing successor records or budgets. These are deterministic
kernel fixtures, not exhaustive concurrent scheduling or a new EL0 quota test.

Copy/read-loan/write-loan fixtures pause preparation and attempt guessed-handle
lookup, mapping, writes, close, physical queries and DMA/copy pins. Every
destination is inaccessible. Mixed retirement rejects all four modes together,
returns private-copy charges and restores source slots at the ceiling. Exact
ASID/cap reuse tests copied/loaned source and destination teardown. Real kernel
vector calls check all-mode preparation failures, rejected copy backing after
a staged loan, and successful four-mode delivery. Mapped read/write loans are
revoked on reply, queued/delivered cancellation, reply-token close and queued
endpoint close; queued copies/moves are also reclaimed. Kernel buffer and DMA
access checks enforce committed loan permissions, not only mapping rights.

Actual mailbox syscalls and completion/timer submissions are rejected by the
shared ceiling with room in their family budgets; failed staging refunds those
family charges. Isolated production counter code tests node/ordinary ceilings,
platform headroom and rejection/refund at the enforced ceilings. It does not fill the
live node pool. These are kernel fixtures, not a new EL0 quota probe or an
exhaustive concurrent-retirement proof. The existing TLA+ serial-authority
model does not model these admission lifetimes.


## IPC connection delivery visibility

`PreparedConnection::install` marks queued and returned connection grants as
pending in their existing admitted entry under IPC. `IpcRegistry::cap` rejects
those entries before any send, call, mint, watch, supervisor-target resolution or
explicit close. No queued grant can mint a child that outlives cancellation.
Hidden entries still participate in endpoint-reference accounting and retain
original sponsorship until private queue/result/namespace cleanup removes them.
The cleanup lookup is private and unavailable to application operations.

Receive and first reply observation publish only the exact connection owned by
their queue/result receipt. A successfully delivered connection becomes usable
and can legitimately mint attenuated children that survive later call close.
Connection and memory reply source qualification follows ordinary visibility
lookup rather than visiting the global IPC registries for qualification.
Existing explicit source-close claims and root leases still protect delivered sources through split-phase loan cleanup.
See the [connection delivery report](../reports/audits/2026-10-06-security-connection-delivery.md).
