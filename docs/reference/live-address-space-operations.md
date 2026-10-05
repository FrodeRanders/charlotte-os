# Live address-space operation leases

`memory::operation::AddressSpaceOperation` retains an exact **live** software
generation. It differs from `RetiredAddressSpace`, which owns a detached root
through final invalidation and destruction.

Acquisition takes lifecycle before the address-space table, rejects kernel,
missing and stale handles, and increments an inline slot count. `SlotLease` is
a linear table/slot/generation token, not a pointer into moving table vectors.
There is no per-lease allocation; each generic slot gains one count. Overflow
rejects admission. Retirement preflight and ordinary extraction reject leases.
Root close returns `OperationsInFlight` before subsystem retirement or completion
metadata allocation: the root, hardware tag, accounts and namespace stay present.
Busy close has not fenced new operations or retired capability/backing admission.

Explicit consuming release verifies the original table/generation and decrements
its count under the table guard. It does not acquire lifecycle, allocate,
invalidate or destroy resources. Only after the last completion may normal root
close proceed. Wrong-identity and underflow completion reject without mutation.
Drop retains the count: abandonment leaves a live root whose close stays busy,
not a detached quarantined namespace. Generic table destruction likewise retains
leased payloads. There is no forced decrement or recovery bypass.

This protects checked retirement/extraction, not arbitrary trusted mutation via
`get_mut`/iterators. Callers must not replace/drop a leased root. Thread
quiescence is still required for actual close; copying a handle does not extend
the operation's lifetime after explicit release.

## Integration still required

Boot fixtures exercise real address-space leases. **Production mapping, IPC and
MMIO paths have not been migrated**, and their masking guards remain. The
supervisor currently assumes no outstanding leases at close. Before activating
split-phase operations, implement:

1. Lease admission before subsystem serialization, with backing, exact scratch
   reservation and loan/connection authority retained in one operation owner.
2. Translation identity capture after lazy root/tag preparation; then release
   preparation guards before rendezvous. Syscall entry's interrupt state and
   unrelated outer guards still matter for recipient progress.
3. Invalidation, scratch and authority completion before consuming the lease.
   Failure/abandonment must retain every uncertain resource, without rendezvous
   in Drop under unknown caller locks.
4. Closing-admission fencing and bounded/deferred busy-close handling outside
   lifecycle/IPC. Do not turn normal overlap into a supervisor kernel panic or
   confuse it with permanent abandoned retention.

SEC-18 remains partial, including recoverable shootdown and hardware quiescence.
This is not a new wire API, scheduler reference, capability right, metadata
budget or completed x86 progress fix.

## Verification

Twelve direct host slot-owner tests include five new live-lease tests:
overlapping counts, rejection before retirement allocation, exact reuse after
last completion, wrong table/generation, overflow/underflow, abandonment/table
destruction and growth to 2,047 entries without pointer-based identity.

Boot fixtures use a real root, heap backing and mapped object. Busy close
preserves frame/charge counts, ARM hardware tag, mappings and new admission.
Completion while lifecycle is held proves release does not re-enter that guard.
Detached and reused generations reject acquisition. Success cleans up normally;
abandonment deliberately retains one additional live root, its software slot/tag,
one heap-page charge and private table frames. Counts are logged. Serialized
fixtures and x86 compilation imply neither concurrent stress nor x86 IPI progress.

See [final root retirement](address-space-retirement.md),
[memory-object retirement](memory-object-retirement.md) and
[scratch admission](scratch-admission.md).
