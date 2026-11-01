# Completion, CQ and watch account backing

Each completion namespace prepares three independent resource accounts: records,
CQ count/backing bytes and event watches. Their own fixed shared allocations
have backing admission, separate from the resource budgets they enforce and from
[timer-account backing](timer-account-admission.md).

| Policy field | Ordinary | Total |
| --- | ---: | ---: |
| `completion_accounts` | 18432 | 24576 |

One slot covers one fixed-layout account Arc and its private `ChargedAllocator`
holder. All three account types share this pool, while retaining separate typed
owners and resource counters. Reservation precedes either allocation; unused
allocation rejection returns the original charge after backing deallocation.
The pool bounds fixed allocation bundles, not exact heap bytes, nested resource
storage or allocator overhead.

The defaults allow twice the three-account needs at the root-metadata pool's
count-only capacity of 3072 ordinary or 4096 total complete roots. Extra slots
accommodate retained generations, namespace replacement and concurrent
preparation. A quarter remains trusted reserve. Sharing the pool avoids three
separate policies and leaves the headroom usable by any of the account types;
one type can consume that headroom, so this supplies no per-type or per-service
progress guarantee. No existing limit is raised and no RAM is preallocated.

The shared allocation owner adds one private holder allocation per account and
one machine word per account reference. A namespace's three inline account
owners consequently grow by 24 bytes on the qualified 64-bit targets. Resource
entry counts remain separate; admitted backing layouts use their actual sizes.
This does not admit the global namespace-map node allocated before the accounts,
or the wider kernel heap.

## Captured classification and actual lifetime

Namespace preparation captures the exact current root generation and trusted
translation-storage classification together, before allocating any account.
The permanent kernel is trusted; absent raw fixture IDs are ordinary. Names,
roles and caller-controlled manifest fields cannot select reserve. Publication
still checks captured namespace identity and root retirement; an account or
copied identity is not a root lease or authority to publish.

Backing admission grants no platform record, CQ or watch permission. Those
adapters keep their existing explicit, generation-qualified resource policy.
Later promotion changes future admission, never an existing backing charge's
classification or refund. The three account constructors have no uncharged
global-allocator alternative.

Records, queued/backlogged results, retired CQ backing and owned watch entries
retain their original account. Namespace close, replacement, root teardown and
ASID reuse cannot refund surviving backing or credit a successor. Even final
strong release leaves admission occupied while account weak references or
allocator clones retain allocation storage. The account and private holder must
both be deallocated before refund.

Root and namespace validation remain at their existing adapters; these counter
accounts do not independently authorize new work after retirement. Complete
unpublished/retired namespace fallback keeps every account and charge inert
under registry/heap/pool guards. Ordinary cancellation and final disposal retain
their existing post-guard requirements. The leaf pool only changes counters;
allocation follows after unlock. No new registry, cleanup family or retry owner
is introduced.

## Qualification and remaining scope

The [versioned evidence](../reports/audits/evidence/2026-11-01-completion-account-admission.json)
records real mixed-account pressure to ordinary and total ceilings, 192 heap-held
constructor denials and rejection of partial namespace replacement with zero,
one or two slots available. Original account identity, buffer, capability and
independent timer-account admission survive rejection. Trusted root preparation
uses backing reserve while actual queue, submission and observation adapters
still consume ordinary resource admission. Closing a promoted root at total
exhaustion returns its original three ordinary slots and permits fresh creation.

A separate control retains a real completion allocation, typed retired heap CQ
and independent owned watch entry across actual root close and exact ASID reuse.
It disposes those resources outside guards, then returns each backing slot only
at its final account weak release, leaving the successor untouched. The private
heap queue has no physical mapping or consumer; this supplies no physical CQ
retirement proof. Existing namespace controls add five-stage rollback, 64
heap/pool-held publications/removals, stale-generation weak retention and actual
unpublished/retired parent abandonment. Existing negative fixtures retain the
same objects; no additional deliberate leak is introduced.

The shared allocation owner's host controls cover holder/payload failure,
allocator clones, weak lifetime and deallocation-before-refund ordering. Native
heap exhaustion is not forced. Capability, IPC and mailbox accounts, global
namespace nodes, Waker/callback storage and aggregate principal/general-heap
admission remain open. No sustained hostile-pressure fairness, worst-case global
lookup/IRQ latency, arbitrary destructor safety or physical hardware claim is
made. SEC-07 remains partial; C18 and G1/G4 in
[cleanup/recovery](cleanup-recovery.md) remain applicable.
