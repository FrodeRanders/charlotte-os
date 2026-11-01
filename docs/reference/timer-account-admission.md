# Scheduler and completion timer-account backing

Scheduler sleeps/watchdogs and completion timers use separate generation-owned
accounts of the same fixed layout. Each account has independent backing
admission in addition to the shared [event limits](scheduler-timer-budgets.md).
Entries, events and cancellation state can retain an account after its root or
completion namespace has gone.

| Policy field | Ordinary | Total |
| --- | ---: | ---: |
| `timer_accounts` | 12288 | 16384 |

One slot admits the account Arc and its private `ChargedAllocator` holder.
Reservation precedes either allocation; ordinary allocation rejection frees
unused backing before returning the original reservation. This counts fixed
allocation bundles, including weak-only storage, rather than exact bytes or
allocator overhead. Event and waiter admission remain separate.

A complete root can have one scheduler account and one current completion-timer
account. These defaults allow twice those two-account needs at the root-metadata
pool's count-only capacity of 3072 ordinary or 4096 total complete roots. Extra
capacity accommodates retained generations, namespace replacement and concurrent
preparation. One quarter remains a shared trusted reserve, without per-service
entitlements. No existing ceiling is raised and no RAM is preallocated.
Coupling accounts to active event counts would unnecessarily limit idle roots.

Reusing the allocation owner costs one private holder allocation per account
and one extra machine word in each account reference. Enclosing admitted layouts
reserve their actual larger size. Every fully prepared namespace has one fresh
timer account, so this also bounds complete namespace candidates and published
namespaces, including empty ones. Independently surviving record/CQ/watch accounts
have their own [backing admission](completion-account-admission.md). Global nodes
prepared before account admission have separate
[byte/count admission](namespace-node-admission.md). No worst-case
global lookup latency follows from this count.

## Classification and lifetime

Scheduler account preparation captures backing classification from the
unpublished translation root before registry publication. Completion namespace
preparation captures one current generation and that root's trusted translation
storage class together under the address-space table hold, before allocation.
The permanent kernel is trusted; absent raw fixture IDs are ordinary. Publication
still validates the captured generation and retirement state. The captured
identity supplies no lease or physical cleanup authority.

Backing admission grants no platform event privilege. Scheduler event admission
still follows the explicit platform grant; completion submission still validates
its captured platform identity. Promotion can change future event admission,
while existing backing and event charges retain their original classification.
No application role, name, manifest field or later ASID lookup selects reserve.

Retirement closes scheduler accounts to new events. Namespace disposal ends its
account ownership but cannot refund an independently retained event, cancellation
handle, allocator clone or weak allocation. The payload and private holder must
both be freed before the original backing slot returns. Root close, namespace
replacement, final strong release and exact ASID reuse cannot refund surviving
backing or credit a successor generation.

The existing root-account, unpublished-namespace and retired-namespace owners
retain complete fields on abandonment. Their guarded controls hold the actual
account pool as well as publication/heap guards. Ordinary cancellation and final
disposal retain their existing post-guard context requirements; this admission
does not qualify every active parent, timer or scheduler destructor. Counter
holds leave before allocation and never resolve an ASID or enter another lock.

## Qualification and remaining work

The [versioned evidence](../reports/audits/evidence/2026-11-01-timer-account-admission.json)
records real account allocations to both ceilings, heap-held admission denial,
trusted root/namespace preparation at ordinary exhaustion, failed root and
namespace rollback, immutable classification after promotion and exact recovery
after close at total exhaustion. Actual completion timer submission verifies
that trusted backing alone grants no event reserve privilege.

A real prepared event and cancellation handle retain the original scheduler
account across root close and exact ASID reuse. Node disposal leaves cancellation
backing charged; handle release ends event admission, while an account weak
reference retains backing admission until final weak release. Completion namespace
replacement separately exercises final weak retention of its own timer account.
Five namespace preparation failure stages return unused timer-account admission;
64 fresh publications/removals execute under real heap/account-pool guards.
Existing partial root preparation and actual namespace abandonment controls
retain their new backing charges without introducing another deliberate leak.

Holder/payload allocation errors are exercised by the shared allocation owner's
15 host tests, rather than forced native heap exhaustion. These fixtures do not
establish aggregate principal/general-heap accounting, sustained hostile-pressure
fairness, arbitrary destructor safety or physical hardware behavior. Capability,
IPC and mailbox accounts have separate [backing admission](namespace-account-admission.md).
Waker/callback storage and other registry metadata remain
open. SEC-07 remains partial; existing C03/C16/C18 and G1/G4 boundaries in
[cleanup/recovery](cleanup-recovery.md) continue to apply.
