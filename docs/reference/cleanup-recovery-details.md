# Cleanup qualification details

These detailed C16/C18 notes were moved without changing their wording from
the coverage map at `100965a0` on 2026-10-25. Line wrapping and paragraph
boundaries are editorial. They preserve qualifications, outstanding gaps and
links to execution evidence; they do not establish broader caller coverage.

Use the [coverage map](cleanup-recovery.md#receipt-coverage-map) for the current
owner and continuation summary. Add future evidence here or in a focused report;
keep the map concise and update its remaining gap when the contract changes.

## C16 thread retirement and stack pair

Owner anchors: [`ReapBatch` / thread retirement](../../crates/catten/src/cpu/scheduler/threads/mod.rs), [`RetirementList`](../../crates/catten/src/klib/collections/retirement_list.rs), [`Stacks`](../../crates/catten/src/memory/thread_stack/stacks.rs) and [`StackSlot`](../../crates/catten/src/memory/thread_stack.rs).

Both architectures use pinned IRQ-enabled owner-LP reapers and defer active contexts.
Physical pair release is explicit and one-shot; its kernel half now retains a
non-copyable `KernelStack` with exact base/pages and a terminal release fence, replacing
scalar teardown and the allocating guard map
([evidence](../reports/audits/2026-10-17-kernel-stack-ownership.md)).

Failed pairs retain the entire thread/context in their existing node and subsequent
scans never retry. Stack/context fallback has no physical cleanup; slot/root/charge
fallback retains original admission.

Ordinary constructor/publication/submission rejection explicitly releases after local
guards.

Complete `Thread` payload fallback now retains context, stack/root admission, waiter
state, exit subscriptions, sponsors and unused retirement storage without callbacks,
cancellation, usage refund or field destruction. Consuming metadata disposal requires
confirmed pair release and retains its node through typed `release_with`; guarded
kernel/user abandonment and post-guard success are qualified in the [metadata
follow-up](../reports/audits/2026-10-12-thread-metadata-retention.md).

The [first-dispatch correction](../reports/audits/2026-10-15-first-dispatch-delivery.md)
completes requested-thread staging and existing deferred delivery on the incoming stack
for fresh and resumed contexts, before register restoration.

Its negative control and native passes do not establish historical IPC/persistence
causality or physical-release qualification.

The [automatic capture
follow-up](../reports/audits/2026-10-15-automatic-failure-capture.md) retains all-LP
stacks/registers, full declared diagnostic arrays and immutable input identity for timed
ARM/x86 runs. Snapshot readiness, absence and debugger stop remain observations, never
exact exit, quiescence or permission to release/retry.

The [captured park-fixture
follow-up](../reports/audits/2026-10-15-captured-park-fixture-panic.md) preserves a
spontaneous ARM panic and forced-switch negative control: producer-LP delivery may
correctly see its publisher Ready after preemption.

The corrected fixture checks the exact generation, Ready/Running state, IRQ policy and
before-registration delivery; four forced Ready cases retain exact peer exit watches.

No production owner, retirement gate or historical timeout disposition changes.

The [exit-watch preparation
batch](../reports/audits/2026-10-15-thread-exit-watch-preparation.md) captures the exact
thread generation/source before fallible list/entry preparation, then revalidates
authority/generation and links without allocation under the thread table.

The same preparation retains losing metadata/weak references/original charges until
post-table disposal. Heap-held publication/rejection and forced TID reuse qualify this
narrow C16 context; completion/observer outer callers, inner growth-walker contexts,
heap/principal bounds and G1/G4/G7 remain open.

No physical retirement proof or retry policy changes. Outer constructor/syscall masks
and explicit callback contexts remain G1.

The [stack-growth execution
owner](../reports/audits/2026-10-15-stack-growth-execution-owner.md) shares the existing
execution fence and retains the original complete thread/context/root/stack/charge
through unlocked growth allocation, mapping and rollback. Exact
lifetime/root/slot/prefix handback precedes fence completion; abandonment retains both
fences. Inner walker and other outer contexts remain G1; no new physical retry/custody
proof.

The abort sweep owns its executor through mutual/remote requests, timer wake and peer
scanning, then masks exact local self-request/root completion/executor release.
Scheduled handoff now observes atomic current-LP mutex ownership rather than immediate
global availability; deterministic local/remote ownership and IRQ-preservation probes
qualify that narrow check.

The [IPC readiness matrix](../reports/audits/2026-10-12-ipc-close-readiness.md) retains
a failed remote lifecycle fixture separately: its handshake prepares the complete thread
outside masking, then masks the observing LP before peer publication and uses bounded
fixture-only spinning until acknowledgement, avoiding a local yielded/preempted task
blocking on the held lifecycle lock.

This changes no production wait or cleanup proof.

The bootstrap catalogue selects a bounded 32 KiB verification stack through the existing
per-domain admission account; signed deployment limits and retirement policy remain
unchanged.

See the [deployment ingress
follow-up](../reports/audits/2026-10-11-security-deployment-ingress.md) for its
guard-fault capture and validation.

No failed-pair recovery adapter; historical timeouts and broader operation contexts
remain unresolved; snapshots and global phase/shootdown counters are not completion
proofs; G1/G2/G7.

The [reaper readiness follow-up](../reports/audits/2026-10-12-reaper-readiness.md)
replaces one-millisecond idle polling with owned event-only waits on the existing fixed
LP heads. Staging detaches admitted waiters into the shared producer-LP transport; only
that LP claims at post-switch/yield/idle boundaries after local guards leave. Short
setup masks bind enqueue/claim to the captured LP, preserving outer IRQ state. Durable
post-registration readiness rechecks preserve the original off-stack/Arm ownership and
one-shot pair gates; failed/started pairs are nonready. Only recorded admission
rejection uses a 10 ms backoff.

This adds no custody or quiescence proof; broader admission-pressure progress and outer
contexts remain G1/G7.

The [timed completion
outcomes](../reports/audits/2026-10-12-completion-wait-outcomes.md) follow-up qualifies
exit-watch waiting through explicit admission/timeout outcomes and exact-generation
failure observations; it changes no exit producer or physical retirement proof.

The [Raft batch
validation](../reports/audits/2026-10-13-protocol-simplification-raft.md) corrects the
watchdog saturation fixture: the root-generation timer account is shared by kernel
threads, so a local mask cannot isolate its counter.

The still-unpublished driver gets a private trusted fixture sponsor before any event
capture, preserving its original root/stack and normal node admission. Production
sponsorship is unchanged; no live replacement or new retirement proof.

The failed paired run is retained separately.

The [scoped launch
validation](../reports/audits/2026-10-13-protocol-simplification-launch.md) corrects a
timer-event fixture that confused a 1 ms pre-admission timeout with admission rejection.

Its dedicated worker prepares a private trusted sponsor before publication;
exact-generation exit interest precedes dispatch.

No live sponsor swap or polling flag remains.

The bounded masked rejection probe changes no production wait outcome or cleanup
authority.  S06 removes unchecked CQ waits and their one-wake watchdog path;
source/watchdog cancellation now uses the existing exact-identity checked scheduler
contract, with explicit admission rejection preserved through the runtime ABI.

No exit, root, stack or physical completion proof changes; wider untimed allocation and
outer IRQ contexts remain G1/G7.

See the [checked-wait
evidence](../reports/audits/2026-10-13-protocol-simplification-cq-wait.md).  The
[park-boundary correction](../reports/audits/2026-10-14-park-deferred-delivery.md)
flushes existing producer-LP readiness delivery before guard-free Blocked publication,
preserving enclosing masks and absolute deadlines.

It adds no physical proof or retry authority; historical exit/persistence attribution
remains open.

The [first-dispatch correction](../reports/audits/2026-10-15-first-dispatch-delivery.md)
runs existing owner-LP staging and deferred metadata delivery on the incoming stack
before register restore, including fresh kernel/user entry. IRQs stay masked, local
guards leave before callbacks, and incoming FP/SIMD state restores afterward. Physical
release remains on the same pinned reaper with unchanged complete owners, charges and
terminal fences.

A selected-generation dispatch fixture tests delivery before worker entry and still
requires its exact exit watch; historical timeout/persistence attribution remains open.

The [selection follow-up](../reports/audits/2026-10-15-scheduler-selection-storage.md)
removes temporary load/candidate vectors and rejects started x86 migration until CPU
ownership is qualified. Selection under held heap/LP/master guards does not qualify
destination run-queue growth, aggregate metadata admission or historical timeout
causation; G1/G4/G7 remain.

The [inline-queue
follow-up](../reports/audits/2026-10-15-inline-scheduler-run-queues.md) removes run-ring
growth with exact membership in the existing complete thread payload. Migration rejects
before source removal; owner-LP abort unlinks before extraction and linked
stack/unstarted cleanup rejects without physical work. Queue abandonment retains
membership/charges; no new registry or retry owner.

Broader table/waiter/timer/heap contexts and G1/G4/G7 remain.

The [wait-publication
follow-up](../reports/audits/2026-10-15-scheduler-wait-publication.md) captures exact
generation/state/sponsor before registration outside local LP/table guards, remembers
early hints and qualifies source callbacks against the installed waker. Ready admission
carries the complete old subscription through guard exit before cancellation;
rejected/immediate-ready preparation disposes there too. Source/outer masks, registry
initialization, timer and general heap contexts remain separately qualified.

No root/physical proof, recovery owner or replay authority is added.

The [source-preparation
follow-up](../reports/audits/2026-10-15-waiter-source-preparation.md) additionally
prepares lazy waiter lists outside their slot mutex and IPC/CQ entries outside
registries. Exact original-list identity precedes readiness and allocation-free linking;
losing/ready/rejected storage survives to post-guard disposal with its original charge.

This qualifies subscription metadata only; source teardown and wider outer contexts
remain open.

The [completion callback/source
follow-up](../reports/audits/2026-10-15-completion-callback-preparation.md) captures
exact record/root/account before callback list/entry preparation outside registry/inner
guards.

Publication revalidates before terminal handling/linking; unused storage disposes before
immediate delivery.

Successful namespace/CQ replacement carries the entire detached payload beyond the
registry before destruction, retaining original charges. Authority/staging errors,
timer-specific producer cleanup and outer contexts remain separate; no physical or retry
proof is added.  The [terminal observer-disposal
follow-up](../reports/audits/2026-10-16-completion-terminal-disposal.md) retains
detached event/worker subscriptions through terminal/CQ publication, then disposes after
local registry/inner unlock. Installation carries replaced/rejected subscriptions across
both guards; the terminal wake uses its captured CQ source even if disposal replaces
that queue. Original record/watch charges and buffers remain owned;
timer/staging/authority metadata and outer producer/IRQ qualification remain open.

No physical receipt, new registry or replay owner.  The [CQ payload-preparation
follow-up](../reports/audits/2026-10-16-cq-payload-preparation.md) moves CQ
list/backlog/ring admission, allocation and partial-error disposal outside COMPLETIONS.
Captured root/account/capacity and retirement revalidate before serialized alias checks,
physical initialization and replacement. Whole private namespace preparation owns its CQ
map node before publication. Original charges remain with rejected owners through unlock
and public adapters dispose them as unstarted rollback; no public retry owner is
exposed. Fresh registry-map nodes, unified authority, timer cleanup and outer callers
remain open.

No physical pin, custody or replay authority.  The [detached timer-disposal
follow-up](../reports/audits/2026-10-16-detached-timer-disposal.md) moves the complete
detached timer-bearing record beyond COMPLETIONS before observer/cancellation disposal
and direct-result charge release. Only actual backlog retention consumes its original
charge. Exact-source notification follows disposal; an active timer queue callback keeps
its separate event charge.

This is terminal logical disposal with no retry/custody or physical receipt. Map
removal, backlog draining, construction/capability timer installation and outer
queue/IRQ contexts remain open.  The [detached preparation
follow-up](../reports/audits/2026-10-16-detached-submission-preparation.md) captures
exact root/accounts/capacity/classification and CQ source before detached record/event
admission and timer node/observer allocation outside COMPLETIONS. Borrowed publication
rechecks retirement and namespace/source identity before moving the complete record;
rejected owners dispose after unlock as unstarted rollback, without public retry
authority. Accepted nodes enqueue once; teardown beforehand retains original event
admission through cancelled insertion.

The [operation-ID follow-up](../reports/audits/2026-10-16-operation-id-exhaustion.md)
rejects permanently before new payload/authority reservation at checked boot-lifetime
exhaustion, excluding zero and the syscall failure sentinel.

It adds no cleanup/retry owner; existing operations remain usable and replacement cannot
reset identities.

The [capability-backed preparation
follow-up](../reports/audits/2026-10-16-capability-completion-preparation.md) moves
record/list/watch and authority storage, timer cancellation/node/producer construction
and ordinary rejected-owner disposal outside COMPLETIONS. Hidden reservation and final
borrowed batch publication revalidate exact root/accounts/capacity/retirement. One
complete owner explicitly finishes after local guards; abandonment retains all fields
without cleanup. Timers preserve their original absolute deadline and
teardown-before-enqueue cancellation.

This is unstarted rollback, not Pending, physical proof or public retry/custody.
Map/backlog metadata, close/terminal cleanup and outer contexts remain open.

The [timer identity
follow-up](../reports/audits/2026-10-16-timer-cancellation-identity.md) removes the
private wrapping cancellation selector. Eager removal borrows the retained state Arc and
matches exact object identity; stale/fired/unpublished handles cannot remove successors.
Original cancellation/backing charges and busy/remote policies remain; this is no
completion receipt or physical release authority.

The [backlog refund
follow-up](../reports/audits/2026-10-16-completion-backlog-refunds.md) carries delivered
charges past COMPLETIONS in two inline original-account/class owners, with checked
post-guard refunds before parking/notification.

Full FIFO mutation and slot updates stay serialized.

It also replaces the IPC fixture's registration-implies-Blocked assertion with bounded
exact-generation observation of both phases.

A second Intel capture identifies a heap/registry cycle from a heap-held CQ waiter
fixture after concurrent workers started; its controls now run in the serialized
pre-scheduler phase.

Both failed samples remain preserved separately. Production map allocation, wider
queue/outer guards and historical exit-timeout attribution remain open.

The [detached metadata
follow-up](../reports/audits/2026-10-16-detached-completion-metadata.md) now prepares
shared admitted nodes outside COMPLETIONS, relinks after exact root/account/CQ
validation and detaches without allocation/free. Direct receipts and backlogs retain the
same original node/charge; flush batches free nodes after unlock before checked class
refunds.

Complete namespace/preparation/terminal fallback retains every field; explicit ordinary
close/replacement drains nodes after local guards.

This is unstarted rollback or terminal retention, not retry custody, physical proof or a
root pin. Capability/CQ/global map metadata, authority revocation and outer contexts
remain open.

The [capability completion metadata
follow-up](../reports/audits/2026-10-16-capability-completion-metadata.md) prepares
payload nodes before hidden authority reservation and relinks after joint
validation/publication. Close, abort/event rollback and namespace teardown retain
detached authority in the same payload node, then free both after local completion
unlock while the charged Arc remains live.

Shared authority release retains its exact charge through actual node deallocation. All
three withdrawal paths remove their exact pending CQ row before returning a slot,
correcting terminal-abort backlog accumulation.

Complete fallback retains node/payload/authority/charges without cleanup; no new
custody, root pin or physical proof. CQ/global maps and wider timer/observer/caller
contexts remain open.

The [CQ metadata follow-up](../reports/audits/2026-10-16-cq-metadata.md) prepares one
admitted queue node with the complete payload before local registry guards.

Publication relinks without allocation; replacement detaches
node/source/ring/backlog/original charge together.

Explicit post-guard finish frees metadata before payload backing and charge; whole
namespace disposal drains these nodes too. Unpublished/retired fallback retains every
field.

This is unstarted rollback or terminal retention, never Pending, retry custody or
physical authority.

The global namespace map, aggregate admission and wider timer/observer/IRQ/caller
contexts remain open.

The [global completion namespace metadata
follow-up](../reports/audits/2026-10-16-completion-namespace-metadata.md) prepares its
admitted node and four accounts fallibly outside COMPLETIONS in the existing complete
namespace owner.

Publication relinks after captured validation and physical alias checks, with no later
admission after ring initialization. Close/replacement returns the entire
node/payload/source/authority/charge set; explicit finish frees global metadata before
payload/account disposal after local unlock. Unpublished/retired fallback retains all
fields. Runtime CQ setup returns typed allocation failure; mandatory kernel/demo/fixture
setup shares the same preparation. Aggregate namespace/account bounds, wider
timer/observer/IRQ/caller contexts and historical attribution remain open; this adds no
root pin, physical receipt, Pending/retry owner or custody.

The [timer queue-disposal
follow-up](../reports/audits/2026-10-16-timer-queue-disposal.md) returns complete
original event nodes and one inline keyed payload in a retaining receipt on filtering,
cancellation, purge, expiry and rejected enqueue. Scheduler, idle, enqueue/cancel and
both IRQ adapters leave the local queue borrow before explicit iterative disposal; node
backing frees before its outside admission. Receipt/queue abandonment retains every
remaining field without cleanup. Exact cancellation identity, comparator/diagnostic
updates, callback timing and enclosing IRQ policy remain.

This qualifies local disposal only: signal callbacks/waiter notification and
temporary-reference disposal remain under the queue; outer scheduler/producer guards,
IRQ execution and unpublished preparation remain G1/G4/G7. One private negative receipt
retains one event charge without counter resets.

No physical proof, Pending/retry custody or historical attribution.

The [timer callback-delivery
follow-up](../reports/audits/2026-10-16-timer-callback-delivery.md) captures a bounded
due prefix into one non-Send/non-Sync PendingDelivery retaining selected and purged
original owners.

After comparator reconciliation and queue unlock, signal callbacks, waiter notification,
temporary observer destruction and disposal run on the captured LP with IRQs masked. One
inline queue claim fences nested drains; reentrant enqueue waits for the next pass, and
selected cancellation rechecks shared state before signal.

Explicit handback reconciles the current queue. Abandonment retains all fields and its
fence; private guarded controls retain two event charges without abandoning a live LP
claim. IRQ execution, unrelated outer guards, unpublished preparation, final handles and
pressure/progress remain G1/G4/G7.

No physical proof, Pending/retry custody or historical attribution.

The [unpublished timer
owner](../reports/audits/2026-10-16-timer-unpublished-ownership.md) now retains a
completed private node on abandonment and explicitly consumes unused preparation after
local publication/registry guards leave. Sleep prepares before its own park mask;
timed-wait rejection/deadline and completion submission finish paths explicitly dispose
unused nodes.

The initial ARM/Intel IRQ-policy panic samples exposed a newly introduced
match-temporary guard crossing local mask release; a statement-bound registration result
corrects that ordering, with failed evidence preserved in the linked report.

Final handles, partial construction, waiter preparation and outer contexts remain
separate; no physical proof, replay or custody.

The [timer cancellation
handback](../reports/audits/2026-10-16-timer-cancellation-handback.md) retains the exact
handle plus removed node/source through enclosing queue/registry exit; explicit finish
disposes payloads before final reference release. Bare-handle and handback abandonment
retain their original charges without cleanup, flag fabrication or replay.

Ordinary unused watchdog handles release explicitly. Raw Arc/Weak finalization, partial
construction, completion containing destructors and outer contexts remain separate.

The [timer node constructor](../reports/audits/2026-10-16-timer-node-construction.md)
captures the already charged event/source and outside admission before allocating empty
storage. Rejection retains payload through allocator return; explicit unused
cancellation frees empty backing before event and admission. Inert phase abandonment
retains every field. Completion initializes the same layout/allocation once into
PreparedEvent; raw event/admission construction, raw references and enclosing contexts
remain separate.

The [event/holder construction
follow-up](../reports/audits/2026-10-16-timer-event-construction.md) captures original
source/callback before reservation and retains parent admission through shared
holder/cancellation preparation. Owning holder rejection cancels after callback return;
borrowed rejection retains exact charge for payload-first disposal.

Complete phase fallback retains all fields without refund or notification. Completed raw
events, published resource-bearing Arc payload destruction and broader outer contexts
remain separate.

The [shared Arc payload
follow-up](../reports/audits/2026-10-16-charged-arc-payload-construction.md) retains the
complete value, empty backing and original admission across allocation; rejection
disposes explicitly after callback return, private initialization is once-only, and
phase abandonment is inert. Outer factories and published/containing destructors remain
separate.

The [record-construction
follow-up](../reports/audits/2026-10-16-completion-record-construction.md) captures
buffer, operation and original charge before holder/list allocation. Rejection
explicitly disposes payload before admission; all three phase fallbacks retain every
field.

Shared Arc transfer preserves the original buffer/identity. Outer factories and
published/containing destruction remain separate.

The [initial completion admission
follow-up](../reports/audits/2026-10-16-completion-initial-admission.md) captures the
original buffer/target before ID and record/watch admission, retains any early record
charge, and borrows the complete transaction through record factories. Pre-record
rollback disposes buffer before admission; captured/record/watch abandonment retains all
fields.

No new family, custody or physical proof.

The [shared observer-entry construction
follow-up](../reports/audits/2026-10-16-observer-entry-construction.md) captures the
exact weak observer and original waiter/watch charge before empty Box allocation;
rejection cancels explicitly after allocator return, initialization is once-only, and
both partial phase fallbacks retain all fields. Completed entry/token/source and wider
containing contexts remain separate.

The [completion callback-owner
follow-up](../reports/audits/2026-10-16-completion-callback-ownership.md) captures exact
target/weak observer before watch/list/entry admission, borrows entry factories and
explicitly finishes ordinary construction/publication outcomes before ready
notification. All four phase fallbacks retain the complete captured record and original
fields; wider observation/token/source destruction remains separate.

The [exit-watch ownership
follow-up](../reports/audits/2026-10-16-thread-exit-watch-ownership.md) retains captured
candidate source, weak callback and original watch charge before list/entry allocation
in one inert owner.

The scheduler hook borrows it; ordinary outcomes explicitly finish after local guards,
candidate source last. Four phase fallbacks retain every field. Initial
capture/admission, generic published/source/token destruction and wider contexts remain
separate; no target/root lease, physical proof or retry custody.

The [waiter preparation ownership
follow-up](../reports/audits/2026-10-16-waiter-preparation-ownership.md) captures exact
source/sponsor/weak observer before waiter admission in one inert owner.

Ordinary preparation and publication outcomes explicitly finish after local guards;
transient entry/admission releases before source and sponsor last. Three phase fallbacks
retain every field through source closure and sponsor retirement. Initial source
capture, generic completed entry, published token/source/batch destruction and wider
contexts remain separate; no root/thread lease, physical proof or recovery custody.

The [completed observer-entry
follow-up](../reports/audits/2026-10-16-completed-observer-entry-ownership.md) makes
shared completed private preparation inert on abandonment and requires consuming finish
from direct registration and callback/exit-watch/waiter containing owners after local
guards.

Publication transfers the same entry once; emptied finish cannot refund it. Active
linked-entry/token/batch/source destruction and wider contexts remain separate; no
physical proof or recovery custody.

The [lazy-source owner
follow-up](../reports/audits/2026-10-16-lazy-waiter-source-ownership.md) captures
original sponsor/weak callback before candidate preparation in one inert transaction,
retaining selected winner and shared waiter child. Losing release stays post-slot and
before admission; ordinary results explicitly finish. Six handoff-stage fallbacks retain
complete fields, while published tokens keep their own entry authority. Inner list
factory, active source/token/batch destruction and wider contexts remain separate; no
root lease, physical proof or recovery custody.

The same stack reservation now owns the signed principal share for maximum
user-plus-kernel capacity.

Its fallback is inert; only unused cancellation or confirmed complete pair cleanup
refunds it. Growth pays no second data charge.

The exact root lease, original classification and all started-release fences are
unchanged; no new recovery family or custody is added.

See [table/stack
evidence](../reports/audits/2026-10-23-security-principal-structure.md). Exact
context/node layout bytes and bundle counts now reserve before either allocation, with
node/ordinary and shared signed-principal ceilings in the existing history record.

The charge follows this same node through staging/failure; confirmed payload/context
disposal and actual node deallocation precede refund. Empty preparation releases after
unused backing disposal; published fallback retains storage and charge. Signed
abandonment under history/pool/root/heap/frame guards takes no locks or refunds.
Source-root retirement and ASID reuse cannot redirect the late refund.

This adds no physical proof, retry custody, root/pin or family. Table capacity,
observer/weak allocations and wider outer contexts remain G1/G7.

See [metadata admission
evidence](../reports/audits/2026-10-23-security-thread-metadata.md).

A fresh Arm first-entry exact-exit wait returned false within its existing five-second
budget; the retained four-LP capture lacks the awaited tuple, so cause remains
unestablished. Failure-only exact-target and live/staged diagnostics add no
quiescence/retry proof.

See [capture](../reports/audits/2026-10-25-security-memory-capability-preparation.md).

## C18 publication rollback and detached memory

The [backing abandonment correction](../reports/audits/2026-10-26-security-backing-abandonment.md)
qualifies `ChargedFrames` fallback separately from its containing callers.
Charge-only, private and published owners retain complete fields without
allocator, registry, accounting, callback or logging work. Ordinary preparation
errors explicitly cancel after local guards; successful publication consumes the
same owner while retaining serialization. Last-pin and cancelled-copy release
are explicit, with the previous destination-metadata-before-copy-backing order.
The containing transfer destructor remains active; wider IRQ/guard contexts and
physical failure/recovery custody remain open. A native negative control restoring
the old Drop refunds an unstarted reservation and fails before held-lock tests.
Guarded data-owner abandonment retains two real pages, three page/object
reservations and complete metadata/authority through root retirement and reuse.

Owner anchors: [`PreparedTransfer`, `PreparedSource`, `ChargedFrames` and `RetiredMemory`](../../crates/catten/src/memory/object.rs), [`PreparedCall` / `PreparedConnection`](../../crates/catten/src/ipc/mod.rs), [`MemoryAttachments`](../../crates/catten/src/ipc/attachments.rs), [`PreparingDomain`](../../crates/catten/src/service/loader.rs), `PreparingNamespace` in [`capability`](../../crates/catten/src/capability.rs), [`PreparingRecord` / `RetiredRecord`](../../crates/catten/src/capability/record.rs), [`PreparingMailbox` / `RetiredMailbox`](../../crates/catten/src/syscall/mailbox_publication.rs), word-queue `Operation` in [`mailbox_queue`](../../crates/catten/src/syscall/mailbox_queue.rs), and `PreparedDmaDomain` in C13.  Bounded plain-value telemetry scratch in [`statistics`](../../crates/catten/src/syscall/statistics.rs).

Telemetry scratch prepares fallibly outside thread/usage guards under one 1 MiB request
limit.

Source growth rejects before output allocation; no prefix is published. Heap reads
retain the exact live root, and C02 final sampling borrows its closing owner. Aggregate
heap/principal and outer contexts remain open
([evidence](../reports/audits/2026-10-18-telemetry-snapshot-preparation.md)).
`PreparingMetadata` now prepares a principal-history candidate with the same unpublished
root. Signed identity installation links one combined stack/heap record for a new
principal; duplicates keep their unused candidate in the exact authority slot. C02/C03
retain that candidate through failed invalidation and release it explicitly after
confirmed invalidation and local guard exit. Published history stays boot-owned;
retirement updates existing counters without allocation. General heap/principal limits
and snapshot/context work remain open
([evidence](../reports/audits/2026-10-18-principal-history-preparation.md)).
`PreparingMetadata` now also prepares the signed-authority slot, initially empty and
granting no identity. Signed-loader installation mutates only the captured generation,
with no allocating insertion. C02/C03 retain the populated principal/roles/digest
through failed invalidation and release its node post-guard
([evidence](../reports/audits/2026-10-18-root-authority-preparation.md)); no new custody
or trust authority. `PreparingMetadata` now adds fallible launch-limit/usage nodes to
the same registration owner before local publication guards; exact-generation insertion
only relinks shared admitted nodes. Sealed cleanup retains both nodes through C02/C03
final invalidation and explicit post-guard release
([evidence](../reports/audits/2026-10-18-root-policy-preparation.md)); snapshot
allocation remains separate; principal history preparation is covered above.
Memory-budget `PreparingAccount` and containing `PreparingRegistration` now retain
fallible node/sponsor metadata with the unpublished root/capability namespace before
local publication guards; exact-generation publication is allocation-free for that
account, and last-charge/final-close node disposal follows ledger unlock
([evidence](../reports/audits/2026-10-18-memory-sponsor-preparation.md)). Outer guards,
aggregate heap/principal admission and policy/telemetry registries remain separate.
Prepared source rollback is exact; hidden destinations are not externally usable.
Unified `PreparingNamespace` now retains its fallible node/account; user preparation
precedes lifecycle/table/ASID publication, ordinary unused storage cancels post-guard,
and Drop retains both allocations.

Complete namespace detachment retains entries/original charges until explicit disposal
outside `CAPABILITIES`; authority records now use the same fallibly prepared nodes.

Preparation owns its original charge through rejection; detached record nodes retain
charges until explicit post-capability-unlock disposal. Batch moves keep source
retirement in the exact escrow/transfer through payload completion and memory-registry
unlock. New preparation/retirement Drop is inert. Mailbox `PreparingMailbox` now joins
exact-root, shared preparation, budget and payload nodes before local publication
guards; explicit `RetiredMailbox` retains both charged nodes until post-guard
release/root completion.

Both fallbacks retain all fields. Word-queue operations capture the exact root before
registry access, prepare admitted nodes/backing outside guards, retain losing
preparation through post-guard completion and fence stale/retired/staged identities.
Queue preparation now shares the exact mailbox family account and reserves set/requested
byte dimensions before fallible fixed-ring allocation; original classification/charge
survives deallocation or retaining fallback.

Final mailbox/queue/authority metadata now stays in C02/C03 through confirmed
invalidation and post-guard release; queue/account metadata and principal admission,
completion/accounting teardown, other active token Drop and outer contexts remain G1/G4.

Detached `RetiredMemory` Drop quarantines; its consuming release has no physical retry.
All MMIO/IRQ/DMA grants share retaining `GrantAdmission` for exact-root/reservation,
fallibly prepared namespace/payload nodes and unified `PreparedReservation` storage
before local guards. Captured generation/closing/memory-budget admission checks precede
reservation; DMA retains its typed backend obligation beside it.

Publication only relinks admitted device storage, and unused-node/reservation disposal
leaves local guards.

DMA grant preparation retains every dependency without implicit field destruction.
Backends borrow its `DmaCreation` and record the admitted domain before reachable
descriptor publication, including errors.

Publication or published-creation rejection uses one-shot explicit rollback outside
local lifecycle/device guards; failure retains all three. Creation releases lifecycle
after exact admission and rechecks captured generation/closing state under lifecycle
before publication; its complete-unit claim leaves backend holds through
reset/construction/configuration.

The same containing grant now retains its exact endpoint reset claim before writes,
through busy capability publication; activation/cancellation are explicit and hardware
uncertainty/abandonment retains every dependency. Confirmed backend cleanup precedes
reset cancellation. Other preparation destructors can acquire registries, release
backing or invoke whole-domain teardown.

Private domain roots/CD/MSI prefixes and metadata now stay in `DmaCreation` through
rejection; one-shot private release/disposal precedes grant refund/root completion
outside local guards. Unit boot preparation now owns its existing slot claim and
complete payload through unlocked cancellation or terminal retention; complete-unit
creation now releases local lifecycle/backend holds through domain construction and
initial creation/reset waits; typed backend registry preparation now retains partial
admitted nodes in that same grant before reset/IDs/backing, with explicit post-guard
disposal only after confirmed rollback or publication; unused/detached mapping nodes now
stay with the exact pending pin in that same maintenance owner, and SMMU unused cache
storage stays in its domain until table completion.

Broader ledger/heap metadata and inner fallback remain separate; copy submission outside
IPC and this scoped grant correction are not universal destructor/context proofs; G1/G4.
Userspace TCP/IP receive/status now retains a caller-supplied buffer, received request
and transient mapping in `PendingReceive`; mapping disposal precedes reply/loan
cancellation, and existing returned-memory transfer retains the original charge.
Missing/oversized/read-only rejection, explicit cancellation and retained-sponsor
lifetime have targeted evidence in the [caller-buffer
follow-up](../reports/audits/2026-10-10-security-tcpip-caller-buffers.md); this does not
qualify new outer kernel guards or add custody.

The same userspace `OwnedSocket` now retains its active HTTP listener through queue-full
accept rejection and bounds close-admission retries before request publication; accepted
close is not replayed by that loop.

Explicit close reporting and best-effort fallback remain, with no general release
guarantee under indefinite saturation.

See the [HTTP progress
follow-up](../reports/audits/2026-10-10-security-http-progress.md) for failed captures
and passing pressure evidence.

The [deployment ingress
follow-up](../reports/audits/2026-10-11-security-deployment-ingress.md) extends this
same userspace protocol-owner boundary: shared borrowed listener setup consumes request
backing, and fallible `RequestBuffer` retains/reuses one local request allocation.
Rejection drops that owner before a static error response.

Shared identity initialization now owns its object-store connection and moved memory
requests, borrows launch authority, and treats only successful creation as permission to
write; losing creators retain no foreign write authority. Read-only publication
observation owns a transient receive/mapping/close-watch completion and returns scalar
metadata after their cleanup; the registry connection is borrowed at its existing ABI
adapter. Scoped lifecycle ownership stays in the agent; observation adds neither a root
lease nor scoped close-watch custody. Physical heap backing still follows the existing
root account.

No new custody or kernel guard qualification.

The same userspace publication boundary now borrows the exact NIC/router endpoint
through bounded queue-full delegated-call admission and one observation. Rejection
creates no pending owner; accepted calls are not replayed and returned attachments
retain typed cleanup.

Existing DMA/IRQ backing and broader guard/field-destructor contexts remain separate; no
new custody.

See the [network publication
follow-up](../reports/audits/2026-10-12-security-network-publication.md).

The [deadline follow-up](../reports/audits/2026-10-12-security-publication-deadlines.md)
extends the same endpoint-borrowing transaction to TCP/IP, time, cluster control,
reliable messaging and discovery.

The latter two now retain typed endpoint owners through their legacy borrowed reactors.
Accepted calls have bounded observation and cancel through C08 on timeout; remote
publication is uncertain, so there is no replay. Endpoint Drop and outer shutdown/IRQ
contexts keep their existing qualification limits. Deployment HTTP now shares
`clusterctl_client` for seven controller RPCs, retaining the exact pending owner until
result or deadline cancellation. Admission failure disposes the returned moved request
through its owner; accepted result attachments remain owned through decode or rejection.
[Controller deadline
evidence](../reports/audits/2026-10-12-security-controller-deadlines.md) checks exact
one-page original-sponsor recovery at saturation after rejection, unreceived
cancellation and returned-page disposal, plus complete queue recovery.

This adds no registry, custody family or uncertain physical completion proof.

The [object-store readiness/mount
follow-up](../reports/audits/2026-10-12-security-object-store-readiness.md) preserves
the same typed block connection, borrowed-page pending call and transient mapping
through metadata observation. Unreadable superblocks abort mount before formatting; no
accepted I/O is retried or reconstructed from diagnostics. Invalid returned attachments
receive their existing typed cleanup. Serving publication and first-failure records add
no root/physical custody; broader guard/Drop qualification stays unchanged.

The [reply CQ follow-up](../reports/audits/2026-10-12-reply-cq-readiness.md) prepares
observer/subscription storage outside IPC and revalidates the exact call before
installing it in that admitted record. Captured CQ list identity rejects successor
wakes; no result/attachment publication occurs at readiness.

Broader implicit metadata field destruction under IPC remains G1/G4.

The [transfer-close follow-up](../reports/audits/2026-10-12-memory-close-readiness.md)
now leases the exact prepared source root and parks on a charged transfer interest
instead of yielding. Fallible source/list-disposal storage precedes registry
publication, with exact object/fence revalidation and closed-source recheck.

Source completion queues admitted notifications and the owning list node in shared
producer-LP metadata transport; another LP cannot consume it beneath the producer IPC
guard. Source/close fallback retains complete fields, ordinary completion releases the
close lease, and wait rejection returns ResourceLimit without authority mutation.

The same transport delivers C16 readiness; it owns no backing/root transaction or retry
authority. Pin/loan rejection and namespace drains remain separate; G1/G4/G7.

The [driver startup
preparation](../reports/audits/2026-10-12-dma-startup-preparation.md) retains each
loaded driver root and lease in its existing launch/publication family until all boot
hardware grants prepare and thread start consumes it. Start finishes the lease after
publication; fallback retains fields. Typed creation diagnostics retain the original
cause across confirmed or failed cancellation; an error is not a replay owner.

The boot ordering excludes sibling-driven unit contention before grant admission,
without adding waits, registries or physical completion proofs.

The [DMA unit readiness](../reports/audits/2026-10-12-dma-unit-readiness.md) follow-up
adds an explicit scheduled grant path that owns its transient interest beside this same
preparation, prepares charged source/disposal nodes before backend serialization, parks
without a timer, and rechecks before complete-unit extraction. Masked/nonscheduled and
admission-rejected waits return a typed error through ordinary unarmed cancellation;
fallback retains every field. Fresh availability never replays armed creation, retries
uncertain cleanup or force-clears an abandoned slot.

The [DMA unmap phase correction](../reports/audits/2026-10-12-dma-unmap-phases.md)
records userspace unmap progress before kernel entry in the existing
`DmaTransfer`/`SharedDmaMemory` owners.

Explicit pre-work busy rejection permits another submission; ambiguous failure retains
its status and memory owner without another `finish`/Drop DMA call. Confirmed DMA
completion survives later CPU-unmap failure.

Existing best-effort local cleanup cannot discharge the kernel's uncertain pin/backing
owner.

No new custody or completion proof is added.

The [grant cancellation
readiness](../reports/audits/2026-10-12-dma-grant-cancellation-readiness.md) follow-up
composes creation and pre-destruction availability through one shared admission helper
and the same complete grant. Rollback-started stays set through its wait and one actual
claim; waiter failure, interruption and hardware/physical rejection return the entire
nonretryable owner. Closed-source observation cannot refund its root/reservation or
reconstruct a domain.

The existing failed-grant fixture now covers rejected admission and retained transient
interest without adding an abandoned root; ready-path real staged-close cancellation
completes its original root.

Broader preparation/Drop contexts and map/unmap/close contention policy remain open.

The [DMA-map failure
ownership](../reports/audits/2026-10-12-dma-map-failure-ownership.md) follow-up retains
failed shared-DMA CPU mappings behind a restricted runtime owner without
slices/pointers; explicit CPU unmap and best-effort Drop cannot discharge uncertain
kernel pins or original backing charges.

The [timed completion
outcomes](../reports/audits/2026-10-12-completion-wait-outcomes.md) follow-up retains
the same completion/buffer through checked admission rejection and unrelated wakes, with
a shared absolute deadline and no result consumption before terminal readiness.

The [scoped launch
receipt](../reports/audits/2026-10-13-protocol-simplification-launch.md) now supplies
the kernel-verified principal directly to the existing `DeployedArtifact` owner.

Its explicit completion and best-effort force fallback keep that identity; zero cannot
select a lone registry entry. Input transfer and kernel/root cleanup obligations are
unchanged, with no new physical retry authority.

The execution-target kernel fixture's `call`/`call_with_memory` adapters now consume
unused observed reply caps and complete their calls outside IPC; this qualifies only its
moved-memory, no-loan ABI adapter, with existing original-charge/retention policy
unchanged. S04b2a removes the descriptor-less launch branch rather than weakening its
rollback requirements. Controller status/rollout and unknown-operation rejection adopt
unique request/reply attachments at the raw receive boundary, use consuming owned moves,
and retain returned memory through validation; raw fixture reply cleanup remains outside
IPC. Other receive handlers and their enclosing contexts remain separate qualification
work; no new custody or physical retry authority. S04b2b removes replicated diagnostics
while preserving the same owned request/result and exact variant pins.

Its initial x86 NIC startup returned backend OPERATION_IN_FLIGHT during DMA mapping and
never reached router readiness; the existing retaining map-failure policy remains; the
passing guest repeat does not qualify public DMA-map readiness or identify the
contending owner.

See the [record-batch
evidence](../reports/audits/2026-10-13-protocol-simplification-records.md).  S06
ordinary service reactors and timer-backed admission pauses abort on checked wait/timer
rejection instead of spinning or advancing protocol time; pending transactions and
borrowed buffers remain governed by the existing domain-retirement owners. Sitas
sleep/reactor budgets retain their original monotonic deadlines; its shared-CQ parker
backstop remains explicitly unqualified as per-waiter readiness.

No new custody, cleanup receipt or retry authority; see the [checked-wait
evidence](../reports/audits/2026-10-13-protocol-simplification-cq-wait.md). S08b1
removes the S3 manifest fallback and duplicate validator while retaining the existing
read-only launch-profile borrow and zeroizing secret owner. Registration metadata now
carries its generation explicitly; readiness/activation and attachment ownership are
unchanged.

This adds no custody, cleanup receipt or physical retry authority; see
[registration/profile
evidence](../reports/audits/2026-10-13-protocol-simplification-registration-profiles.md).
S08b2a replaces Kafka authentication sections with fixed borrowed credential fields,
removing the temporary secret-bearing allocation while retaining the same zeroizing
final launch buffer, read-only profile mapping and connector secret owners. Rejection
creates no broker transport; no new custody, cleanup receipt or physical retry
authority.

See [Kafka profile
evidence](../reports/audits/2026-10-13-protocol-simplification-kafka-profile.md). S08b2b
consumes catalog requests once at the raw receive boundary into IncomingMessage before
the scalar attachment guard. CatalogReply contains the existing ReplyToken, encoded
bytes and typed OwnedMemory/mapping state for complete bounded output through
allocation, mapping, unmap and consuming move; rejected moves return their owner to
existing Drop. Request rejection releases unique attachments; no scalar catalog cleanup
ladder remains. Catalog encoding borrows the existing userspace registries and prepares
one fallible buffer, without cloned-entry snapshots; their mutex context is unchanged.
HTTP resolves one fresh owned DNS connection per requested node/cluster observation and
keeps its existing pending reply/mapping through exact-length and whole-set validation,
with no clamped prefix or invented empty table.

Existing runtime fallbacks and broader guard/physical qualifications remain; no custody
or retry authority.

See [catalog observation
evidence](../reports/audits/2026-10-13-protocol-simplification-dns-catalog.md). S08b2c1
makes correlated-query identity copies/output preparation fallible and exact, before
existing pending-query publication.

The original name moves into that same pending record after byte preparation, with its
deadline captured beforehand.

No accepted invocation is replayed, no closeable authority is created by the codec, and
rejected reply preparation leaves the original caller query/deadline intact.

Existing raw reactor reply/attachment contexts, metadata publication/destruction and
readiness subscriptions remain separately qualified.

No custody, registry, cleanup receipt or physical retry authority.

See [query-relay
evidence](../reports/audits/2026-10-14-protocol-simplification-dns-query-relay.md).
S08b2c2 retains a fallibly prepared fixed invocation reply buffer in the same remote
local-call destination before admission.

Ordinary rejection returns the entire destination; terminal progress consumes its
original buffer without result-time codec allocation. Direct request preparation
precedes in-flight publication with the captured deadline; query-routed invocation
retains its original query deadline and rejects expired routing before admission.

The scalar local lookup now rejects unrepresentable names before submission instead of
truncating; its full-name owning replacement remains pending.
Source/session/peer/generation and dedup ACK fences remain unchanged. Transport/dedup
metadata, raw completion/connection/attachment cleanup and broader guard/Drop contexts
remain separate; no custody, registry, physical completion or retry authority.

Both S08b2c2 final-source linked pairs failed during x86 NIC startup with backend
DMA-map OPERATION_IN_FLIGHT; their failed captures remain separately recorded.

The map-readiness follow-up now verifies unchanged final DNS bytes in two passing linked
pairs, including real ordinary/exclusive map contention controls. Exact contention
ownership and public map readiness are unqualified; diagnostics permit no replay.

See [invocation-relay
evidence](../reports/audits/2026-10-14-protocol-simplification-dns-invocation-relay.md).

The [DMA-map readiness follow-up](../reports/audits/2026-10-14-dma-map-readiness.md)
extends the existing public DmaOperation: original pin, captured root classification and
transient interest stay inside it before source preparation outside local guards.

Shared admission passes that same owner to its once-only backend claim, returning
complete MappingMaintenance before guard exit.

Ordinary unused preparation releases explicitly; pre-claim Drop retains
root/claim/pin/interest without locks, allocation, callbacks or physical cleanup.
Syscalls opt into qualified scheduled admission while serialized kernel adapters remain
rejecting. Wider IRQ/metadata contexts, masked SVC progress, unmap/close and historical
contention attribution remain open; no new custody/registry or replay authority. S08b2c3
keeps registration connections and reply authority in the existing
Prepare/Activate/RemoteRegister phases as Connection/ReplyToken owners, transferring
connections into LocalPublication without raw re-adoption.

Preparation rejection and deferred abort drop the same owners; full-name owned
lookup/registration retains returned resources through CallResult disposal. Follower
request and pending storage prepare before transport publication; the leader retains a
prepared terminal reply through both Raft phases. Replies match captured
peer/session/nonreused request, while retraction still carries exact owner/generation.
Watch completions, transport/catalog allocation, other raw adapters and wider Drop/IRQ
contexts remain separate.

The second linked run exposed an unresolved scheduled IPC endpoint-close exact-exit
timeout, followed by Raft persistence errors.

The probe now captures exact-generation live/staged state on timeout without widening
its budget or permitting cleanup; passing HTTP repeats do not establish its cause.

The [IPC exit
investigation](../reports/audits/2026-10-14-ipc-close-exit-investigation.md) adds
bounded repeated same/cross-LP real close episodes and retains the exact admitted claim
Interest through exit observation for source/phase diagnosis. Eight rounds across up to
four LPs require the original exact watch and busy-root rejection; 192 close/exit
episodes passed across two mixed pairs. Those initial passes did not reproduce the
timeout; the subsequent NS generation batch reproduced it on both architectures. Later
park/first-dispatch controls establish distinct ordering corrections, not historical
attribution or physical stack/backing release.

No new custody, registry, wait source, physical proof or retry authority; see
[registration-relay
evidence](../reports/audits/2026-10-14-protocol-simplification-dns-registration-relay.md).

The [owned local DNS invocation](dns-local-invocation.md) replaces scalar lookup/invoke
stages with `LocalInvocation`, typed client reply authority and unique incoming
attachment ownership.

Both accepted RPC phases bind readiness to the same CQ and retain the original
kernel-monotonic deadline; post-accept binding rejection/timeout is uncertain, never
replayable. Original pending/connection owners survive until confirmed observation or
runtime cancellation/close policy; this grants no physical or custody proof. Raw
receive/transport/watch contexts and broader metadata admission remain gaps.

The [NS generation replacement](name-service-generations.md) now carries exact full
names and zero/positive generations in one mandatory record. NS consumes unique incoming
attachments and retains registrar/manager checks; DNS cleanup owns CQ-bound pending
calls and one original deadline, without replay. Storage/preparation/submission
rejection fails closed with the containing transaction; binding rejection retains the
accepted owner through abort.

Explicit observation rejects before catalog mutation on admission/wait failure.
Best-effort NS connection close remains logical teardown only; raw
registry/watch/transport contexts and physical proof remain separate.  The
[park-boundary correction](../reports/audits/2026-10-14-park-deferred-delivery.md)
flushes existing producer-LP readiness delivery before guard-free Blocked publication,
preserving enclosing masks and absolute deadlines.

It adds no physical proof or retry authority; historical exit/persistence attribution
remains open.  The [NS endpoint-death
producer](../reports/audits/2026-10-14-ns-endpoint-death.md) retains each source
connection and persistent watch in one publication, admits before mutation, and
logically retires only on its confirmed exact close result. Shared-CQ intake is
event-only and preserves principal/generation checks. Watch cancellation precedes source
disposal; this is no physical cleanup proof or retry owner.  The [NS bearer
removal](../reports/audits/2026-10-15-ns-bearer-removal.md) rejects retired/unknown
opcodes through one owning incoming transaction instead of scalar attachment cleanup; no
body/key fallback or authority is retained. Public policy denial and exact
publication/watch owners remain.

The [NS status replacement](name-service-status.md) extends C18 through one
incoming/status reply owner, retaining bytes, memory/mapping and reply authority through
complete bounded preparation and consuming publication.

Ordinary preparation/move rejection uses existing runtime disposal without replay; no
new physical proof, custody or outer-context qualification.

See [execution evidence](../reports/audits/2026-10-15-ns-status-snapshot.md).

Shared record/list/timer charge-holder construction now retains original admission
outside empty Arc storage, initializes privately once and cancels ordinary rejection
after allocator return; inert phase fallback retains all fields.

This qualifies that primitive, not containing publication/destructor contexts or
physical cleanup; see the [event/holder
report](../reports/audits/2026-10-16-timer-event-construction.md).

The [shared Arc payload
follow-up](../reports/audits/2026-10-16-charged-arc-payload-construction.md) retains the
complete value, empty backing and original admission across allocation; rejection
disposes explicitly after callback return, private initialization is once-only, and
phase abandonment is inert. Outer factories and published/containing destructors remain
separate.

The [record-construction
follow-up](../reports/audits/2026-10-16-completion-record-construction.md) captures
buffer, operation and original charge before holder/list allocation. Rejection
explicitly disposes payload before admission; all three phase fallbacks retain every
field.

Shared Arc transfer preserves the original buffer/identity. Outer factories and
published/containing destruction remain separate.

The [initial completion admission
follow-up](../reports/audits/2026-10-16-completion-initial-admission.md) captures the
original buffer/target before ID and record/watch admission, retains any early record
charge, and borrows the complete transaction through record factories. Pre-record
rollback disposes buffer before admission; captured/record/watch abandonment retains all
fields.

No new family, custody or physical proof.

The [shared observer-entry construction
follow-up](../reports/audits/2026-10-16-observer-entry-construction.md) captures the
exact weak observer and original waiter/watch charge before empty Box allocation;
rejection cancels explicitly after allocator return, initialization is once-only, and
both partial phase fallbacks retain all fields. Completed entry/token/source and wider
containing contexts remain separate.

The [completion callback-owner
follow-up](../reports/audits/2026-10-16-completion-callback-ownership.md) captures exact
target/weak observer before watch/list/entry admission, borrows entry factories and
explicitly finishes ordinary construction/publication outcomes before ready
notification. All four phase fallbacks retain the complete captured record and original
fields; wider observation/token/source destruction remains separate.

The [exit-watch ownership
follow-up](../reports/audits/2026-10-16-thread-exit-watch-ownership.md) retains captured
candidate source, weak callback and original watch charge before list/entry allocation
in one inert owner.

The scheduler hook borrows it; ordinary outcomes explicitly finish after local guards,
candidate source last. Four phase fallbacks retain every field. Initial
capture/admission, generic published/source/token destruction and wider contexts remain
separate; no target/root lease, physical proof or retry custody.

The [waiter preparation ownership
follow-up](../reports/audits/2026-10-16-waiter-preparation-ownership.md) captures exact
source/sponsor/weak observer before waiter admission in one inert owner.

Ordinary preparation and publication outcomes explicitly finish after local guards;
transient entry/admission releases before source and sponsor last. Three phase fallbacks
retain every field through source closure and sponsor retirement. Initial source
capture, generic completed entry, published token/source/batch destruction and wider
contexts remain separate; no root/thread lease, physical proof or recovery custody.

The [completed observer-entry
follow-up](../reports/audits/2026-10-16-completed-observer-entry-ownership.md) makes
shared completed private preparation inert on abandonment and requires consuming finish
from direct registration and callback/exit-watch/waiter containing owners after local
guards.

Publication transfers the same entry once; emptied finish cannot refund it. Active
linked-entry/token/batch/source destruction and wider contexts remain separate; no
physical proof or recovery custody.

The [lazy-source owner
follow-up](../reports/audits/2026-10-16-lazy-waiter-source-ownership.md) captures
original sponsor/weak callback before candidate preparation in one inert transaction,
retaining selected winner and shared waiter child. Losing release stays post-slot and
before admission; ordinary results explicitly finish. Six handoff-stage fallbacks retain
complete fields, while published tokens keep their own entry authority. Inner list
factory, active source/token/batch destruction and wider contexts remain separate; no
root lease, physical proof or recovery custody.

The [coherent-DMA remap fence](../reports/audits/2026-10-18-coherent-dma-remap-fence.md)
blocks fresh read-only and writable CPU mappings until the last DMA pin is removed.
Consuming restricted CPU teardown does not discharge a retained pin or original charge;
existing coherent mappings precede pinning.

The [NS publication model and generation
correction](../reports/audits/2026-10-18-name-publication-model.md) projects
candidate/watch ownership, protected public issuance, exact death retirement and
watch-before-source disposal. Policy publication rejects beyond the positive signed NS
generation range before mutation, leaving the existing binding/tombstone and unused
candidate owner intact.

Kernel producer provenance, trusted watch-budget preparation, actual metadata/Drop/outer
contexts and physical completion remain separate.

The [per-read fence follow-up](../reports/audits/2026-10-19-raft-read-fences.md) retains
the query fence, original deadline and typed reply destination in one bounded DNS
operation. Timeout/drop abandons the read without rolling back its no-op or admitting a
business invocation. Placement lookup returns no local capability authority; remote
metadata uses an owned moved-memory reply. Subsequent invocation retains its existing
owned NS lookup/call phases on the shared CQ, with original deadlines and fail-closed
binding/cancellation policy.

This extends C18 without a new custody registry or physical completion claim; raw
transport and wider allocation/readiness contexts remain separate.

The [Raft stream correction](../reports/audits/2026-10-19-raft-snapshot-streams.md)
keeps partial bytes with their captured leader/RPC term and boundary; election/term
change discards that local buffer.

This releases only core heap storage in the serialized userspace reactor, with no
capability, kernel root, physical proof or retry authority. General heap admission and
raw transport contexts remain separate.

The [early artifact-authority
follow-up](../reports/audits/2026-10-19-security-platform-artifact-authority.md) retains
the zeroizing boot preparation in the existing slot before platform loading. Composition
takes that owner once while public authority stays immutable; rejected
selection/publication returns the complete unused owner after unlock.

Final publication requires the selected policy/digest.

This adds no capability/root cleanup, recovery custody or physical proof; protected
installation/custody remains separate.

The [SEC-15 hash-state
follow-up](../reports/audits/2026-10-19-security-hash-state-erasure.md) extends the
bounded local scratch inventory: SHA Drop erases named state/buffer/counters,
compression owns its schedule/working words, and shared HMAC plus SCRAM PBKDF2 transfer
zeroizing result owners.

This is infallible local erasure without guards, allocation, capability release or
recovery custody. Scalar/compiler/library copies, abort-without-Drop and physical
teardown remain separate; no physical proof or replay authority is added.

The [SCRAM phase follow-up](../reports/audits/2026-10-19-security-scram-phases.md)
consumes the initial password/transcript owner during challenge handling and retains
proof plus expected signature in one final owner. Every terminal verification result
consumes that phase, including early rejection. Implicit Drop erases/frees its existing
local buffers in ordinary userspace authentication context; no capability syscall, new
registry, physical proof or replay permission is added. Configured credentials,
third-party/caller copies, forced termination and broader allocator/outer contexts
remain separate.  The [HPKE erasure
follow-up](../reports/audits/2026-10-19-security-hpke-erasure.md) requires erasing
X25519 private/shared dependency owners and returns generated private bytes in their
existing zeroizing owner. Authentication rejection explicitly zeroizes the attempted
plaintext prefix.

These local destructors add no locks, allocation, capability cleanup or recovery
custody; library scratch, forced termination and physical release remain separate.  The
[Base64 scratch follow-up](../reports/audits/2026-10-19-security-base64-scratch.md)
constructs SCRAM helper output owners before writing, reserves the full decode bound and
erases partial output on rejection. Quantum digits/words have local zeroizing owners.

Their infallible Drop erases/frees ordinary userspace scratch without guards, capability
calls, custody or physical completion. Frame/transport copies and forced termination
remain separate.  The [SASL frame
follow-up](../reports/audits/2026-10-19-security-sasl-frames.md) validates full request
size before copying into a reserved erasing encoder and transfers that buffer directly.
Reply parsing consumes the complete zeroizing frame and lends a validated range,
eliminating the independent authentication copy through late rejection. Scoped
first-exchange buffers drop before the final exchange.

These local byte owners perform no capability/physical cleanup, guard acquisition or
custody work; transport copies and forced termination remain separate.  The
[receive-owner follow-up](../reports/audits/2026-10-19-security-receive-owners.md) puts
Kafka partial frames and cached chunks in one existing transport owner. Header
validation admits one frame reservation; completed owners transfer without growth/split
copies. Error/unwind fences reads until stream discard/reset. TLS/plain chunks and
adapted S3 buffers erase through local Drop; consumed ranges wipe explicitly.

No capability/root release, guards, allocator custody or replay proof is added. S3
growth, library records, socket backing and forced termination remain separate.  The [S3
response-buffer follow-up](../reports/audits/2026-10-19-security-s3-response-buffers.md)
reserves the same local header/body owners before copying and erases framing scratch
after consumption/Drop.

Failed parsing fences that episode; service rejection removes the containing operation.
Local byte Drop adds no capability, guard, recovery-custody or physical replay
authority.  The [S3 request-owner
follow-up](../reports/audits/2026-10-19-security-s3-request-owners.md) retains
signature/authorization text and the reserved serialized header in local erasing owners
through send/rejection. Field validation precedes output allocation; normal byte Drop
introduces no capability release, guard acquisition, custody or physical/replay proof.
The [canonical-signing
follow-up](../reports/audits/2026-10-20-security-s3-canonical-owners.md) reserves and
owns canonical URI/query/header copies, scope/hash text and returned
request/transcript/name fields before writing. Invalid header shapes reject before
normalization; source inputs retain their own lifetimes.

Ordinary byte Drop adds no capability, guard, custody, physical or replay authority.
The [response-metadata
follow-up](../reports/audits/2026-10-20-security-s3-metadata-owners.md) consumes the
existing header buffer into ResponseHead and lends validated metadata ranges through
encoding. Rejection/Drop erases that whole owner; replacement retains superseded bytes
until Drop. Exported IPC/client copies keep their own lifetime.

No independent metadata allocation, resource registry, guard, custody or physical/replay
authority is added.  The [request-key/raw-path
follow-up](../reports/audits/2026-10-20-security-s3-path-owners.md) retains an
independently owned erasing key snapshot after the existing IPC mapping boundary, and
reserves the complete erasing raw path. Rejection/abandonment drops local byte owners
without capability or guard work. Validated profile fields still determine
bucket/prefix; no new registry, custody or physical/replay authority is added.  The [S3
profile-owner follow-up](../reports/audits/2026-10-20-security-s3-profile-owners.md)
retains the complete validated service-local profile in one immutable erasing frame,
lends fields without copies and reserves erasing HTTP authority text.

Ordinary byte Drop performs no capability calls, guard acquisition, waits, remote
teardown or recovery-custody work. Original launch memory, logging/provisioning copies
and physical release remain separate; no new registry, retry or cleanup authority is
added.  The [S3 profile-encoding
follow-up](../reports/audits/2026-10-20-security-s3-profile-encoding.md) returns the
existing single encoded frame under an erasing owner before credential writes.

The kernel caller's former immediate wrapper is removed; its containing launch scope,
byte-buffer release and borrowed launch-memory preparation remain unchanged.

Ordinary byte Drop adds no guards, waits, capability/root cleanup, custody or
physical/replay authority. Caller inputs and launch-memory backing retain separate
policies.  The [Kafka profile-encoding
follow-up](../reports/audits/2026-10-20-security-kafka-profile-encoding.md) returns the
existing single frame under an erasing owner before credential writes.

The kernel's immediate wrapper is removed while its containing launch scope and
byte-buffer release context stay the same.

Ordinary byte Drop adds no guards, waits, capability/root cleanup, remote teardown,
recovery custody or physical/replay proof.

Source credentials, connector-local copies and launch backing keep separate policies.
The [SCRAM initial-owner
follow-up](../reports/audits/2026-10-20-security-scram-initial-owners.md) keeps nonce,
password and directly escaped transcript in the existing initial phase, removing one
scratch allocation. Consuming success/rejection and ordinary Drop erase/release that
local state before final-phase use.

No capability calls, guards, waits, remote teardown, new registry, custody or
physical/replay authority are added; original inputs and configured credentials retain
separate policies.  The [host key-file
follow-up](../reports/audits/2026-10-20-security-host-key-encoding.md) reserves one
erasing hex-plus-newline owner before copying generated key bytes.

The existing host file writer borrows it through success/rejection; its prior
final-buffer erasure and filesystem rollback contexts remain unchanged.

Ordinary byte Drop adds no Charlotte capability/root cleanup, guards, waits, new
registry, custody or physical/replay proof.

Source keys and OS/file buffers retain separate policies.

The [host profile-input
follow-up](../reports/audits/2026-10-20-security-host-profile-input.md) retains one
complete initialized erasing byte destination before reads, including partial failure
and unreported dirty tails.

Ordinary byte Drop adds no capability/root work, guards, waits, remote teardown, custody
or physical/replay authority.

Source files and OS buffers retain separate policies.

The [public-key input
follow-up](../reports/audits/2026-10-21-security-public-key-input.md) shares the
existing bounded host file reader and returns public key buffers through the existing
local byte-owner scope.

Ordinary release adds no capability calls, guards, waits, new registry, recovery custody
or physical/replay proof; public file checks grant no key-provisioning authority.

The [operational-input
follow-up](../reports/audits/2026-10-21-security-operational-input.md) shares that host
reader for bounded ciphertext and release references, checking binding count before
referenced reads. Rejection drops existing public byte owners before output/network
publication.

No new capability/root cleanup, guards, waits, registry, custody or physical/replay
authority is added.

The [control-input follow-up](../reports/audits/2026-10-21-security-control-input.md)
extends the same local host reader/byte-owner scope to structured control records and
release references. Rejection precedes notification/apply connection; release count
admission precedes referenced reads.

Existing apply preparation order remains.

No new capability/root cleanup, guard, wait, registry, custody or physical/replay proof
is added.

The [artifact-input follow-up](../reports/audits/2026-10-21-security-artifact-input.md)
shares captured-descriptor admission for bounded ELF reads and streaming hash input.
Note output sizing and variant count reject before their respective allocations/reads;
the hash loop owns one erasing 16 KiB array plus existing SHA state.

Ordinary host byte/descriptor Drop adds no Charlotte capability/root cleanup, guard,
wait, custody or physical/replay authority.

The artifact-edit follow-up below addresses write redirection; atomicity and other
copies remain separate.

The [artifact-edit follow-up](../reports/audits/2026-10-21-security-artifact-edit.md)
retains the exact host read/write File and prepared ELF bytes in ArtifactEdit until
consuming commit returns. Borrowed shared admission/reads keep that descriptor live.
Drop before commit closes/releases local fields without writing; commit error consumes
the owner with no retry/rollback or durable-completion proof.

No path reopening, registry, Charlotte root/capability cleanup, guards, waits or
physical/replay authority is added. Rename/unlink changes names without changing the
selected file; pathname continuity, concurrent writers and partial writes remain
separate.

The [control-output follow-up](../reports/audits/2026-10-21-security-control-output.md)
reuses the existing exclusive host writer for deployment/release/shutdown records.
Rejection at creation leaves existing paths untouched and releases prepared public byte
owners; ordinary success closes the local file. Fresh private fixture directories retain
each run's signed records.

No Charlotte capability/root cleanup, guard, wait, registry, recovery custody or
physical/replay authority is added.

The output-retention follow-up below removes failed-write pathname unlink and key-pair
rollback; exclusive admission supplies no atomic/durable publication proof.

The [output-retention
follow-up](../reports/audits/2026-10-21-security-output-retention.md) closes the
existing host File on write error without pathname deletion. One shared pair writer
retains a completed private file when public output fails and reports incomplete
generation. Per-file erasing text still releases before the next encoding; source key
owners retain their existing lifetime.

Retained filesystem content is operator-inspection state, never a retry owner or
completed-pair proof.

No new guards, waits, Charlotte capability/root cleanup, registry, custody or
physical/replay authority is added; namespace continuity, disk erasure and
atomic/durable publication remain separate.

The [fixture-publication
follow-up](../reports/audits/2026-10-21-security-fixture-publication.md) retains the
local bundle's staging file in the existing private run directory.

An exclusive Python file owner closes before one named replacement; staging/write/close
failure leaves prior output alone, and tested replacement rejection retains complete
staging. Failure cleanup performs no pathname deletion or implicit retry. Success
publishes a new file and preserves earlier open readers; retained filesystem artifacts
are inspection state, never a retry owner, capability/root completion or custody proof.

No guards, waits, registry or recovery family is added. Namespace integrity, output
metadata, concurrent writers, fsync/durability and ambiguous filesystem errors remain
separate.

The [deployment-generation
follow-up](../reports/audits/2026-10-21-security-deployment-generation.md) reserves host
observation storage before descriptor notification and borrows captured names from the
retained prepared byte set.

Each successful acknowledgement supplies its own generation; ordinary terminal
success/error drops public byte/string/vector fields and closes existing host sockets.
Malformed or superseded observation grants no rollback, completion of remote work or
authority to replay accepted notification.

No Charlotte capability/root cleanup, guards, waits, new registry/family, recovery
custody or physical proof is added.

The [release-generation
follow-up](../reports/audits/2026-10-21-security-release-generation.md) prepares local
reply authority plus mapped output before Raft admission or relay, and bounded remote
terminal bytes before submission.

Existing pending owners retain these through commit, term change, redirection and
timeout. Incoming transactions transfer once before scalar guards; consuming completion
moves the same output memory. Clusterctl/deployd retain request and returned-memory
owners across their existing guard-free service calls.

Ordinary preparation rejection drops local owners before mutation; accepted ambiguity is
terminal and grants no replay or rollback authority. Host observation storage reserves
before POST and borrows retained component names.

No kernel root, pin, charge, physical proof, custody registry or new family is added.
Live allocator/map/wait-rejection injection and wider destructor contexts remain
unqualified.

The [host deadline follow-up](../reports/audits/2026-10-21-security-ingress-deadline.md)
carries one checked monotonic deadline through prepared inputs, all notifications and
observation.

Its shared transport owns one host socket across connect, partial writes and response
reads; every step consumes remaining budget, and ordinary error/timeout closes that
socket without replay or remote rollback. Numeric endpoints reject DNS fallback; no
worker thread, registry, Charlotte capability or new cleanup family is added. Unstarted
expiry opens no socket.

The [named-status follow-up](../reports/audits/2026-10-21-security-named-status.md)
validates complete read-only observations and preserves the complete status body for all
apply/status callers. Malformed HTTP-200 data terminates observation without another
request; invalid names admit no socket or wait.

Existing public response strings and host socket scopes dispose normally.

No owner, cleanup/retry policy, remote mutation, capability/root, registry or family is
added; a valid current-name observation supplies no admission, retirement or cleanup
authority.

The [response-reader
follow-up](../reports/audits/2026-10-21-security-ingress-response.md) applies
complete-body extraction and current scalar/receipt/policy codecs in the same host
socket/public-string scopes. Malformed acknowledgement is terminal uncertainty with no
POST replay; malformed read-only policy observations reject before output.

No owner, root, capability, retry/custody policy, registry or family changes. Valid
structure grants no local UTC or peer-authentication authority. Host blocking file
I/O/stdout, OS return latency, polling and peer authentication remain separate.

The [private-key input
review](../reports/audits/2026-10-21-security-private-key-input.md) keeps the existing
host file, bounded erasing input and decoded-key scope; normalization now returns its
erasing text owner directly.

Ordinary byte Drop performs no Charlotte capability/root cleanup, guards, waits, remote
teardown or custody work.

The existing caller already erased its normalization result; no input leak, retry
authority, physical proof, registry or new family is claimed. Files/OS copies and forced
termination remain separate.

The [AES dependency follow-up](../reports/audits/2026-10-21-security-aes-erasure.md)
enables local round-key wiping in the existing record cipher's Drop and requires its
AES-128 owner traits.

No Charlotte capability/root cleanup, allocation, guard, wait, remote teardown,
registry, custody or physical/retry authority is added. Cipher-local erasure is not
complete TLS traffic-secret/HKDF/GCM-state erasure.

The [GCM feature follow-up](../reports/audits/2026-10-22-security-gcm-erasure.md)
enables existing constructor-scratch wiping and software POLYVAL state Drop in the same
record-cipher owner. Local field erasure performs no capability/root cleanup,
allocation, guard, wait or remote teardown and adds no registry, custody or
physical/retry authority.

The AArch64 destructor-presence gate rejects unqualified backend changes; x86 dispatch,
optional PMULL and enclosing TLS traffic-key/IV/HKDF state remain gaps.

The [deployment input isolation
follow-up](../reports/audits/2026-10-22-security-deployment-input.md) composes the
existing userspace transaction family into four bounded listening/receiving positions
with `Input` and `Accepting` owners. Spare listeners are preposted before accept
observation. Accepted pending calls precede socket fields; ordinary completion, EOF,
framing/allocation rejection and original-deadline expiry dispose that call before
returning the socket for existing explicit close. TCP/IP retains the full authenticated
accept request beside its existing receive transaction and completes it from actual
state/close events. Accepted-call CQ binding or wait admission rejection aborts with the
whole pool live, without replay or fabricated completion.

No kernel root/pin, physical proof, custody registry or retry authority is added. Caller
mapping failures release their owned returned memory normally; synchronous
controller/send/close and their outer failure contexts remain separate qualification
work.

The [controller reactor
follow-up](../reports/audits/2026-10-22-security-deployment-control.md) retains each
accepted controller call, socket, receipt envelope, original deadline and active request
reservation in one `Operation`.

The same four positions cover listening, input and control phases; rounded moved pages
and retained envelope capacity share input admission. Observe/cancel the pending call
before returning its socket for existing response/close policy. Accepted binding or CQ
wait rejection retains the complete pool through abort. Local timeout is terminal
uncertainty without replay; the receiver still owns delivered memory and its original
kernel charge. Dropping a logical reservation is no physical release or charge-refund
proof.

No custody registry, retry owner, root or family is added. Synchronous
preparation/render/send/setup/close and controller-server internals remain separate
contexts.

The [output reactor
follow-up](../reports/audits/2026-10-22-security-deployment-output.md) extends the same
C18 owners with complete bounded response backing, socket, original deadline and pending
moved send page. Observe/cancel pending before socket release. TCP/IP retains the
complete authenticated request and read-only mapping through handshake/buffer pressure;
consuming reply ends mapping before authority. Only confirmed positive prefixes advance;
timeout/observation failure remains terminal uncertainty without replay.
Mapping/submission failure disposes unstarted owners; binding/wait rejection retains the
pool through abort.

Existing synchronous socket close and its implicit fallback remain separate contexts; no
kernel root/pin, original charge refund, physical completion, retry custody, registry or
family is added. Native success/rejection covers normal sends and moved-frame cleanup;
real map/bind rejection and saturated buffers remain unqualified.

The [logical-close follow-up](../reports/audits/2026-10-22-security-deployment-close.md)
consumes each finished/rejected client into a `ClosingSocket` holding the accepted
pending call before its exact socket.

The same four-position pool retains one original admission/observation deadline.
Queue-full rejection alone permits bounded admission backoff; no accepted replay.
Checked Started/Unavailable scalar receipt disarms the socket before owner disposal and
returns its position, without physical/backing/charge proof. Other rejection,
malformed/uncertain observation, deadline or incomplete Drop aborts with the complete
scope live and no second remote teardown.

Existing closes drain against original budgets at lifecycle shutdown while other owners
remain live. Other phases' implicit `OwnedSocket` destructors and partial setup remain
separate blocking contexts.

The older synchronous helper could still restore an ID after accepted-call error at that
milestone; the synchronous follow-up below removes that path. Unavailable concerns only
an absent full ID below the nonreused reservation watermark in the same endpoint
lifetime; it proves no former owner or cleanup. Foreign live/unreserved targets reject.
Opaque unpublished serial reservation is consumed by publication, burns on failed
publication and rejects exhaustion before backend allocation.

No new root/pin, custody, retry owner, registry or family is added. Host adapters check
retained ownership at abort; native ordinary and already-reaped close are qualified,
real admission/observation fault injection and whole-shutdown latency are not.

The [synchronous-close
follow-up](../reports/audits/2026-10-22-security-synchronous-close.md) moves one private
`SocketLease` (selector plus original connection incarnation) between OwnedSocket and
ClosingSocket.

Explicit close and ordinary Drop enter the same owning Blocking transaction once; no
scalar reconstruction or second destructor submission. One five-second deadline covers
admission/one checked wait; queue-full-only 50 ms backoff stops after the original first
second.

Final unstarted rejection returns its actual IPC error and forgets the complete driver
without field destruction; any TCP/IP record and original charge remain, with no retry
custody, rollback or refund proof.

This unsubmitted driver owns no request memory, loan or accepted kernel grant. Accepted
timeout, rejected wait admission or invalid/service/observation outcome retains the
close transaction through domain abort. Terminal runtime observation may already consume
its pending kernel grant; no ID is restored. Confirmed logical receipt closes the
pending owner before consuming the lease, without physical release proof.

Shared observation disposes unexpected returned owners under existing runtime policy.

These are guard-free userspace calls; ordinary Drop can block, containing fields may
already have disposed independent resources, and broader parent destructors/latency
remain unqualified.

No new root/pin, registry, family or recovery owner is added. Host adapters check no
replay/reentry and owner retention; native shared success is exercised, actual failure
injection is not.

The [immediate-send follow-up](../reports/audits/2026-10-22-security-immediate-send.md)
removes scalar packed-ID decoding and raw memory cleanup from OP_TRY_SEND.

Both send forms retain the same PendingSend complete incoming transaction,
caller-sponsored backing, read-only mapping, reply authority and once-only enqueue
state. Immediate TCP/UDP attempts once and never installs a retained send waiter.
Malformed/retired frames reject before enqueue; exact full-ID owner-generation lookup
cannot select a low-half alias. Submission and local mapping failures use existing
returned owners; server rejection disposes mapping/backing before consuming reply
authority.

No physical proof, original-charge refund before actual release, new
root/pin/registry/family or recovery custody is added.

These are the existing guard-free service-loop contexts; broader containing destructor,
cancellation and pressure qualification remain open. Native
malformed/missing/extended/high-half controls and normal UDP/TCP paths qualify shared
cleanup success, without injected map/submission/reply failure or kernel cancellation
proof.

The [listener setup
follow-up](../reports/audits/2026-10-22-security-deployment-listener.md) extends this
same four-position family through creation/listen/accept. OpeningSocket retains its
accepted pending request and original connection borrow until checked scalar creation
receipt; no socket ID is reconstructed. Known socket, moved listen page/pending call and
one original setup deadline remain in the containing listener owner. Queue-full
submission rejection alone retries a phase; confirmed negative creation owns no socket,
while unstarted preparation or confirmed later rejection moves the known socket into the
existing close pool. Binding, malformed/uncertain observation, expiry or incomplete
owner destruction aborts before field release, without accepted replay or synchronous
socket fallback.

These are guard-free userspace contexts; returned attachments follow existing runtime
disposal, without a new physical release/charge-refund proof, root/pin, custody,
registry or owner family. Shutdown retains accepted creation/listen until observation,
then moves known sockets to close; idle accept cancellation disposes its pending owner
before moving the socket, with no remote rollback proof. Other parent
destructors/syscall latency remain unqualified. Host adapters cover admission, mapping
rejection, delayed phases, no replay, retained abort and shutdown; native IPC adapters
exercise moved setup and held creation, while native HTTP qualifies ordinary TCP
success. Real map/bind faults, saturated admission/backends and complete shutdown
pressure remain open.

The [controller-server relay
follow-up](../reports/audits/2026-10-22-security-controller-relay.md) extends this same
service transaction family through the original IncomingMessage, read-only source
mapping, one accepted DNS PendingCall and checked returned result. Eight server
positions reserve original backing plus a copied DNS request before preparation; one
server deadline spans preflight/mutation without renewing the caller's separate
deadline. Only checked absence/older-record evidence advances to submission;
malformed/failed/foreign records never authorize it. Cancellation/timeout ends pending
DNS before source mapping and upstream reply/loan authority; consuming result
publication moves its returned memory once. Binding/wait rejection retains the complete
pool through abort. Shutdown uses the same pending-before-mapping-before-incoming
disposal.

These are guard-free userspace contexts, without a new root/pin, registry, family or
recovery custody. Logical pool release does not refund a DNS-retained copy or original
kernel charge. Returned records/decoder allocations and historical completed-phase
copies remain outside this logical byte reservation. Native adapters cover returned
memory, unexpected returned connection, incoming mapping timeout and receiver-retained
DNS copy; actual map/bind faults and full shutdown/pressure remain unqualified. Legacy
upload/join/key handlers and broader parent/destructor/physical cleanup contexts remain
separate.

The [host startup follow-up](../reports/audits/2026-10-23-security-release-startup.md)
retains the exact envelope, existing socket scope and original deadline in one command.
Only failure before HTTP writes or a complete checked target rejection permits a new
admission attempt. Uncertain submission or accepted rollout failure ends the command;
both boot workers invoke it once after the listener and initial DNS leader markers.
Socket disposal supplies no remote cancellation, rollback, physical completion or new
retry custody.

No owner family or kernel release order changes. Aggregate principal shares now
accompany the same memory-object generation account and joint heap/image preparation.
Copies charge their caller; moves and retained receiver backing keep the source
principal. Abandonment merges existing shares without counter locks; confirmed
rollback/physical release alone refunds them.

Shared signed-history storage supplies no new registry or owner family.

See [principal backing
evidence](../reports/audits/2026-10-23-security-principal-backing.md). Call preparation
now owns an exact caller-root lease, original call/reply reservations, fallible prepared
pending node, authority and all attachment staging together. Quota precedes allocation,
outside IPC; publication only relinks the node.

Ordinary finish releases unused fields after local IPC unlock and the root last.
Fallback retains all fields without cleanup. Pending node disposal precedes its original
count refund. Other maps/waiter/vector factories and enclosing masks remain separate;
see [pending storage
evidence](../reports/audits/2026-10-24-security-ipc-pending-storage.md).

The same call owner now prepares its reply node after the pending node, outside IPC with
both original counts reserved. Second-allocation failure retains and explicitly cancels
the first prefix post-guard; fallback keeps all fields.

Publication relinks both nodes; original charges follow node lifetimes.

No new root/registry/family/custody or physical retry.

See [reply storage
evidence](../reports/audits/2026-10-24-security-ipc-reply-storage.md). Endpoint creation
now owns its exact root, atomic record/slot reservations, node, queue, observer lists
and authority through fallible unlocked preparation. Queue growth retains its original
account/classification and both backings through revalidation; old/unused storage
disposes post-IPC, root last.

Complete fallback retains every field.

Guarded payload/authority factories and outer contexts remain separate.

See [endpoint storage
evidence](../reports/audits/2026-10-24-security-ipc-endpoint-storage.md). Per-capability
payload nodes prepare fallibly after shared count reservation and before
authority/attachment mutation in the containing endpoint, call, grant or receive owner.
Unused preparation disposes post-IPC; abandonment retains its fields. Receive retains
speculative payload/authority retirement with its preparation. Factories still run under
IPC; byte/principal and outer contexts remain separate.

See [capability storage
evidence](../reports/audits/2026-10-24-security-ipc-capability-storage.md).
Call/endpoint owners now retain shared-authority preparation outside IPC before captured
count admission; payload allocation and call pending waiter-list construction leave IPC
too.

Ordinary rejected/unused metadata disposes after guards, root last; complete fallback
retains the prefix. Direct grants, receives, returns and memory payload-map contexts
remain separate.

See [unlocked preparation
evidence](../reports/audits/2026-10-24-security-ipc-unlocked-preparation.md). Memory/IPC
authority publication now borrows existing reservations/transfers for validation and
mutation under one capability hold, with no temporary vector or batch-sized stack
storage.

Existing source escrow, copied backing, roots/pins and original charges remain in their
containing owners.

Ordinary unstarted rejection still cancels through those owners; no new cleanup
receipt/custody or physical retry. Mixed memory payload maps remain allocating after
authority publication.

See [borrowed publication
evidence](../reports/audits/2026-10-24-security-ipc-borrowed-publication.md).

Private direct/copy backing now owns fallible object storage after original page/object
admission; copies preclaim a nonreused identity before authority publication. Global
object publication relinks prepared nodes.

Retirement carries the complete node in ChargedFrames/RetiredMemory through physical
completion, node disposal and original charge refund.

A pre-attempt fence and inert backing fields retain all state on interruption/failure;
ordinary rollback keeps its existing caller qualification. Capability/borrower maps and
wider metadata/IRQ contexts remain separate.

See [object storage
evidence](../reports/audits/2026-10-24-security-memory-object-storage.md). Read-loan
borrower storage now prepares fallibly after exact destination admission, before memory
serialization/source escrow.

The same PreparedTransfer cancels unused storage or relinks once; no new
root/pin/registry/family or physical proof. Capability/namespace maps and wider outer
contexts remain separate.

See [borrower storage
evidence](../reports/audits/2026-10-24-security-memory-borrower-storage.md).

Memory payload and namespace nodes now prepare after shared reservation before
backing/source escrow.

Complete commit relinks without heap work; unused candidates stay in the containing
preparation until post-guard disposal.

Source moves retain their detached payload with the same escrow, disposing payload
before retired authority. Read-loan field order now releases unused borrower storage
before destination refund.

No new roots/pins/registry/family/custody or retry proof.

See [memory capability storage
evidence](../reports/audits/2026-10-24-security-memory-capability-storage.md). Mapping
preparation now owns an empty node plus fallible snapshot capacity outside the memory
registry; captured revalidation precedes publication and the existing pin.

Ordinary unused rejection disposes after local guards; uncertain published cleanup
retains the complete node in existing C05 owners.

No new root/pin/registry/family/custody.

See [mapping storage
evidence](../reports/audits/2026-10-25-security-memory-mapping-storage.md). Scratch
preparation now owns fallible extent and namespace nodes before local publication;
ordinary prefix/candidate disposal leaves scratch/device guards, while abandonment
retains all partial fields.

Publication only relinks after captured-generation/current-space revalidation. Whole
namespace/extent fallback stays in the same root metadata family.

No new physical proof/replay authority; see [scratch storage
evidence](../reports/audits/2026-10-25-security-scratch-storage.md). Scratch
namespace/extent requested bytes and node counts now reserve before allocation and stay
in these same owners through actual node free; partial/terminal abandonment retains
original charges without counter entry. Promotion/reuse cannot reclassify refunds.

See [scratch admission
evidence](../reports/audits/2026-10-25-security-scratch-admission.md). Copy-pin snapshot
capacity now prepares outside memory serialization before frame identity/pin capture,
then revalidates exact namespace/source/state before allocation-free fill and pin
publication.

Bare preparation/pin fallback retains metadata and the original backing fence; explicit
snapshot disposal precedes pin decrement. Scoped/source-transfer destructors, DMA
snapshots and aggregate snapshot admission remain wider G1/G4/G7 work.

See [copy snapshot
evidence](../reports/audits/2026-10-25-security-copy-snapshot-storage.md).

DMA snapshot capacity now shares inert pin-free preparation with copy snapshots outside
memory serialization. Captured source/access validation precedes allocation-free fill
and both pin-counter publications; bare fallback retains snapshot/fences, and explicit
snapshot free precedes typed unpin. Original DMA hardware proof and complete
operation/maintenance ownership remain required.

See [DMA snapshot
evidence](../reports/audits/2026-10-25-security-dma-snapshot-storage.md). Mapping
preparation now owns partial node/shared-snapshot storage inertly before factories;
ordinary constructed returns explicitly finish after local memory/table guards. Captured
namespace-generation checks precede mapping/pin publication.

Guarded partial/full/published fallback retains metadata without allocator entry.

Existing roots, physical proofs and uncertainty fences remain required; no retry
owner/custody.

See [preparation
evidence](../reports/audits/2026-10-25-security-mapping-preparation-ownership.md).
Mapping/copy/DMA snapshot buffers now share requested-byte/count admission before
allocation.

Their existing typed buffer owner retains original charges through publication, root
retirement and abandonment; actual buffer free precedes refund, with immutable captured
classification.

Existing roots, backing/hardware proofs and retry policies remain required.

See [snapshot admission](snapshot-buffer-admission.md). Mapping nodes now reserve
requested-layout bytes/count before allocation; the same node retains original
captured-classification admission through preparation, publication and
selected/whole-map retirement. Confirmed actual node free precedes refund and peer-root
completion; guarded abandonment retains all fields without counter entry.

No new physical proof, custody or replay owner.

See [mapping-node admission](mapping-node-admission.md). Capacity defaults now come from
the immutable compiled [resource policy](resource-policy.md); selecting another build
profile does not resize a live account, reclassify a sponsor, refund retained work or
add cleanup/replay authority.

Memory capability preparation now holds partial payload/namespace storage, exact
Reservation/account and generation inertly before factories.

Ordinary finish frees unused storage before cancelling authority; local rejection
returns after guards.

The containing transfer explicitly finishes its destination after existing
source/borrower work.

Bare owner abandonment does no allocator/registry/Arc destruction. Namespace teardown
may drain hidden authority while payload nodes stay retained without byte admission;
late finish cannot affect a successor. Outer transfer/backing/IRQ contexts remain G1
work.

See [preparation
evidence](../reports/audits/2026-10-25-security-memory-capability-preparation.md).
