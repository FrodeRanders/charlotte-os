# Shared capability-record admission

The kernel's unified namespace now accounts for every capability kind:
IPC, memory, completion, device, mailbox and system-observer authority.
Mailbox opens and capability-backed completion submissions (including timers,
event watches and workers) enforce shared admission in addition to their
existing family limits. The other allocation paths are counted but **not yet
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
| Escrow | No | `MoveEscrow::restore` restores the source; token Drop commits revocation. |

A rejected reservation consumes no serial. A successfully staged identity
remains consumed after cancellation; reusing it could revive stale authority.
Cancelled admission returns capacity immediately. Numeric handles are never
enough for token cleanup: tokens capture the exact namespace budget object.
Their publish, restore and Drop operations cannot alter a replacement's entry
even when both ASID and capability number are reused. Namespace teardown
releases entry charges, although retained tokens may keep the old, now empty,
budget control block alive.

`MoveEscrow` preserves the source's charged slot while its authority is hidden.
Restoration needs no fresh quota, even if the source namespace is full. It is
only an authority/accounting primitive: it does not move or roll back memory,
loans or other subsystem payloads. **Production memory/IPC moves have not yet
adopted it.** Their owning payload transactions must be converted together;
tests of escrow alone are not evidence that attachment rollback is migrated.
The current primitive refuses restoration after its account is retired or
replaced. Integration must explicitly handle retirement during rollback.

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
The permanent kernel namespace and kernel-only pseudo-domain fixtures use
`None`; that is not an application-selectable identity.

Counter ordering is namespace registry → domain counter → node counter.
Counter guards never enter a subsystem or allocate. Token Drop enters the
capability registry and therefore must not run while that registry is already
owned. Do not acquire lifecycle while holding a subsystem registry. See
[lock ordering](locking.md).

## Cutover, not compatibility

There is no requirement to retain old internal APIs or wire formats. The old
generic allocator/restorer names have been removed. Remaining calls use the
deliberately explicit `allocate_unmigrated`/`restore_unmigrated` names. Their
temporary bypass prevents a newly fallible quota check from occurring *after*
an unconverted operation has already moved ownership. It is not a compatibility
promise and must disappear as those payload transactions are replaced.

The next migration needs to:

- Stage endpoint, connection, call, device and memory identities before their
  payloads change; return normal resource errors on rejected admission.
- Preserve a queued receive and result page when reply-cap admission fails.
- Reserve all vector destinations before the first transfer; hold source
  escrow until commit and use owning reverse-order rollback.
- Replace scalar memory rollback with a consuming transaction API, including
  retirement, cancellation and partial-transfer failure.
- Remove both unconverted helpers after every caller has migrated.

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

Actual mailbox syscalls and completion/timer submissions are rejected by the
shared ceiling with room in their family budgets; failed staging refunds those
family charges. Isolated production counter code tests node/ordinary ceilings,
platform headroom and the explicitly unfinished bypass. It does not fill the
live node pool. These are kernel fixtures, not a new EL0 quota probe or an
exhaustive concurrent-retirement proof. The existing TLA+ serial-authority
model does not model these admission lifetimes.
