# Scheduler and completion timer-account backing

Scheduler sleeps/watchdogs and completion timers use separate generation-owned
accounts of the same fixed layout. Events and cancellation handles can retain
an account after its root or namespace has gone.

| Policy field | Ordinary | Total |
| --- | ---: | ---: |
| `timer_accounts` | 12288 | 16384 |

One slot covers an account Arc and its private `ChargedAllocator` holder.
`klib::account_backing` combines that typed family reservation with a captured
`SponsoredAllocator`. Both actual requested layouts, including Arc control and
alignment bytes, consume the principal's shared **1 MiB reactor-account allowance**
(`principal_reactor_account_bytes`). The allowance is independent of namespace
accounts and the 16 MiB working-buffer allowance. It uses the existing boot-lifetime
principal record, with no second identity registry or per-account heap owner.

Family reservation precedes either allocation; principal bytes are reserved at
each actual allocation boundary. Rejection rolls back unused reservations.
Actual deallocation precedes that allocation's byte refund; the family slot
returns only after both payload and private holder are freed. Weak references
and allocator clones retain their original funding. Allocator excess and external
reference fields remain outside this account's own byte charge.

The unchanged global ceilings leave twice the count-only complete-root needs
at 3072 ordinary/4096 total roots, with one quarter trusted reserve. These are
independent ceilings, not preallocated RAM or service entitlements. The separate
principal allowance bounds retained generations without consuming buffer funding
needed for inputs or cleanup. A larger allowance permits more retained memory;
a smaller one can reject mandatory root or namespace preparation. No qualified
workload requires increasing global limits or reducing logical limits.

## Captured sponsorship and release

Scheduler preparation captures the unpublished root’s prebound principal and
translation-storage class. Completion namespace preparation captures the exact
current generation, principal and class together under one root-table hold.
Capture precedes allocation and publication guards. A name, role, manifest field,
worker or later ASID lookup cannot select reserve or the refund recipient.
Unidentified permanent-kernel/raw fixture construction is a trusted exemption;
signed application roots use their prebound account. A copied funding token is
neither a root lease nor authority to publish or release physical backing.

Storage funding grants no platform privilege for timer events. Existing explicit,
generation-qualified resource grants remain separate. Promotion can change
future resource admission but cannot reclassify backing or redirect its refund.
Root close, namespace replacement and exact ASID reuse cannot return live bytes
or credit a successor. Final strong release cannot refund weak-only backing.

Root retirement closes scheduler accounts to new events. Completion timer
submission retains its existing captured-root validation; namespace disposal
cannot refund an independently retained event or cancellation allocation.

The existing complete root/namespace owners retain partial fields on abandonment.
Ordinary cancellation and final disposal occur outside their local publication
guards. Counter locks leave before allocation; the principal-history leaf lookup
never enters another subsystem. These mechanics qualify no arbitrary parent
or scheduler destructor and add no physical completion or retry authority.

## Qualification and remaining scope

[Reactor-account qualification](../reports/audits/2026-11-06-q4-reactor-principals.md)
records native actual layouts for all six account types, sibling sharing,
384 heap-held denials, independent healthy sponsorship, immutable promotion,
four real root and eight namespace rejection prefixes, and real bundle pressure
at the default principal byte ceiling while family count headroom remains.
Root rejection restores frames, roots and original charges. Namespace rejection
preserves established account identity and its CQ. Existing complete unpublished
root abandonment retains funded accounts under history, family and publication
locks without adding another deliberately retained object.

Real waiter admission, prepared timer/event/cancellation backing, completion weak
backing, a typed retired heap CQ and an owned watch entry retain their respective
original principals across root close and same-slot generation reuse. Final weak
release returns the old principal's bytes without changing the successor.
The heap CQ has no physical mapping or consumer; this is no physical CQ proof.
The [original family evidence](../reports/audits/evidence/2026-11-01-timer-account-admission.json)
records actual ordinary/total family exhaustion, trusted reserve, partial
publication rollback and guarded parent controls remain. Shared host allocation
controls cover holder/payload failure, allocator clones, actual free ordering,
alignment and concurrent final-weak release; native heap exhaustion is not forced.

The runtime pool inventory reports bundle counts, not principal byte usage.
Other account families, enclosing metadata, allocator excess and joint heap fit
remain separate Q4 work. Sustained hostile-pressure progress, arbitrary caller/IRQ
safety and physical hardware behavior remain unqualified. SEC-07 stays partial;
existing C03/C16/C18 and G1/G4 boundaries in [cleanup/recovery](cleanup-recovery.md)
remain applicable. See [namespace accounts](namespace-account-admission.md),
[observer handles](observer-ownership.md) and the
[principal coverage checklist](metadata-admission.md#principal-coverage-and-remaining-reconciliation).
