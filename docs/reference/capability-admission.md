# Shared capability-record admission

The kernel's unified namespace now accounts for every capability kind:
IPC, memory, completion, device, mailbox and system-observer authority.
Mailbox opens and capability-backed completion submissions (including timers,
event watches and workers), and all memory-object destinations enforce shared
admission in addition to their existing family limits. IPC, device and
system-observer allocation paths are counted but **not yet
limited by this policy**. SEC-07 remains partially implemented.

| Shared admission scope | Record limit |
| --- | ---: |
| One namespace | 4,096 |
| Node total | 65,536 |
| Ordinary records on the node | 49,152 |

The remaining 16,384 records are shared platform headroom for converted
allocation paths, not a per-service allowance. Kernel platform launch policy
sets the namespace's class against an exact address-space handle. A later
promotion affects future records; existing charges retain their original
class. Applications cannot select the class or override these limits.
Unconverted paths can exceed the limits and therefore can still exhaust this
headroom. These are not complete aggregate kernel-memory protections.

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

`PreparedMove` combines destination admission, source `MoveEscrow` and a
backing-retention pin. The payload remains with the source until commit, but
neither source nor staged destination grants application access. Preparation
rejects existing mappings, loans and DMA/copy pins. Drop restores the original
source handle without fresh quota, even at the namespace ceiling, cancels
destination admission and releases the pin. The scalar `rollback_move_to` and
`restore_unmigrated` APIs have been removed.

`commit_moves` validates every source and destination before publishing any of
the batch. It holds the memory registry across atomic capability publication
and the remaining payload updates. IPC vectors use this owner for moves; reply
memory is prepared before loan revocation and committed afterward. The
multi-state kernel upgrade helper also owns a prepared batch, but the actual
userspace upgrade syscall still accepts one state object.

If a namespace has been retired but its payload is not yet drained, cancellation
may restore its *existing* source authority for teardown; this admits no new
record. A removed or replaced namespace cannot be restored. If source teardown
has already removed its payload, the pin retains the frames until cancellation,
and the original sponsorship charge is released without debiting a successor.
Copies and loans enforce destination admission but are still published
individually during IPC vector preparation; atomic staging of those aliases is
a separate remaining migration.

## Lifecycle and locks

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

## Cutover, not compatibility

There is no requirement to retain old internal APIs or wire formats. The old
generic allocator/restorer names have been removed. Remaining allocation calls
use the deliberately explicit `allocate_unmigrated` name. Its temporary bypass
prevents a newly fallible quota check from occurring *after*
an unconverted operation has already moved ownership. It is not a compatibility
promise and must disappear as those payload transactions are replaced.

The next migration needs to:

- Stage endpoint, connection, call, device and system-observer identities before their
  payloads change; return normal resource errors on rejected admission.
- Preserve a queued receive and result page when reply-cap admission fails.
- Keep copied/loaned vector aliases hidden until the complete IPC transaction
  can publish, as move destinations already are. Move batches are atomic;
  the complete mixed-mode attachment transaction is not yet staged atomically.
- Remove the unconverted allocation helper after every caller has migrated.

These count limits do not charge allocator bytes, empty namespace/control
blocks, page tables, loader/heap backing or arbitrary callback captures.
`Arc` control-block preparation is fallible, but BTreeMap allocation remains
infallible. Count admission is not physical out-of-memory handling.

## Verification

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

Actual mailbox syscalls and completion/timer submissions are rejected by the
shared ceiling with room in their family budgets; failed staging refunds those
family charges. Isolated production counter code tests node/ordinary ceilings,
platform headroom and the explicitly unfinished bypass. It does not fill the
live node pool. These are kernel fixtures, not a new EL0 quota probe or an
exhaustive concurrent-retirement proof. The existing TLA+ serial-authority
model does not model these admission lifetimes.
