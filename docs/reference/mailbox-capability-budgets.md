# Mailbox capability-record budgets

Userspace LP mailboxes are domain-local. Their sender/receiver capabilities now
have independent record admission in `syscall/mailbox_budget.rs`:

| Scope | Maximum live or staged records |
| --- | ---: |
| One domain namespace | 512 |
| Node total | 8,192 |
| Ordinary domains on the node | 6,144 |

The remaining 2,048 node records are a shared progress reserve for
kernel-designated platform domains, not an allowance for each service. Reserve
classification is captured against the exact address-space generation; caller
names, roles and deployment fields cannot request it. Each record retains its
captured classification until destruction. Designating a domain as platform
later affects subsequent opens, not outstanding ordinary charges. There is no
application-facing quota setter or deployment override yet.

Node counters are statically initialized behind the IRQ-safe mutex, without a
first-use `LazyLock` dependency in the syscall path.

## Open, reuse and close

Both sender and receiver entries occupy one record. An open reserves admission
before minting its capability identity or publishing payload. Quota rejection
does not consume a serial. Serial exhaustion refunds the staged record and
returns failure without changing existing capabilities. A mailbox open returns
zero on failure, preserving its ABI; it does not identify which pool failed.

An existing per-LP receiver is returned even when the namespace is full,
because that lookup creates no new record. It is the **same capability**, not
a second independently owned resource. Applications need one canonical
receiver owner per LP; repeated lookup is not permission to adopt the same
handle twice. Closing an entry removes its unified authority and drops its
owning charge, making capacity reusable. Repeated open/close does not retain
record charges.

These limits concern handle records. Mailbox words still use the separate
256-entry queue per LP. Legacy word send/receive does not mint a capability
and is not charged by this record budget. This is not queue-backing memory
admission, bandwidth control or cross-domain IPC authority.

## Retirement and generation fencing

Mailbox opens capture their domain identity, then take
`ADDRESS_SPACE_LIFECYCLE → USER_MAILBOX_CAPS` to validate and publish. The
lifecycle guard serializes the entire open with production retirement/reuse;
an `accepting()` snapshot alone would leave a publication window. Ordinary
send/receive does not take that lifecycle guard. Opens across domains now
share this short metadata critical section; no throughput result is claimed.

The namespace stores its exact handle and owns a fresh reference-counted
budget. Teardown retires that budget before releasing entries. A captured old
open cannot recreate a retired namespace or publish into a replacement with
the same numeric ASID. A retained staged charge refunds only its original
account, never a replacement. Kernel-API pseudo-domain fixtures retain their
existing `None` identity convention; production user domains have real handles.

Budget locks follow registry, domain counter, then node counter. Charge Drop
enters only those independent counters. The budget account allocation is
fallible; BTreeMap backing allocation and empty namespace/control-block memory
are not fully charged or allocation-failure-safe by these count limits.

## Verification

Kernel fixtures call actual mailbox syscalls to fill 512 mixed sender/receiver
records, reject another sender, reuse a receiver at the ceiling, close/reopen
one slot and perform 1,024 open/close cycles. A high-bit-invalid LP is rejected
before narrowing. Serial-exhaustion injection tests staged refund and preserved
existing authority. Retained-charge fixtures reuse exact numeric capabilities
without crediting the replacement. Real address spaces test retirement before
and after registry removal, subsequent ASID recycling and late captured opens.
Ordinary/node saturation and platform reserve are checked with isolated
production counter code, not thousands of live allocations or mutation of the
shared node pool. This is kernel dispatch testing, not a new real-EL0 quota
probe or exhaustive concurrent-retirement exploration.

## Shared namespace admission and remaining migration

Mailbox opens also enforce the [shared capability budget](capability-admission.md):
4,096 records per namespace, 65,536 node-wide and 49,152 ordinary records.
Completion submissions and memory destinations use the same bounded path.
Every IPC capability publication does too;
receive rejection preserves queued work and vector result bytes.
Other families contribute
to those counters, but their unconverted allocation paths can still exceed the
shared policy. These limits therefore do not establish full aggregate protection.

Staged capability owners now capture exact namespace identity, and source
escrow preserves a rollback slot at capacity. Production memory moves, including
all four IPC vector modes, now use owning prepared transactions and atomic
mixed-mode publication. Copies stay private and loans have no live borrower
state during preparation. Completing the cutover still needs these contracts:

- Reserve destination identities/counts before mutating device or
  system-observer payload state. IPC now composes its call/grant/returned
  identities with memory attachments in the same atomic publication.
- Retain an exact namespace identity in staged owners. Their Drop must not
  revoke a replacement's reused numeric handle.
- Retire admission before payload teardown, and use the same trusted
  generation-aware platform classification for one aggregate node pool.

The old generic allocator API is not retained for compatibility. Explicitly
named unconverted helpers identify remaining transaction migrations and are
scheduled for removal, not for indefinite support.

Loader/page-table/heap accounting, queue backing and broader kernel metadata
remain separate requirements. SEC-07 remains partial.
