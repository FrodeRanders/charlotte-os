# Live address-space operation leases

`memory::operation::AddressSpaceOperation` retains an exact **live** software
generation. It differs from `RetiredAddressSpace`, which owns a detached root
through final invalidation and destruction.

Acquisition takes lifecycle before the address-space table, rejects kernel,
missing and stale handles, and increments an inline slot count. `SlotLease` is
a linear table/slot/generation token, not a pointer into moving table vectors.
There is no per-lease allocation; each generic slot stores a count and closing
flag. Overflow rejects admission. Retirement preflight and ordinary extraction
reject leases.
Root close returns `OperationsInFlight` before subsystem retirement or completion
metadata allocation: the root, hardware tag, accounts and namespace stay present.
Busy close has not fenced new operations or retired capability/backing admission.

Explicit consuming release verifies the original table/generation and decrements
its count under the table guard. It does not acquire lifecycle, allocate,
invalidate or destroy resources. Only after the last completion may normal root
close proceed. Wrong-identity and underflow completion reject without mutation.
Drop retains the count: abandonment leaves a live root whose close stays busy,
not a detached quarantined namespace. Generic table destruction likewise retains
leased or closing payloads. There is no forced decrement or recovery bypass.

This protects checked retirement/extraction, not arbitrary trusted mutation via
`get_mut`/iterators. Callers must not replace/drop a leased root. Thread
quiescence is still required for actual close; copying a handle does not extend
the operation's lifetime after explicit release.

## Owned staged close

`memory::retirement::ClosingAddressSpace::begin(handle)` requires the caller to
establish thread quiescence first. It validates the exact generation and prepares
free-slot completion storage before publishing a closing flag. Preparation
failure leaves admission unchanged. The request owns a linear `ClosingSlot`
bound to its table/slot/generation. The owner itself is inline; admission may
allocate shared completion storage.

Closing rejects new operation leases with `OperationError::Closing` and rejects
competing staged or immediate close with `CloseInProgress`. Older operations may
still explicitly complete. `poll(self)` returns `CloseProgress::Pending(self)`
while any lease remains, without subsystem retirement or invalidation. The
caller must retain the returned owner. When ready, poll refreshes completion
storage (the table may have grown during the unlocked interval), performs the
existing logical cleanup under lifecycle, then releases those guards before
final root invalidation/destruction. Immediate close remains a distinct
nonwaiting operation: if no staged fence exists, a busy result changes nothing.

`wait(self, timeout_ms)` polls and sleeps only after its own guards have gone.
Timeout returns `OperationDrainTimedOut`; dropping the request retains the
closing flag, root, tag, slot and accounts, even if the last operation later
completes. A ready request can complete with a zero waiting budget. Neither
timeout nor another close request can force recovery. Final invalidation can
still stall in the current x86 rendezvous; this timeout bounds the **lease-drain
polling interval**, not all teardown or hardware waiting.

The fence covers **operation-lease admission only**. Existing non-lease resource
paths retain their current guards and admission until logical cleanup. Begin
does not stop threads, revoke all capabilities or install a controller queue.
Callers must not hold unrelated masking guards across poll/wait; releasing the
owner's guards alone does not prove recipient progress.

## Integration still required

Public memory-object and MMIO map/map-any/unmap paths now acquire an operation
lease before registry access and retain it through scratch release and TLB
invalidation. They release it explicitly after the operation returns, including
ordinary failure; panic or abandonment retains the root. MMIO additionally
claims its device capability until invalidation finishes, so concurrent
`device_close` rejects without consuming it. Explicit device close also leases
the live root, detaches the device object under lifecycle/device serialization,
and releases lifecycle before MMIO invalidation. Direct loan revocation owns
leases for both owner and borrower alongside its revocation transaction. The
transaction fences backing and loan authority through detach, invalidation,
scratch completion and removal of the borrower capability. Ordinary failure
releases root leases but retains the transaction's backing pin and revocation
fence; abandonment of the whole operation also retains both root leases.
Plain borrowed-memory IPC replies compose both leases and loan receipts in
`PreparedReply`. A claim prevents concurrent reply/close from consuming records
while revocation runs outside IPC. Close waits without holding IPC; abandonment
retains claim, roots and backing. Ordinary cleanup failure completes root leases
but keeps uncertain loan backing fenced. No-loan scalar replies need no detached
interval and remain atomic under IPC.
Whole-domain device cleanup still holds lifecycle across invalidation. Before
extending split-phase operation leases, implement:

1. Extend IPC composition to returned memory/connection authority and
   cancellation's own revocation. Compose IPC leases with backing, exact scratch
   reservation and loan/connection
   authority in one operation owner. IPC must acquire lifecycle before IPC
   serialization, never from within an IPC guard. Reply/cancellation ownership
   must survive an IPC unlock together with the loan transaction. Plain replies
   implement that ownership; returned-authority replies/cancellation still retain
   their masking guard. Move
   whole-domain device cleanup invalidation out of lifecycle under an owned claim.
2. Translation identity capture after lazy root/tag preparation; then release
   preparation guards before rendezvous. Syscall entry's interrupt state and
   unrelated outer guards still matter for recipient progress.
3. Invalidation, scratch and authority completion before consuming the lease.
   Failure/abandonment must retain every uncertain resource, without rendezvous
   in Drop under unknown caller locks.
4. Extend supervisor lifecycle policy as needed. Deployment retirement and
   node/device shutdown now retain a `DomainTeardown` owner and poll outside
   registry/coordinator guards. The supervisor bounds thread/lease drain to five
   seconds; terminal reclamation error retains the deployment entry or prevents
   node poweroff. This provides no recovery path for abandoned close.

SEC-18 remains partial, including recoverable shootdown and hardware quiescence.
This is not a new wire API, scheduler reference, capability right, metadata
budget or completed x86 progress fix.

## Verification

Eighteen direct host slot-owner tests include five live-lease tests:
overlapping counts, rejection before retirement allocation, exact reuse after
last completion, wrong table/generation, overflow/underflow, abandonment/table
destruction and growth to 2,047 entries without pointer-based identity.
Six staged-close tests check admission fencing, existing completion, failed
preparation without a published fence, wrong identity, abandoned closing with
zero leases, completion-capacity refresh after growth, and fail-closed
detachment without prepared capacity.

Boot fixtures use a real root, heap backing and mapped object. Busy close
preserves frame/charge counts, ARM hardware tag, mappings and new admission.
Completion while lifecycle is held proves release does not re-enter that guard.
Detached and reused generations reject acquisition. Success cleans up normally;
abandonment deliberately retains one additional live root, its software slot/tag,
one heap-page charge and private table frames. Counts are logged. Serialized
fixtures and x86 compilation imply neither concurrent stress nor x86 IPI progress.

Staged-close boot fixtures check two pending polls, owner return, old-operation
completion under lifecycle, new-lease and competing-close rejection, retained
mapped backing before readiness, final post-guard cleanup and exact slot reuse.
A zero-budget wait checks ready success and pending timeout. Timeout deliberately
retains one additional closing root, one heap-page charge and its private table
frames, slot and ARM tag. Subsequent lease completion cannot reopen it. Nonzero
wait scheduling and concurrent close stress are not exercised by these fixtures.

See [final root retirement](address-space-retirement.md),
[memory-object retirement](memory-object-retirement.md) and
[scratch admission](scratch-admission.md).
