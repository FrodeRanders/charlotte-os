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
handle twice. Explicit close detaches its payload and unified authority into
`RetiredMailbox`.
Post-guard completion releases both original charged nodes and then its exact
root, making capacity reusable. Repeated open/close does not retain
record charges.

These limits concern handle records. Mailbox words still use the separate
256-entry queue per LP. Legacy word send/receive does not mint a capability
and is not charged by this record budget. Word-ring backing has its own
independent dimensions in the same captured family account:

| Scope | Prepared/live queue sets | Requested ring backing bytes |
| --- | ---: | ---: |
| One exact domain generation | 2 | 1 MiB |
| Node total | 1,024 | 8 MiB |
| Ordinary domains on the node | 768 | 6 MiB |

Both dimensions must admit before allocation. The remaining 256 sets / 2 MiB
are a shared platform progress reserve. A published queue and every competing
preparation consume this same account; legacy words cannot create separate
quotas. Classification comes from the captured kernel designation, and existing
charges never change after promotion. These are requested allocation bytes;
allocator overhead, namespace/account nodes and aggregate principal/heap budgets
remain outside them. They do not implement bandwidth control or cross-domain IPC
authority.

## Retirement and generation fencing

Mailbox opens capture their domain identity and retain an exact
`AddressSpaceOperation`. `PreparingMailbox` fallibly prepares namespace/endpoint
nodes, an unused namespace budget and shared `PreparedReservation` before
`ADDRESS_SPACE_LIFECYCLE → USER_MAILBOX_CAPS`. Publication revalidates generation,
retirement and closing state, reserves both original charges and only relinks
existing `AdmittedMap` nodes. A staged close can drain this operation while
rejecting its publication. Existing receiver lookup needs no fresh storage.
Unused preparation and ordinary cancellation finish after local guards leave,
with the root completed last. No payload insertion can allocate after authority
publication; the old lifecycle-only allocator helper has been removed. Word
send/receive now also retain an exact root and use lifecycle before their shared
queue-registry borrow to validate identity, retirement and staged closing. No
throughput result is claimed.

The namespace stores its exact handle and owns a fresh reference-counted
budget. Teardown retires that budget before releasing entries. A captured old
open cannot recreate a retired namespace or publish into a replacement with
the same numeric ASID. A retained staged charge refunds only its original
account, never a replacement. Kernel-API pseudo-domain fixtures retain their
existing `None` identity convention; production user domains have real handles.

Budget locks follow registry, domain counter, then node counter. Charge Drop
enters only those independent counters. Namespace/endpoint storage and the
budget account allocation are now fallible.
`PreparingMailbox` and `RetiredMailbox` retain all fields on abandonment without
registry/counter/allocator access or logging. Retained charges and roots are
terminal retention, with no retry adapter. Empty namespace/control-block bytes
are outside the count ceilings. Final-root mailbox teardown now detaches the
complete admitted namespace and queue node/backing into its existing closing
transaction, then moves them with the unified authority namespace into `RetiredAddressSpace`.
Failed root invalidation retains both original record charges and queue backing.
Confirmed invalidation explicitly releases payload and authority metadata outside
local lifecycle/mailbox/capability guards, before physical root/slot completion.
There is no snapshot, independent retry or cleanup in fallback. The word-queue
namespace now uses the same admitted map: prepare its node fallibly and its queue
backing outside local guards, then revalidate and relink. A competing creator
leaves unused storage with its exact operation until post-guard completion, root
last. Final teardown detaches the complete node into the existing root receipt,
without map-node deallocation under the registry. Its captured generation fences
late send/receive against successors and staged close. The word API now uses
fixed 256-word rings behind existing IRQ-state-preserving per-LP mutexes. A single Vec is fallibly prepared with exact
capacity; requested layout bytes and one queue set are reserved first. No queue
Arc/Weak escapes. Ring allocation drops before its original charge; complete
preparation/root fallback retains both. A missing family account is prepared
fallibly outside guards and relinked before backing admission, leaving an empty
owned namespace after ordinary backing rejection. An exact shared account
prevents competing creators or legacy/capability paths from splitting quotas.
Principal, metadata and aggregate heap admission remain open. Raw serialized
teardown is confined to boot fixtures. See the [final-metadata evidence](../reports/audits/2026-10-10-security-root-metadata-retirement.md).

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

Additional serialized fixtures reject namespace node, endpoint node, budget
and shared record preparation before charge/serial mutation. Heap-held publication
and detachment prove those operations only relink admitted storage. Explicit
staged cancellation refunds both charges after guards; staged root close rejects
publication, then completes once the operation releases. Guarded abandonment
retains two exact roots, two mailbox charges and two shared authority charges
(including a detached record), plus their nodes/control blocks and CPU-root
backing. It creates no extra mailbox queues or data frames. The
[publication evidence](../reports/audits/2026-10-10-security-mailbox-publication.md) records failed runs separately from repeats.

Additional [word-queue fixtures](../reports/audits/2026-10-10-security-mailbox-queue-storage.md)
reject node preparation before registry mutation, preserve established sends
without allocation, retain losing creators through post-guard completion, and
verify two 256-word FIFO/backpressure/wrap rounds. Heap-held publication and
actual final metadata detach qualify allocation-free relinking. Retired, staged
and stale roots reject; successor content stays intact. Guarded abandonment
retains one additional root lease, unused admitted node and 256-word queue per
LP, without data frames or record charges. This is terminal retention, not
retry custody or real OOM/concurrency qualification. The
[backing-admission follow-up](../reports/audits/2026-10-10-security-mailbox-backing-admission.md)
adds backing rejection and layout overflow, actual shared generation ceilings,
isolated node byte/set saturation and platform reserve, post-detach refunds,
and queue charges in real root custody retry/abandonment/physical rejection.
Existing FIFO/wrap/ABI checks now exercise fixed rings. No extra retained root,
queue or data frame is introduced; previously retained queue owners keep their
new original backing charge.

## Shared namespace admission

Mailbox opens also enforce the [shared capability budget](capability-admission.md):
4,096 records per namespace, 65,536 node-wide and 49,152 ordinary records.
Completion submissions and memory destinations use the same bounded path.
Every IPC capability publication does too;
receive rejection preserves queued work and vector result bytes.
Device/system-observer grants are also converted. All six kinds enforce the
shared policy; count limits do not establish full aggregate byte protection.

Staged capability owners now capture exact namespace identity, and source
escrow preserves a rollback slot at capacity. Production memory moves, including
all four IPC vector modes, now use owning prepared transactions and atomic
mixed-mode publication. Copies stay private and loans have no live borrower
state during preparation. The completed cutover preserves these contracts:

- Reserve destination identities/counts before mutating device or
  system-observer payload state. IPC composes its call/grant/returned
  identities with memory attachments in the same atomic publication.
- Retain an exact namespace identity in staged owners. Their Drop must not
  revoke a replacement's reused numeric handle.
- Retire admission before payload teardown, and use the same trusted
  generation-aware platform classification for one aggregate node pool.

The old generic allocator API and temporary unconverted helper are removed,
with no compatibility alias or budget/retirement bypass.

Loader/page-table/heap accounting, queue backing and broader kernel metadata
remain separate requirements. SEC-07 remains partial.
