# Security remediation status and completion criteria

Updated 2026-10-27 following native management TLS, restart/rotation, validity and leader-loss work;
SEC-10 remains partial. Earlier reviews retain their original dispositions. Source and tests
remain authoritative. Historical reports preserve the state at their own
revision; their remaining-work paragraphs are not a current backlog.

## Finding ledger

| State | Findings | Meaning |
| --- | --- | --- |
| Corrected in the documented scope | SEC-01, 02, 03, 05, 11, 12, 13, 16, 17, 19–38 | The identified defect has a correction and recorded validation. This is not a claim of complete subsystem security. |
| Partly corrected or mitigated | SEC-04, 06, 07, 10, 14, 15, 18 | Specific paths are strengthened; the broader finding still has required work. |
| Open | SEC-08, 09 | The required security boundary is not implemented. |

There are **38 numbered findings: 29 corrected, seven partial and two open**.
SEC-10 now has native authenticated transport, exact operator roles and x86
revision-advance/restart revocation evidence. Protected enrollment, platform
anti-rollback and broader rotation/failover qualification are still required.
The [2026-10-10 progress verification](../reports/audits/2026-10-10-security-progress-verification.md)
temporarily returned SEC-12 to partial for metadata-read failure and automatic-ID
aliasing. The [follow-up correction](../reports/audits/2026-10-10-security-object-store-follow-up.md)
rejects metadata failure, retains one validated object snapshot and bounds
collision-free creation. Host/guest regressions also exercise malformed and
unauthorized policy rejection and live scope revocation. SEC-12 returns to
corrected in that documented scope; historical reports preserve each checkpoint.
The earlier SEC-01–25 accounting was 15 corrected, six partial and four open;
it excluded the five additional, corrected findings SEC-26–30. Finding counts
measure bookkeeping, not percentage of effort or production readiness.

The [2026-10-15 process review](../reports/audits/2026-10-15-audit-process-review.md)
reconciles the panic history and outstanding work. IPC exact-exit timeouts
reproduced on both architectures after the initial passing investigation.
Later park/first-dispatch corrections have negative controls and passing native
execution, but do not establish the historical timeout or Raft persistence
cause. The CI Sitas ABI mismatch and missing-frame-pointer configuration have
separate reproduced/corrected evidence. No numbered finding is closed by these
results; the review's fresh host suite passes 483 tests with four ignored.

The [first audit](../reports/audits/2026-10-03-security-audit.md),
[renewed audit](../reports/audits/2026-10-05-security-audit.md),
[SEC-23–25 follow-up](../reports/audits/2026-10-05-security-follow-up.md),
[2026-10-10 renewed audit](../reports/audits/2026-10-10-security-audit.md),
[SEC-31–35 remediation](../reports/audits/2026-10-10-security-audit-remediation.md),
[SEC-36–38 remediation](../reports/audits/2026-10-10-security-audit-remediation-2.md),
the [live-upgrade IOMMU initialization fix](../reports/audits/2026-10-10-security-live-upgrade-iommu-init.md),
and [report index](../reports/README.md) retain the evidence. This page gives
the remaining acceptance criteria, so a completed narrow fix need not reopen
an unbounded audit loop.

The [read-borrower admission correction](../reports/audits/2026-10-26-security-borrower-node-admission.md)
extends the existing configurable memory-capability node pool to read-loan
borrower metadata. All three nodes reserve before source mutation; actual
ordinary/total pressure, trusted reserve and original-charge revocation exercise
the production path. SEC-07/18 remain partial; no new limit family is added.

## SEC-31–38: renewed-audit findings

The [2026-10-10 renewed audit](../reports/audits/2026-10-10-security-audit.md)
confirmed eight new source-confirmed findings. SEC-31 and SEC-33–38 are
corrected in the
[first](../reports/audits/2026-10-10-security-audit-remediation.md) and
[second](../reports/audits/2026-10-10-security-audit-remediation-2.md)
remediation reports. The [caller-buffer follow-up](../reports/audits/2026-10-10-security-tcpip-caller-buffers.md)
also corrects SEC-32 in the receive/status scope: clients provide reply backing,
and retaining it consumes the original caller budget rather than TCP/IP's.
Aggregate principal/heap admission and broad pressure remain SEC-07 work.
The caller-buffer report preserves repeated combined security/HTTP host
failures. The [HTTP progress follow-up](../reports/audits/2026-10-10-security-http-progress.md)
identifies listener abandonment on queue-full call admission and corrects it
while retaining the same socket owner. Combined kernel and host checks now
pass within their existing limits. Broader SEC-06 flood fairness and SEC-07
R7-4/R7-5 qualification remain open.

| ID | State | Finding | Acceptance direction |
| --- | --- | --- | --- |
| SEC-32 | Corrected in documented scope | Receive and status requests move a caller-supplied page, which returns with the data; TCP/IP allocates no reply backing for these paths. | Caller-ceiling saturation, cancellation/mapping rejection, retained-charge lifecycle and bidirectional TCP retention are covered in the [follow-up](../reports/audits/2026-10-10-security-tcpip-caller-buffers.md). Broader heap/aggregate admission remains R7-3/R7-5. |

## SEC-06: management progress

The [entropy deadline follow-up](../reports/audits/2026-10-26-security-entropy-deadlines.md)
bounds client-side accepted RNG reply observation with one configurable
request budget installed alongside the source. Pending/reply/mapping owners
retain the original deadline and source/output borrows; timeout and exact
wait-admission rejection stay distinct, with no replay or fabricated bytes.
Native reply/attachment, explicit cancel/Drop, late-result and full/silent queue
controls qualify that scope. This does not bound dependency naming, RNG-server
publication, kernel mapping/cancellation latency or a complete TLS exchange.
The subsequent [RNG caller-buffer correction](../reports/audits/2026-10-26-security-rng-caller-buffers.md)
removes producer-sponsored reply allocation: RNG v2 fills/returns supplied
caller backing, rejecting the retired scalar path. Retained/unobserved pages
keep their original caller charge. Concrete ceiling, producer-account,
independent-progress and sponsor-retirement controls qualify this narrow
retention path; authorized flooding, shared metadata pressure and broader
SEC-07 acceptance remain open. Finding dispositions stay unchanged.

The [RNG server-readiness follow-up](../reports/audits/2026-10-26-security-rng-readiness.md)
binds its owned endpoint to the default CQ before bounded shared publication.
Idle request/lifecycle observation now registers event-only interest instead of
10 ms timer polling; rejected binding/wait/publication aborts with device and
endpoint owners live. Native exact-thread/CQ/no-timer checks reject the old
signed server and qualify endpoint progress and unrelated/lifecycle-publication
repark on the replacement. This does not qualify actual hardware shutdown,
descriptor timeout recovery or broader flood fairness.

The [RNG descriptor-owner correction](../reports/audits/2026-10-26-security-rng-descriptor-fence.md)
now arms a terminal fence before queue mutation and checks exact completion
index, descriptor and length before advancing. Timeout/backend failure retains
the complete DMA/MMIO owner and captured attempt; late completion or another
request cannot restart it, and fallback cannot destroy nested fields. A fenced
shutdown retains the endpoint/device/request without a quiesced acknowledgement.
Host/native fault controls and isolated replay/destructor regression mutations
qualify the software policy. Real hardware timeout/reset/recovery, partial setup,
healthy-shutdown proof and full pressure remain separate SEC-06/18 work.

The [HTTP listener correction](../reports/audits/2026-10-10-security-http-progress.md)
retains listeners through queue-full admission. The
[deployment ingress follow-up](../reports/audits/2026-10-11-security-deployment-ingress.md)
extends connection-local rejection to deployment setup/accept and makes its
pre-authentication parser allocation fallible without a second body copy.
Both endpoints have total request budgets. Deployment input and controller reply observation now share the bounded concurrent loop described below; response transmission now uses the shared CQ as well. The same follow-up corrects shared identity creation ownership, the catalogue verification stack budget and scoped readiness observation. The read-only namespace query returns a live publication generation without granting invocation authority; legacy lookup stays denied. Final signed upload/release/pull/verification/launch/readiness passes. Earlier failed signed runs and intermittent AMD frame-router startup remain recorded separately; a passing repeat does not resolve that intermittence.

The [network publication follow-up](../reports/audits/2026-10-12-security-network-publication.md)
adds first-failure NIC startup diagnostics and corrects source-confirmed terminal
registration on delegated queue-full rejection. Admission retains the same
endpoint; accepted calls are observed once. The historical AMD exit remains
unattributed despite passing diagnostic repeats.

The [platform publication deadline follow-up](../reports/audits/2026-10-12-security-publication-deadlines.md)
removes TCP/IP's unlimited retry of permanent registration rejection and applies
the same endpoint-preserving admission policy to time, cluster control, reliable
messaging and discovery. The shared helper now bounds accepted-call observation
to five seconds and cancels through its pending owner without replay. This bounds
these registration waits; dependency lookup, general controller RPC completion
and cancellation syscall/remote scheduling progress remain separate boundaries.

The [deployment controller deadline follow-up](../reports/audits/2026-10-12-security-controller-deadlines.md)
bounds all seven deployment HTTP controls' controller admission/observation to
one five-second policy deadline. Accepted mutations are not replayed; timeout
cancels the exact pending owner and reports an uncertain outcome as a distinct
503 error. This qualifies the ingress-side wait, not controller-internal
blocking, concurrent healthy-client fairness or transport teardown progress.
The same follow-up retains an Intel object-store/readiness panic (`0xd1`, block
read failure) separately from a passing identical-artifact repeat. Its exact
read result/rejection phase and startup-versus-serving classification remain
unverified; the historical AMD NIC exit remains separately unattributed.

The [object-store readiness/mount follow-up](../reports/audits/2026-10-12-security-object-store-readiness.md)
replaces numeric-stage admission with an explicit published serving milestone,
separates startup failure from first-I/O diagnostics and rejects unreadable
superblocks before formatting. It corrects source-confirmed gate/mount policy
defects; the underlying historical Intel read rejection and AMD NIC exit still
need causal captures. A boot milestone does not establish ongoing storage health.

The [deployment input isolation follow-up](../reports/audits/2026-10-22-security-deployment-input.md)
adds four bounded listening/receiving positions, preposted spare listeners, a
shared input-prefix byte budget, rotating input
turns and CQ-bound deferred accept/receive. A healthy input can finish beside
three incomplete peers. Original header/total deadlines survive progress and
unrelated wakes. Accepted-work binding/wait rejection retains the operation
owners through terminal abort. That milestone left controller calls synchronous;
the next follow-up addresses their ingress-side observation. HTTP keyhole
serving remains serial.

The [controller reactor follow-up](../reports/audits/2026-10-22-security-deployment-control.md)
keeps all seven controls' accepted calls, sockets and validation data in owned
CQ-bound operations. Input and controller phases share four positions and a
conservative active-request byte budget. Another ready response can progress
beside an unanswered controller request. Five-second budgets start before
preparation; timeout is uncertain and never replayed. The receiver can retain
its moved request and original charge after local timeout. Controller-server
execution, response send and socket setup/close remained blocking at that
milestone; the output follow-up below addresses send observation.

The [output reactor follow-up](../reports/audits/2026-10-22-security-deployment-output.md)
adds CQ-bound response sends to those same four positions. Complete response
backing (up to 64 KiB capacity), socket, pending moved page and original five-second
deadline stay together. Only a positive TCP prefix receipt advances the remaining
bytes. Timeout and observation failure terminate without replay or delivery
claims. The new exact CSOCKTX1 frame carries the full socket ID. Native normal
HTTP/release and rejection/cleanup cases qualify this path; saturated native TCP
buffers and whole-service fairness remain unqualified. TCP/IP retains its timed
transport cadence. Setup/close/render/controller work remained blocking at that
milestone; later follow-ups below cover ingress-side close and setup.

The [logical-close follow-up](../reports/audits/2026-10-22-security-deployment-close.md)
consumes completed/rejected clients into CQ-bound close operations, keeping the
same four-position budget until a checked Started/Unavailable logical receipt. Each close has
one five-second admission/observation deadline; queue-full rejection alone permits
50 ms admission backoff. Accepted close is never replayed. Failure, expiry or
rejected binding/wait aborts with exact owners retained. Existing close operations
drain on lifecycle shutdown without restarting their budgets. The synchronous
ID-restoration fallback remained a gap until the follow-up below. Other phases'
socket destructors, partial setup/open/listen and controller/render work remained
blocking contexts at that milestone. Native ordinary close succeeds; real held-close,
queue-saturation and abort-boundary injection remain unqualified. Initial ARM
and x86 HTTP abortive-peer controls exposed reaping before close and an erroneous
fail-closed response to ERR_BAD_SOCKET. The corrected producer distinguishes
unavailability from live foreign ownership, and full IDs no longer wrap or reuse.
Final fixtures retain an unactivated owner across an existing five-second test
and require its Unavailable receipt. Failed initial runs remain separate in
the evidence; no request/fixture deadline is widened.

The [synchronous-close follow-up](../reports/audits/2026-10-22-security-synchronous-close.md)
removes ID restoration and the second close from ordinary Drop. Both explicit
close and Drop consume the same private lease/ClosingSocket as reactor close.
A single five-second budget carries through admission and one owned reply wait;
queue-full-only 50 ms backoff stops within its first second. Final admission
rejection returns its actual error and retains the complete driver without retry
custody or charge refund. Accepted timeout, wait-admission rejection or invalid/
service/observation result aborts with the owner live. Host controls check these
boundaries; native ordinary TCP/UDP, Drop and reaped-owner success use the shared
observer. Real rejection/uncertainty fault injection and broader destructor
latency remain unqualified.

The [immediate-send follow-up](../reports/audits/2026-10-22-security-immediate-send.md)
removes the remaining 32-bit socket-selector truncation from immediate TCP/UDP
send. Both send operations now require the same exact CSOCKTX1 page and complete
ID. Immediate send adopts its complete incoming transaction once and retains its
read-only mapping through one enqueue attempt, then disposes mapping/backing
before reply authority. Retired packed pages and malformed frames reject before
backend enqueue; no old decoder or low-half lookup remains. Current callers
migrate together. This corrects identity/ownership boundaries without adding
readiness or completing management progress and pressure gates.

Its initial x86 security boot passed both application scopes, then panicked with
`AddressSpace(OperationDrainTimedOut)` while reclaiming the separate cancellation-
stress domain. The captured stack identifies `flood.stop`; it does not identify
which lease or namespace cleanup stage stayed pending. That failure remains
separate from any passing repeat, without a causal conclusion or wider timeout.

The [root-retirement diagnostic follow-up](../reports/audits/2026-10-22-root-retirement-diagnostics.md)
records each actual Pending branch in the existing exact-root owner, including
preflight lease counts, selected IPC token/epoch and memory blockers. A supervisor
terminal error reports that observation before retaining its closing owner.
This makes future recurrences distinguishable; it does not resolve the earlier
cancellation-stress timeout, change its budget or authorize reclamation/retry.

The [namespace selection correction](../reports/audits/2026-10-22-ipc-namespace-selection.md)
reproduces a separate stale-Pending race: a real peer close consumes a selected
token before cancellation preparation. Cleanup now rechecks exact presence and
rescans only after observed removal, while keeping every still-present claim
Pending. The same closing root, original charges and completion proofs remain
required. This removes one possible false timeout boundary; its relationship to
the historical live cancellation-stress timeout remains unproven.

The [listener setup follow-up](../reports/audits/2026-10-22-security-deployment-listener.md)
puts socket creation, listen and accept in the existing four-position reactor.
Every accepted phase binds to the same CQ. One setup deadline survives phase
changes and queue-full-only admission backoff; admitted idle accept is event-only.
Confirmed rejection transfers known sockets into the close pool. Uncertain
creation/setup retains the complete owner through abort, without replay. Shutdown
observes accepted creation/listen and cancels idle accepts into that pool under
the original budgets. Rendering, controller internals, other shutdown destructors
and all-position admission pressure remain separate boundaries.

The [controller-server relay follow-up](../reports/audits/2026-10-22-security-controller-relay.md)
removes synchronous DNS reply waits from all seven HTTP controls plus named status.
Eight owning positions and a 2,098,176-byte original/copy reservation bound active
requests. Accepted phases bind to the controller CQ; preflight and mutation share
one original server deadline, separate from the unchanged ingress deadline.
Only checked absence/newer-request evidence advances preflight. Failed or malformed
queries cannot become an empty-record fallback. Source mappings end before upstream
reply/loan authority, while a delivered DNS copy may retain its original charge
after timeout. Legacy upload/join/key handlers, synchronous bounded validation and
rendering, DNS execution and whole-service pressure remain separate boundaries.

Its first x86 security run failed the existing available-publication lookup
check before the new adapter. The capture lacks the returned scalar/attachments,
so neither its rejection cause nor a relationship to the relay change is known.
The same-source repeat advanced past that check; later fixture corrections and
passes do not resolve the original observation. The failed run remains separate
in the linked evidence, alongside the historical reclamation timeout.

The [release startup follow-up](../reports/audits/2026-10-23-security-release-startup.md)
removes both host workers' unconditional command-retry loops. `release-apply`
retains its exact envelope and one original deadline, retrying only before an
HTTP write or after a complete checked incompatible-target rejection. Uncertain
submission, malformed results and accepted readiness failures stop. This fixes a
fixture replay gap exposed by the controller relay run; it adds no reconciliation
service, peer authentication, physical cleanup proof or flood-capacity guarantee.

Remaining acceptance requires progress during saturated admission and slow
DNS/transport and legacy controller phases, sustained connection/packet flood, NIC failure
and lifecycle shutdown evidence on supported targets. Signatures checked after receipt do
not protect listener availability. Passing deadline/rejection probes do not
complete these fairness gates or SEC-07's essential-service pressure matrix.

## SEC-07: bounded resource admission

Implemented owners now cover capabilities, memory objects, heap/image frames,
private and fresh shared translation tables, runtime stacks, IOMMU backing,
IPC records/queues, CQ/completion records, timers, waiters, watches and observer
lists. Their individual contracts describe limits and retained backing. Stack
and translation-table admission listed as missing in the renewed audit has
since been implemented; do not repeat those historical gaps as current ones.

The [kernel stack range ownership follow-up](../reports/audits/2026-10-17-kernel-stack-ownership.md)
removes the guard-address `BTreeMap` identified in the October 17 review.
`Stacks` now retains a non-copyable exact kernel-range owner, and teardown
borrows its one-shot fence. Guard bookkeeping no longer allocates under arena
serialization. Existing pair/root/reservation retention and physical completion
rules remain unchanged. This correction does not complete SEC-07 or SEC-18.

The [memory sponsor preparation follow-up](../reports/audits/2026-10-18-memory-sponsor-preparation.md)
replaces lazy memory-budget account insertion with fallibly prepared nodes and
waiter/timer sponsors before root publication. One containing owner retains the
unpublished root, capability namespace and account metadata. The exact generation
is bound under publication serialization; runtime sponsor lookup is allocation-free.
Kernel sponsorship is prepared before schedulers/APs start. Closed old accounts
remain until their original memory charges end; detached metadata disposes after
the local ledger guard. General heap/aggregate limits and enclosing
caller/IRQ/destructor contexts remain open R7/R18 work. Principal history
preparation is covered below.

The [root policy/telemetry follow-up](../reports/audits/2026-10-18-root-policy-preparation.md)
prepares the existing launch-limit and domain-usage nodes before root publication,
retaining partial storage with the unpublished root. Exact-generation publication
only relinks admitted nodes. Sealed cleanup transfers both nodes through the
existing closing/final-root receipt; invalidation rejection retains them and
confirmed invalidation disposes them after local guards, before backing/slot
completion. No new registry, custody authority or numbered disposition change.
Snapshot allocations, aggregate budgets, list hold-time pressure and wider
destructor contexts remain open; principal history preparation is covered below.

The [signed-authority slot follow-up](../reports/audits/2026-10-18-root-authority-preparation.md)
prepares its storage with that same root transaction. Empty slots grant no identity;
the signed loader fills the exact assigned generation without allocation, rejecting
zero principals, duplicates and stale generations before mutation. Sealed cleanup
carries the populated slot in the same final-root receipt through invalidation.
Authority-node disposal is post-guard; aggregate budgets and wider caller/IRQ
contexts remain open. This does not change platform trust or enrollment.

The [principal-history preparation follow-up](../reports/audits/2026-10-18-principal-history-preparation.md)
replaces the two infallible stack/heap history maps with one shared admitted
record per signed principal. Every root prepares a candidate before publication;
identity installation either links it without allocation or keeps it in that
root's authority slot. Retirement only updates existing counters. Unused storage
follows C02/C03 through invalidation and explicit post-guard disposal. Published
history remains for the boot lifetime so restarts preserve their sizing inputs.
This addresses the high-water insertion gap above. The later storage-admission
batch below bounds history/candidate node bytes and counts; other metadata,
hostile-pressure hold times and outer contexts remain open. Finding dispositions are unchanged.

The [principal backing follow-up](../reports/audits/2026-10-23-security-principal-backing.md)
adds one shared 64 MiB/1,024-object ceiling per verified artifact principal for
memory-object, demand-heap and image/runtime backing. Generations and instances
share it; moved/retired backing retains its original sponsor. Binding is atomic,
precedes signed preparation and thread publication, and extends the existing
principal-history record. Failed cleanup retains aggregate charges. This is a
production enforcement change in R7-3, with [current scope](principal-backing-admission.md).
General kernel heap/metadata and other families still need aggregate principal
admission; concurrent pressure and essential-service guarantees remain R7-4/R7-5.

The [table/stack extension](../reports/audits/2026-10-23-security-principal-structure.md)
charges private translation pages and full user-thread stack capacity to that
same principal limit. Sponsor preparation now precedes even initial root backing;
constructor failures, cached prefixes and uncertain stack cleanup retain their
original shares. Data growth does not pay twice. This extends R7-3 enforcement
through two more resource families; shared kernel tables, IOMMU/device metadata,
general heap/metadata and the broader pressure/progress criteria remain open.

The [thread metadata extension](../reports/audits/2026-10-23-security-thread-metadata.md)
reserves exact context/node layout bytes and bundle counts before either allocator.
The existing principal-history record shares 1 MiB/128 bundles across instances;
node and ordinary limits preserve separate platform headroom. Original charges
follow delayed/failed retirement through actual context/node deallocation. Real
prepared-node exhaustion checks one kernel admission and subsequent recovery;
thread-table capacity, observer storage, general heap and sustained concurrent
progress remain gaps. Its final-layout x86 run also captured an unresolved
`UnmapFailed` panic in the scheduled IPC endpoint fixture after the new metadata
controls passed; preserve that failed run separately from passing executions. See [current policy](thread-metadata-admission.md).

The [mapping admission follow-up](../reports/audits/2026-10-23-security-map-admission.md)
removes cross-LP maintenance from a returned rejection before mapping-pin
admission. This prevents denied calls from consuming shootdown admission or
retaining otherwise-unused scratch because that maintenance failed. Admitted
prefixes still require confirmation. Kernel errors retain the per-call cause;
the earlier valid queued-loan failure remains causally unresolved. This narrows
an admission/availability path without closing SEC-07 or SEC-18.

The [principal-history storage batch](../reports/audits/2026-10-23-security-history-admission.md)
bounds published records and root candidates together: 1 MiB/4,096 nodes, with
ordinary work limited to 768 KiB/3,072. Classification comes from trusted signed
preparation or the unpublished root's translation account. Duplicate candidates
remain charged through failed invalidation; published records retain their charge
for the boot lifetime. Real total exhaustion still permits existing-root close
without new history storage. This qualifies that R7-2/R7-3 metadata family;
other root/control-block bytes, hold times and sustained progress remain open.
See [current policy](principal-history-admission.md).

The [root-record admission batch](../reports/audits/2026-10-24-security-root-metadata-admission.md)
adds byte/count admission for limits, usage and authority nodes, including
partial preparation before history admission. Each node keeps its original
trusted classification through publication, failed invalidation and actual
post-proof deallocation. Real one/two-slot exhaustion rejects partial root
publication; existing-root close proceeds at total exhaustion. Namespace/account
control blocks, other IPC metadata and sustained progress remain separate work.
See [policy](root-metadata-admission.md).

The [IPC namespace publication batch](../reports/audits/2026-10-24-security-ipc-namespace-publication.md)
prepares its node and both budget accounts fallibly in the existing root
registration transaction. A first exact-root operation lease protects publication
after lifecycle/table unlock; insertion only relinks prepared storage under IPC.
Missing and retired namespaces reject instead of recreating. Fenced cleanup
detaches the empty node and releases its account references after IPC unlock;
foreign charges keep their original accounts. Guarded per-capability
payload factories, namespace/account byte and principal ceilings, wider destructor
contexts and sustained progress remain open. See [current contract](ipc-namespace-publication.md).

The [pending-call storage batch](../reports/audits/2026-10-24-security-ipc-pending-storage.md)
retains the caller's exact root, reserves original call/reply counts, then prepares
the pending node fallibly outside IPC. Publication relinks that node; ordinary
rollback finishes the whole preparation after IPC unlock, root last. Existing
call/cancellation owners carry detached nodes through post-guard disposal and
refund only after node deallocation. Abandonment retains the containing owner.
Guarded per-capability payload factories, waiter/vector/control-block contexts, byte/principal ceilings
and sustained progress remain open. This advances R7-2 and C08/C18 without a
finding-count change; see [record policy](ipc-record-budgets.md).

The [reply-token storage batch](../reports/audits/2026-10-24-security-ipc-reply-storage.md)
prepares the second record node in that same charged call owner before authority
or attachment mutation. Failure returns the complete prefix after local guards.
Reply/cancellation and serialized close retain detached token nodes through
post-IPC disposal before refund; scalar completion fallback retains notifications
together with the node. Original claim-source classification survives promotion
and root reuse. Guarded per-capability payload factories, other factories/destructors,
byte/principal ceilings and pressure progress remain open. This continues
C07/C08/C09/C10/C18 without changing finding dispositions.

The [endpoint storage batch](../reports/audits/2026-10-24-security-ipc-endpoint-storage.md)
reserves endpoint/queue counts before fallible node, backing and observer-list
preparation outside IPC. Its complete owner retains the exact root through
revalidation, publication and post-guard unused disposal. Queue growth retains
both backings and the original sponsor/classification through unlocked
preparation and checked FIFO replacement; old backing frees after unlock.
Final references detach closed endpoint nodes into existing cleanup owners,
with the original record charge held through actual node free. Abandonment
retains complete preparation/retirement fields. This advances C08/C09/C10/C18
and R7-2; guarded payload/authority factories, other payload
contexts, byte/principal ceilings and sustained pressure progress remain open.
Finding dispositions are unchanged; see [policy](endpoint-budgets.md).

The [capability payload storage batch](../reports/audits/2026-10-24-security-ipc-capability-storage.md)
removes infallible per-capability map insertion after authority mutation across
endpoint, call, connection, returned-connection and receive publication. Original
shared admission precedes fallible payload-node allocation; complete preparation
stays in existing owners through post-guard unused disposal. Capability removal
pairs its payload with detached shared authority in the same temporary cleanup
list, outside IPC for disposal. Connection counts follow actual payload/node
free. Speculative receive rollback and scalar/claimed reply share this boundary.
Other capability factories still run under IPC; byte/principal ceilings, broader contexts and
pressure progress remain open. This advances C07/C08/C09/C10/C18 and R7-2 without
changing finding dispositions. See [contract](ipc-capability-storage.md).

The [unlocked call/endpoint preparation batch](../reports/audits/2026-10-24-security-ipc-unlocked-preparation.md)
retains shared-authority preparation in the same exact-root parents before IPC
admission. Captured-namespace validation and count reservation only relink under
IPC; payload allocation leaves it again. Call pending waiter-list construction
also leaves IPC, with original sponsorship. Shared quota rejects before those
payload/list factories. Ordinary unused disposal remains post-guard, root last;
fallback retains the complete prefix. Direct-grant/receive/return contexts and
mixed memory payload maps remain guarded work, alongside wider
byte/principal and pressure qualifications. This advances C18/R7-2 without a
finding disposition change.

The [borrowed publication batch](../reports/audits/2026-10-24-security-ipc-borrowed-publication.md)
removes the temporary authority vector from memory/IPC publication. Existing
reservation/escrow slices yield stable borrowed views for two passes under one
capability hold: validate the complete set, then mutate. Full prepared scalar
commit now publishes, installs admitted nodes and enqueues without heap work.
The mixed 257-member control brackets authority publication only; memory payload
maps still allocate afterward under their registry. Guarded factories, those
payload maps, wider contexts and aggregate byte/principal/pressure boundaries
remain open. This advances C18/R7-2 without changing finding dispositions.
See [contract](ipc-publication-batches.md).

The [memory-object storage batch](../reports/audits/2026-10-24-security-memory-object-storage.md)
removes global object-map insertion allocation after authority mutation.
Original page/object admission precedes fallible node preparation; direct/copy
publication relinks prepared storage with an identity claimed before publication.
Complete detached nodes remain in existing backing receipts through physical
release and post-registry disposal before original charge refund. The physical
attempt fence is recorded before allocator entry; failed/interrupted backing
fields cannot implicitly refund or clean up. Target capability/borrower maps,
other factory/outer contexts, metadata-byte/principal and pressure guarantees
remain open. This advances C05/C18/R7-2 without changing finding dispositions.
See [contract](memory-object-storage.md).

The [read-borrower storage batch](../reports/audits/2026-10-24-security-memory-borrower-storage.md) removes read-loan map allocation
after shared authority publication. Exact destination admission precedes fallible
node preparation outside the memory registry and source escrow; the existing
transfer owns cancellation/publication. Revocation retains its complete prior
map on failure/abandonment. Confirmed cleanup pairs detached borrower/authority
nodes through post-guard node disposal before original authority refund.
Namespace and final-object cleanup explicitly dispose the same nodes after their
existing proofs. Capability/namespace and mapping maps, byte/principal ceilings,
wider IRQ/guard/destructor contexts and pressure remain open. This advances
C05/C06/C18 and R7-2 without changing finding dispositions.

The [memory capability/namespace storage batch](../reports/audits/2026-10-24-security-memory-capability-storage.md) prepares both node types
fallibly after exact shared admission, before memory serialization and publication.
Complete mixed commit now relinks all memory payloads with the heap held.
Detached payload/authority nodes remain in the existing source, borrower,
namespace or complete object/backing owners through post-guard disposal; failed
physical release retains both original backing and authority charges. Ordinary
read-loan cancellation now drops unused borrower storage before destination
refund. Mapping/scratch metadata, source-escrow and wider caller/IRQ/destructor
contexts, byte/principal ceilings and sustained pressure remain open. This
advances C05/C06/C18 and R7-2 without a finding disposition change.

The [mapping-node/snapshot batch](../reports/audits/2026-10-25-security-memory-mapping-storage.md) removes allocating map-record
publication and snapshot growth under the memory registry. One unused preparation
owns both buffers outside that guard; captured revalidation precedes relink/pin
mutation. Existing pins/receipts retain complete detached records and exact peer
leases through uncertain cleanup; explicit release follows confirmation and local
guard exit. Published-prefix metadata survives rollback until maintenance, and
unmap keeps its record through scratch completion. Scratch namespace factories,
concurrent preparation bytes, wider caller/IRQ/destructor contexts, aggregate
metadata/principal admission and pressure remain open. This advances C05/C06/C18
and R7-2 without changing finding dispositions.

The [scratch storage batch](../reports/audits/2026-10-25-security-scratch-storage.md) removes allocating namespace insertion
and extent-vector growth beneath the scratch registry. One inert preparation
owns both nodes outside scratch/device holds, with the original root/MMIO claim
retained; generation and current first fit revalidate before relinking. Sealed
root cleanup carries complete scratch metadata in its existing receipt through
final invalidation, releasing post-guard before root backing. Atomic MMIO claim
admission also rechecks mapped state after its preliminary probe. Metadata-byte/
principal and peak preparation admission, scalar reservation identity, raw fixture
and broader outer contexts, pressure/fairness and historical failures remain open.
This advances C02/C03/C05/C13/C18 and R7-2 without changing finding dispositions.

The [scratch admission batch](../reports/audits/2026-10-25-security-scratch-admission.md)
adds 512 KiB/8,192 total and 384 KiB/6,144 ordinary requested-layout/count
ceilings before the two factories. Captured trusted classification, partial
preparation, losing candidates and detached root metadata retain original charges
through actual node deallocation. Real node/byte exhaustion, partial quota
rollback, trusted reserve, promotion/reuse and retained-root controls qualify
that boundary. Allocator overhead, aggregate principal metadata, wider contexts
and sustained concurrent progress remain open. This advances R7-2/R7-3 and the
same C02/C03/C05/C13/C18 owners; SEC-07/18 and finding counts remain unchanged.

The [copy snapshot batch](../reports/audits/2026-10-25-security-copy-snapshot-storage.md)
moves frame-vector capacity outside memory serialization, then revalidates exact
namespace generation, source object/size and every pin admission predicate before
allocation-free snapshot fill and pin publication. Ordinary unused preparation
disposes post-guard; bare preparation/pin abandonment retains metadata and the
original backing fence. Explicit unpin frees the snapshot before decrementing
that fence. Source close, writable-map/DMA competition, real ASID reuse and
guarded fallback controls qualify this C18/R7-2 boundary. Aggregate metadata/
principal admission, `ScopedCopyPin`/source-transfer destructors, DMA snapshots,
wider outer contexts and pressure remain open. SEC-07/18 counts are unchanged.

The [DMA snapshot batch](../reports/audits/2026-10-25-security-dma-snapshot-storage.md)
replaces infallible frame cloning under memory serialization after pin-counter
mutation. Shared inert vector capacity prepares outside that registry; captured
source and all direction/rights/loan/exclusion predicates revalidate before
allocation-free snapshot fill and both counter publications. Bare DMA-pin fallback
retains metadata and original fences; explicit snapshot free precedes unpin.
Existing complete DMA operation/maintenance owners and their typed hardware proof
remain authoritative. Concurrent snapshot bytes, aggregate principal metadata,
wider caller/destructor/IRQ contexts and sustained progress remain open. This
advances R7-2 and C14/C15/C18 without changing SEC-07/18 or finding counts.

The [mapping preparation batch](../reports/audits/2026-10-25-security-mapping-preparation-ownership.md)
keeps partial node/shared-snapshot fields inert on abandonment and explicitly
finishes every ordinary constructed return after local memory/table guards.
Final namespace-generation validation precedes snapshot fill and mapping/pin
publication. Native controls qualify captured reuse rejection, ordinary success/
prefix rollback disposal and guarded partial/full/published fallback. No new
root/pin, registry, recovery custody or physical/replay proof is added. Aggregate
metadata admission, broader outer contexts, pressure and historical failure
attribution remain open. This advances C05/C18 and G1/R7-2 without changing finding
dispositions or SEC-07/18 status.

The [shared snapshot admission batch](../reports/audits/2026-10-25-security-snapshot-buffer-admission.md)
adds requested buffer-byte/count enforcement for mapping/copy/DMA: 4 MiB/8,192
total and 3 MiB/6,144 ordinary. One typed buffer owner keeps its original charge
through preparation, publication, root retirement and abandonment, refunding only
after actual buffer free. Captured classification survives promotion and ASID
reuse. Actual buffer exhaustion exercises all three rejection paths, trusted
reserve, empty escrow and cleanup at exhaustion. This closes that family's
unbounded snapshot-buffer reservation gap under R7-2/R7-3, not the whole gates.
Per-principal/general metadata, allocator overhead, sustained EL0 pressure and
essential-service progress remain open. See the [acceptance map](snapshot-buffer-admission.md#acceptance-scope).
No finding disposition or SEC-07/18 status changes.

The [mapping-node admission follow-up](../reports/audits/2026-10-25-security-mapping-node-admission.md)
adds shared requested-layout/count ceilings before allocation: 1 MiB/8,192 total,
768 KiB/6,144 ordinary. Captured classification and original admission stay in the
same prepared/published/detached node until actual deallocation; promotion and
root reuse cannot refund live records. Real-node exhaustion, public map/map-any
denial, trusted reserve and unmap at total exhaustion qualify R7-2/R7-3 and C05/C18.
Other metadata families, aggregate principals/general heap and sustained concurrent
progress remain open. SEC-07/18 and finding dispositions remain unchanged.
The [resource-policy follow-up](../reports/audits/2026-10-25-security-resource-policy.md)
centralizes kernel hardening capacities, RAM ratios and related backing/local
ceilings in a typed compile-time policy. Operators can select an absolute-file
profile; compile-time validation rejects inconsistent settings. Existing defaults
and charge lifetimes remain. The Configuration manual chapter and
[guide](../guides/configuration.md) document dependencies and qualification.
Boot-resource installation, remaining service/protocol/structural inventory and
sustained concurrent progress remain separate; no finding disposition changes.

The [memory capability preparation follow-up](../reports/audits/2026-10-25-security-memory-capability-preparation.md)
retains partial payload/namespace nodes with exact authority/account references
before factories. Ordinary rejection/success explicitly consumes unused metadata
after local guards, storage before authority cancellation. Guarded bare fallback
performs no allocator/registry/Arc destruction; actual namespace retirement/reuse
and late cancellation preserve successor authority. Namespace teardown may drain
hidden authority while payload metadata remains retained; the node admission
follow-up below now charges that storage. Containing transfer/backing destructors
and broader contexts remain separate. This preparation correction advances C18/G1
and R7-2 without adding custody or changing finding dispositions.

The [memory-capability node admission follow-up](../reports/audits/2026-10-25-security-memory-capability-node-admission.md)
adds requested byte/count admission before payload and namespace allocation,
including unused candidates. Original charges follow the existing complete nodes
through publication, detached cleanup and abandonment until actual deallocation.
The central compile-time policy supplies 4 MiB/32768 total and 3 MiB/24576 ordinary
defaults. Real exhaustion, trusted reserve, two-node partial rejection, public
allocate/move/copy/loan denial and cleanup/root reuse at exhaustion qualify this
R7-2/R7-3 family. Aggregate principal/general-heap accounting, other metadata and
sustained concurrent progress remain open; SEC-07/18 stay partial.

SEC-07 remains partial until all of the following deliverables are accepted:

The [telemetry snapshot follow-up](../reports/audits/2026-10-18-telemetry-snapshot-preparation.md)
removes infallible thread/domain capture and encoding vectors from the statistics
syscall. One bounded request budget prepares scratch outside source guards;
capture growth and allocation rejection return unavailable without a prefix or
an output capability. Heap sampling retains the captured root generation and
validates its status mapping; retirement borrows its existing closing owner.
This qualifies those R7-1/R7-2 and C02/C18 boundaries, not aggregate heap/principal
admission, all downstream metadata, outer syscall contexts or pressure progress.
Finding dispositions remain unchanged.

| Deliverable | Completion evidence |
| --- | --- |
| R7-1: allocation inventory | Enumerate application-triggerable kernel allocation sites and identify their byte/count limits, sponsor, lifetime owner, rejection behavior and reserve classification. Include thread nodes, scheduler queues/migration storage, registry entries, namespace/sponsor metadata, callback captures and allocator bookkeeping. Explicitly justify bounded boot-only exclusions. |
| R7-2: remaining scheduler/registry storage | Prepare storage fallibly before publication. Every application-triggerable allocation rejects without a kernel allocation panic or partially published authority. Admission follows the actual allocation through detached work and weak references. Thread contexts and retirement nodes now have shared requested-byte/count admission by signed principal and node; table capacity, observer/weak storage and other registry families still need admission. |
| R7-3: heap and aggregate sponsor budgets | Shared signed-principal object/heap/image/private-table, full user-thread-stack and context/retirement-node admission is implemented; complete other families and bound general kernel heap bytes and aggregate logical-principal consumption across domain creation/restart and outstanding work. Nested allocations and retained quarantine consume the original limits; ASID reuse, transfer, promotion and logical retirement cannot refund live backing or gain platform classification. |
| R7-4: essential-service progress | Reserve usable allocation/admission capacity for trusted platform work. Demonstrate management, timer/scheduler and teardown progress during sustained ordinary pressure; separate pools alone are insufficient evidence of fairness. Rejected work must be bounded rather than busy-looping. |
| R7-5: pressure matrix | Run concurrent EL0 clients on all supported QEMU targets against ordinary and aggregate ceilings, allocation rejection, cancellation, weak retention, domain restart and generation reuse. Verify exact charges after confirmed cleanup and retained charges after uncertain cleanup. Record progress/latency against fixture deadlines chosen before the run. Include real backing pressure as well as counter-only saturation. |

Acceptance requires R7-1's inventory to have no unexplained application-triggered
allocation bypass. It does not require retroactively adopting inherited boot
frames, but their exclusion and fixed boot cost must be documented. New features
must extend the inventory and pressure matrix rather than silently bypass them.

Relevant contracts: [translation](translation-admission.md),
[stacks](stack-admission.md), [IOMMU](iommu-table-admission.md),
[completion records](completion-record-budgets.md),
[observer lists](observer-list-admission.md) and
[thread retirement](thread-retirement.md).

## SEC-18: quiescence and recovery

The [backing abandonment correction](../reports/audits/2026-10-26-security-backing-abandonment.md)
removes physical release, metadata destruction and refund from the memory-object
backing owner's Drop, including reservation-only prefixes. Ordinary preparation
rejection, last-pin cleanup and cancelled-copy release consume the same complete
owner explicitly after their local guards, preserving original charges and
publication/cleanup order. A native old-Drop negative control fails on premature
reservation refund; guarded data-owner abandonment retains backing and authority
past root retirement/reuse. This advances C05/C18 and G1; containing transfer
destructors and broader outer IRQ/guard/physical recovery qualification remain
open. SEC-18/07 and finding dispositions remain partial/unchanged.

Implemented paths retain exact roots and ownership through staged copy rollback,
IPC delivery/reply/cancellation, endpoint and namespace cleanup, memory/device
retirement, thread abort, cooperative runtime-page access and final root destruction. Backing is released only
after confirmed invalidation/hardware completion; uncertainty retains charges
and fences. QEMU CPU epochs, VT-d/AMD-Vi/SMMUv3 maintenance and the supported NVMe
reset path have execution evidence. Aborting threads is request submission, not
a quiescence receipt.

The [memory capability preparation follow-up](../reports/audits/2026-10-25-security-memory-capability-preparation.md)
also retains a fresh Arm panic in the existing first-entry exit-watch fixture:
the unchanged five-second wait returned false after new metadata controls passed.
A four-LP capture identifies the fixture but lacks its exact awaited target tuple.
Failure-only exact-target/typed-wait/live-staged diagnostics are now added; cause
and relationship to the memory change or historical failures remain unestablished.
Passing repeats cannot resolve this C16/G1/G7 observation or authorize cleanup.

SEC-18 remains partial. These deliverables track its remaining work and the
implemented R18-2 scope:

The [cleanup ownership and recovery strategy](cleanup-recovery.md) now maps
18 owner families, their current retry/retention boundaries and seven named
cross-category gaps. It defines shared custody/controller policy separately
from typed physical completion, and orders context qualification before adding
another registry. This is a source-reviewed planning baseline, not completion
of the full R18-1 call-chain inventory or implementation of a general controller.

The [heap/image abandonment correction](../reports/audits/2026-10-09-security-preparation-abandonment.md)
now removes allocator/pool access and logging from that preparation fallback,
and kernel-range fallback no longer logs. Ordinary physical rollback still
borrows the table. The [table/raw-frame follow-up](../reports/audits/2026-10-09-security-table-abandonment.md)
also removes cleanup from private/shared table and raw-frame fallback, preserving
explicit normal cancellation and Arm hardware-tag rejection. The
[stack preparation follow-up](../reports/audits/2026-10-09-security-stack-preparation-abandonment.md)
removes physical/lease/pool cleanup from initial/growth and unpublished-slot
fallback, while retaining explicit ordinary cancellation. The
[published-pair follow-up](../reports/audits/2026-10-09-security-published-stack-retirement.md)
removes physical stack/context destruction, adds Arm pinned IRQ-enabled reaping
and retains failed complete pairs in their existing nodes without retry. Ordinary
rejection runs explicit cleanup after local guards; context metadata allocation
precedes generation/backing. The [complete thread metadata follow-up](../reports/audits/2026-10-12-thread-metadata-retention.md)
now also removes callbacks, wait cancellation, usage refund and implicit field
destruction from `Thread` fallback. Explicit consuming metadata disposal follows
confirmed stack-pair release, with its retirement node retained until return.
Guarded kernel/user abandonment retains the exact user root and original
admission. Outer ordinary caller masks, explicit callback contexts, backend
contexts, growth's enclosing thread-table guard and the complete call-chain
inventory remain required; C16/G1 and SEC-18 remain open. The
[reaper readiness follow-up](../reports/audits/2026-10-12-reaper-readiness.md)
now replaces the per-LP one-millisecond idle poll with an owned event-only
retirement-head subscription and deferred post-guard notification. Failed pairs
are nonready; recorded wait-admission rejection alone uses a 10 ms backoff.
This changes no physical completion or retry rule, and broader admission
pressure/progress remains G7. The
[transfer-close readiness follow-up](../reports/audits/2026-10-12-memory-close-readiness.md)
replaces the explicit memory source-escrow yield loop with a retained-root,
owned wait. Charged source/disposal storage precedes registry publication;
producer-LP deferred transport shares C16 delivery and disposes metadata only
after producer/local guard exit. This changes no pin/loan rejection, physical
receipt or retry policy. Initial scheduled-watch fixture failures are preserved
separately from corrected runs; broad G1/G4/G7 and SEC-18 remain open.
The [IPC claim-readiness follow-up](../reports/audits/2026-10-12-ipc-close-readiness.md)
replaces explicit call/reply/source and owned endpoint-front yield waits with
exact-root and claim-epoch subscriptions. Completion, rollback and failure wake
only revalidation through the existing producer-LP transport. Uncertain loans
stay fenced, abandoned claims retain their owners, and no physical retry or
terminal call result is inferred. Namespace/lease drains and broader C07/C08/C09
metadata/guard qualification remain open; SEC-18 is still partial.
Its final-source Intel guest also stopped once at NIC DMA-domain creation with
DmaUnavailable; the identical-artifact repeat passed. That historical capture
establishes no backend cause. The
[startup preparation follow-up](../reports/audits/2026-10-12-dma-startup-preparation.md)
preserves typed stage/cause and consuming cancellation outcomes, and prepares
all boot hardware grants before any sibling driver starts. Real claimed-unit
rejection is now distinguished from other failure classes without authority
mutation. General dynamic admission/readiness, historical causality and broader
pressure remain open; diagnostics and passing guests authorize no uncertain
creation retry.
The [IOMMU preparation follow-up](../reports/audits/2026-10-09-security-iommu-preparation-abandonment.md)
also removes physical/pool cleanup and implicit ledger heap deallocation from
C15 fallback. Private construction and ordinary metadata/allocator rejection use
explicit cancellation across all three backends. Ordinary private/creation
rollback and hardware maintenance still retain backend serialization; G3 and the full
call-chain inventory remain required. Finding states are unchanged.
The [detached-domain follow-up](../reports/audits/2026-10-09-security-iommu-detached-release.md)
now moves explicit DMA destruction's physical table release outside backend
serialization, preserving the complete owner and existing slot/requester claim.
Partial physical failure restores that frozen owner without allocation;
abandonment retains metadata/pins and fences the slot. Remaining backend waits
and creation rollback require command-engine phase ownership, so G3 and SEC-18 remain
partial. No custody registry or new owner-family row is added.
The [command-maintenance follow-up](../reports/audits/2026-10-09-security-iommu-command-maintenance.md)
now moves explicit destruction's actual command engine and complete domain before
unlocked hardware maintenance. Empty engine admission fences ordinary mutation;
timeout restores the exact engine/domain, abandonment retains them and fences the
unit. Physical completion remains available during unrelated maintenance.
Creation/map/unmap/initialization waits and private rollback remain G3 work;
SEC-18 remains partial and finding counts are unchanged.
The [DMA grant publication follow-up](../reports/audits/2026-10-09-security-dma-grant-rollback.md)
removes hardware destruction/logging and implicit reservation cleanup from grant
fallback. It retains the exact root, reservation and backend rollback obligation
together; ordinary publication rejection rolls back explicitly after local
guards leave, while failure/abandonment retains all original admission. Backend
private construction/reset contexts, outer caller qualification and general
publication metadata admission remain G1/G3/G4. Finding states are unchanged.
The [published-creation follow-up](../reports/audits/2026-10-09-security-dma-creation-rollback.md)
now records the admitted domain in that enclosing owner before any reachable
descriptor, including backend error. Published creation rejection uses unlocked
destruction after local lifecycle/device/config guards leave; confirmed completion
refunds authority and closes the root, while failure retains the full obligation.
Real QEMU NVMe rejection after actual initial configuration checks guard/config
availability, disabled bus mastering and exact charge refund. Initial creation/
reset waits, private construction and full outer contexts remain G1/G3 work;
SEC-18 and finding counts remain unchanged.
The [private-construction follow-up](../reports/audits/2026-10-09-security-dma-private-rollback.md)
now carries root/CD/MSI prefix and metadata rejection in a complete typed grant.
Private physical release and metadata disposal leave local lifecycle/device/
backend/config guards first, then confirmed completion refunds authority/root.
Rejected/abandoned cleanup retains all original charges and dependencies without
retry. Real prefix rejection and guarded partial-release probes qualify this
boundary. Initial waits/reset, unit-private initialization and metadata allocation
remain G1/G3/G4/G7 work; SEC-18 and counts are unchanged.
The [unit-initialization follow-up](../reports/audits/2026-10-09-security-iommu-unit-initialization.md)
now claims the existing typed unit slot before unlocked boot preparation/control/
waits/cancellation and removes lazy initialization below ordinary DMA requests.
Uncertain control, published error, failed release or abandonment retains the
complete unit and claim without replay, even when its new backing is private.
Real allocation-prefix and successful-wait probes plus synthetic terminal
retention qualify this boundary. Initial domain creation/reset, map/unmap and
metadata/outer-context work remained open at that checkpoint; SEC-18 and counts
were unchanged.
The [map/unmap follow-up](../reports/audits/2026-10-09-security-dma-mapping-maintenance.md)
now retains the exact public root/capability claim and complete domain, actual
engine and pending data pin through unlocked walking and maintenance. Exact
restoration precedes confirmed post-guard unpin. Rejected unmap quarantines its
pin without exceptional allocating reinsertion; abandonment retains every field
and claim. Real QEMU boundary probes and rejected-completion/retirement evidence
qualify this scope, while the synthetic complete-owner fixture retains one
domain-table frame, two independently charged data frames and their exact root.
Initial creation/reset, general metadata admission, full outer contexts,
concurrent/outstanding-I/O evidence and custody remain open. SEC-18 remains
partial and the finding counts are unchanged.
The [PCI reset-claim follow-up](../reports/audits/2026-10-09-security-pci-reset-claim.md)
now retains the exact endpoint config/BAR/ECAM fence in the original DMA grant.
Config/device holds leave controller polling; ordinary config/MSI and overlapping
MMIO authority reject the claim. Explicit activation follows exact busy-cap
publication; confirmed backing rollback precedes reset cancellation. Hardware
uncertainty/abandonment retains root, reservation and endpoint claim without
destructor writes. Real staged exclusion/reset/publication probes and synthetic
complete-grant guarded retention qualify this scope. Wider lifecycle/backend
creation guards remained open at that checkpoint and are addressed below;
preparation metadata, concurrency/outstanding I/O and custody remain open; SEC-18 and the finding counts remain unchanged.
The [complete-unit creation follow-up](../reports/audits/2026-10-09-security-dma-creation-phases.md)
now releases local lifecycle/backend serialization through reset, construction
and initial configuration. The original grant retains its exact root/reservation;
publication rechecks captured generation and closing state. The existing unit
slot claims its entire installed payload and rejects absent engines or detached
domain cells before extraction, preserving physical finalization. Exact ordinary
restoration precedes rollback; abandonment retains the whole unit and grant.
Real boundary and staged-close fixtures plus guarded synthetic retention qualify
this scope. General metadata admission, inner fallback/wider contexts,
concurrency/outstanding I/O and custody remain open. SEC-18 and finding counts
remain unchanged.
The [DMA close-claim follow-up](../reports/audits/2026-10-09-security-dma-close-claim.md)
now keeps the original device record/authority in place through backend destruction,
using the existing exact-root `DmaOperation`. Ordinary rejection restores the
public claim without allocating reinsertion; confirmed success consumes authority
only after backend completion. Abandonment retains root/claim. Heap-held synthetic
rejection and real maintenance/physical probes qualify this scope; general grant
metadata admission, successful removal/destruction, wider callers and concurrency
remain open. SEC-18 and finding counts are unchanged.
The [device-registry storage follow-up](../reports/audits/2026-10-09-security-device-registry-storage.md)
now prepares namespace/payload nodes fallibly before local guards/hardware in
shared exact-root/reservation `GrantAdmission`. Publication/detachment relinks
existing retirement-list nodes; confirmed close and namespace completion dispose
them explicitly after guards. DMA retains its typed hardware obligation alongside
that shared admission. Host allocator tracing and guest first/second-node
rejection, heap-held publication and real post-guard disposal qualify this scope.
Unified capability/backend metadata, byte/principal limits, wider contexts and
pressure/concurrency remain open; SEC-07/18 and finding counts are unchanged.
The [unified namespace-storage follow-up](../reports/audits/2026-10-09-security-capability-namespace-storage.md)
now prepares its owning node/account before user ASID publication, shares
`AdmittedMap` with device storage, and disposes complete detached namespaces after
`CAPABILITIES` unlock. Failure and guarded abandonment tests preserve exact
account identity and charge lifetime. Individual authority-record metadata,
outer contexts, backend storage and aggregate byte/progress admission remain
open; SEC-07/18 and finding counts are unchanged.
The [authority-record storage follow-up](../reports/audits/2026-10-09-security-capability-record-storage.md)
extends that same shared machinery to all six kinds. Fallible preparation precedes
capability serialization and serial/charge mutation. Detached nodes preserve
original charges until explicit disposal outside `CAPABILITIES`; batch moves
retain their source node in the containing transfer through payload completion
and memory-registry unlock. Guarded preparation/retirement abandonment preserves
two original ordinary charges through root teardown/reuse. Active token Drop and
outer lifecycle/IPC/device contexts, backend metadata, byte/principal admission
and pressure/concurrency remain open; SEC-07/18 and finding counts are unchanged.
The [device authority-context follow-up](../reports/audits/2026-10-09-security-device-authority-context.md)
now prepares unified authority inside the existing grant before its local guards,
retaining captured generation and memory-budget policy checks. Close detaches
authority into charged metadata owners and disposes it after device/lifecycle
unlock; non-DMA close composes root, descriptor and both metadata owners through
invalidation/scratch completion. Rejection/abandonment keeps five additional
exact root leases and original authority charges in explicit terminal probes.
Heap-held admission/detach and real NVMe metadata boundary probes qualify this
scope. Other authority callers, backend metadata and complete outer-context,
byte/progress/custody work remain open; finding counts are unchanged.
The [backend registry-storage follow-up](../reports/audits/2026-10-09-security-backend-registry-storage.md)
now admits domain cells, requester fences and VT-d context metadata in typed
preparation inside the original grant before reset, IDs or physical domain backing.
Publication relinks shared admitted nodes and reuses the exact zero requester
fence; destruction keeps the cell/source claim through tables and all data unpins
before post-guard node disposal. Guarded complete-grant abandonment retains one
additional exact root/reservation and unused metadata without new IOMMU table/data
charge. Real node rejection and allocation/disposal entry probes qualify this
scope. Per-domain mapping/walker metadata, other authority callers, general heap/
principal admission, wider contexts, concurrency and custody remain open; SEC-07/18
and finding counts are unchanged.
The [mapping/cache metadata follow-up](../reports/audits/2026-10-10-security-dma-metadata-storage.md)
now prepares shared mapping nodes before data leaves, retaining the exact pin
and original node through detached maintenance or allocation-free quarantine.
SMMU cache misses admit metadata before branch linking and retain unused storage
through partial walks; confirmed table completion permits post-guard disposal.
Guarded complete-owner retention includes the new nodes without adding root,
authority, table or data-frame retention. Heap-held adapters and real node
rejection/context probes qualify the local storage boundary. Table ledgers,
other authority callers, broader heap/principal admission, enclosing contexts,
concurrency and custody remain open. SEC-07/18 and finding counts are unchanged.
The [mailbox publication-context follow-up](../reports/audits/2026-10-10-security-mailbox-publication.md) now prepares shared authority,
payload nodes and budget in one exact-root owner before local lifecycle/mailbox
guards. Explicit close retains both charged nodes through post-guard disposal,
then releases its root; both fallbacks retain every field. Storage rejection,
receiver reuse, heap-held relinking, ordinary cancellation, staged-close rejection
and guarded complete-owner abandonment qualify this scope. Final-root metadata
release is qualified by the follow-ups below; queue/account metadata, principal
and aggregate heap admission, other authority callers and wider contexts remain
open. SEC-07/18 and finding counts are unchanged.
The [final-root metadata follow-up](../reports/audits/2026-10-10-security-root-metadata-retirement.md) now carries detached mailbox/queue
payload and the complete unified authority namespace in the existing closing and
retired-root owners. Original charges survive final invalidation rejection and
bounded custody; confirmed invalidation permits post-guard metadata release
before the one-shot physical walk. Actual custody retry, stale generation,
guarded attempt abandonment and existing partial physical rejection qualify
this scope. Queue/account metadata, principal admission, completion/bookkeeping,
other authority callers and broader context/admission remain open. SEC-07/18 counts are unchanged.
The [word-queue storage follow-up](../reports/audits/2026-10-10-security-mailbox-queue-storage.md) now prepares admitted namespace nodes and queue backing outside local guards
in an exact-root operation, revalidating captured identity before relinking.
Established sends need no fresh storage. Losing creators retain unused fields
until post-guard completion; final teardown retains the complete node in the
existing root receipt. Node rejection, FIFO/backpressure/wrap, heap-held
publication/final detach, retired/staged/stale-root rejection and guarded
abandonment qualify this scope. The follow-up below qualifies backing
construction/set-byte admission; metadata/principal/aggregate heap admission,
completion/bookkeeping and other authority contexts remain open. SEC-07/18 and
finding counts are unchanged.
The [word-ring backing follow-up](../reports/audits/2026-10-10-security-mailbox-backing-admission.md) now replaces the word API's infallible queue factory with fallible fixed-ring
allocation. Its original set/requested-byte charge uses the same captured
mailbox family account for legacy and capability callers, including competing
preparations. Byte/set ceilings reserve a shared platform progress pool; original
classification persists. Rejected/losing preparation refunds after backing
release; existing final-root custody retains charge through failed invalidation
and terminal abandonment. Existing physical rejection checks metadata/queue
charges have ended before its one-shot owning walk. No extra retained root or
quarantine episode is added. Metadata/principal/aggregate heap budgets, other
authority contexts and completion/bookkeeping remain open. SEC-07/18 counts are
unchanged.
The earlier Intel user-stack lease timeout reproduced during the initial
IOMMU follow-up run and remains unresolved. Exact staged-node outcome snapshots
and independent atomic user-retirement/x86 shootdown observations improve evidence. Fresh-run success does not close C16/G1/G2/G7 or SEC-18.
It also recurred on AMD during the published-creation follow-up (14 passed/one
failed; unchanged-artifact repeat 15 passed). A fresh 16-run sequential and
three-way concurrent x86 sweep on the pre-correction tree did not reproduce
the timeout; the
[timeout investigation](../reports/audits/2026-10-10-security-user-stack-lease-timeout.md)
adds bounded pre-release invalidation re-confirmation for the only identified
transient failure class while deterministic rejections still retain
terminally. The historical cause is not established, so C16/G1/G2/G7 and
SEC-18 remain unchanged. The
[abort-sweep handoff correction](../reports/audits/2026-10-09-security-abort-handoff.md)
now defers the caller until peer requests finish and masks only its exact local
self-request/root-operation completion. A real fault fixture forces eight
pre-handoff scheduling boundaries and requires successful root teardown.
This closes the identified self-request window without establishing it as the
cause of those historical timeouts. The [executor-owner follow-up](../reports/audits/2026-10-09-security-abort-executor.md)
now defers mutual/remote requests throughout an admitted sweep, including timer
wait/wake, before root completion and executor release. Deterministic simultaneous
faults and scheduled ordinary rejection cover that boundary. The
[2026-10-10 progress verification](../reports/audits/2026-10-10-security-progress-verification.md#failed-amd-vi-abort-handoff-fixture)
also records a fresh AMD-Vi panic in the handoff fixture's immediate lifecycle
`try_lock` assertion. The [follow-up](../reports/audits/2026-10-10-security-object-store-follow-up.md)
replaces immediate global availability with atomic current-LP ownership checks
and tests deliberate remote contention, local ownership and IRQ preservation.
The original holder/cause is unresolved; this qualifies the narrower probe
without explaining historical lease timeouts. Broader retained
kernel operations, outer caller qualification and historical timeout causation
remain open; finding counts are unchanged.

| Deliverable | Completion evidence |
| --- | --- |
| R18-1: locking and destructor inventory | Partial owner-family/context baseline: [C01–C18 and G1/G3/G4](cleanup-recovery.md). Complete every production call chain and implicit field drop, including wider scheduler/observer/allocation paths. Remove allocation, arbitrary callbacks, blocking hardware maintenance and physical destruction from masking serialization. Include ARM's enclosing reaper IRQ state. No destructor may assume that its caller holds no unrelated lock. |
| R18-2: cooperative shutdown ownership | Implemented runtime-page retention: exact root admission and fixed image-leaf identity checks precede cooperative drain/status access; force publication borrows its sweep lease. Deployment claims release registry guards before admission. Closing/reused roots and mismatched frames reject before access; ordinary completion releases the lease and abandonment retains image backing. See the [runtime-page contract](live-address-space-operations.md#supervisor-runtime-page-access) and [execution report](../reports/audits/2026-10-08-security-cooperative-pages.md). |
| R18-3: retained-owner recovery | Partial implementation: eight fixed slots retain complete detached roots rejected by final invalidation, with exact serial tickets, two explicit retries, unlocked physical work, masked-caller rejection, terminal abandonment/partial-release states and capability-scoped observer counts. See [final-root recovery](root-recovery.md). [G2/G5/G6](cleanup-recovery.md#named-gaps-and-acceptance-gates) track typed owner continuity, supervisor reconciliation and common authorized custody. Other receipt kinds and stronger reset/reboot qualification remain required; never clear fences or decrement counts to manufacture progress. |
| R18-4: QEMU device coverage | Extend reset contracts to supported VirtIO, AHCI and NIC paths where applicable. Exercise active outstanding I/O, withheld hardware completion, rejected reset, old-MMIO authority, restart and reassignment. The existing NVMe fixture enables real queues but submits no outstanding I/O; that gap remains. Unsupported devices must remain fenced and explicitly excluded from recovery support. |
| R18-5: concurrent recovery matrix | On Intel VT-d, AMD-Vi and Arm SMMUv3, combine peer/root close, loan/copy/DMA activity, thread switching, actual lost/stale CPU acknowledgements, hardware timeout, interrupted publication and partial physical-release rejection. Verify guard availability, no early pin/refund/reuse, later essential progress, and exact baseline recovery where confirmed completion permits it. |
| R18-6: physical-platform qualification | For each claimed production device/platform, establish CPU/device reset and drain guarantees, interrupt remapping/ATS policy and negative execution evidence on that hardware. QEMU evidence cannot satisfy this criterion. |

R18-1–5 define the **QEMU milestone**, matching the user's selected first target.
Completing it must not close the broader SEC-18 finding while R18-6 is unmet.
Physical qualification is a later platform milestone requiring a selected device
and platform. This separation limits the immediate test matrix without inventing
a physical-hardware guarantee.

Relevant contracts: [hardware quiescence](hardware-quiescence.md),
[kernel frame retirement](kernel-frame-retirement.md),
[address-space retirement](address-space-retirement.md),
[live operations](live-address-space-operations.md) and
[memory-object retirement](memory-object-retirement.md).

## Production security milestones

The [exact boot-receipt follow-up](../reports/audits/2026-10-19-security-boot-receipts.md)
moves the fixture's exact installed-state check into shared `verify_for_boot`.
The kernel requires its distinct `VerifiedBootPolicy`; a generic enrollment or
rotation receipt cannot be passed directly to preparation. The existing native
fixture already rejected uninstalled successors. This strengthens the common
API contract without implementing protected state, serialization or custody;
SEC-04 remains partial and production remains disabled.

The next priority is trust and authenticated communication. Work on SEC-07/18
continues against the finite criteria above; it does not substitute for these
missing boundaries.

The [27 October native management TLS migration](../reports/audits/2026-10-27-security-management-tls-native.md)
replaces both plaintext listeners with mandatory TLS 1.3 and exact enrolled
reader/administrator identities. One signed HPKE boot resource supplies server
credentials, client roots, roles and all eleven adjustable limits from the same
prepared kernel trust owner. Handler authorization precedes controller dispatch;
signed mutation bodies remain required. Verified host/browser setup and native
CQ ownership are documented in the [management contract](management-tls.md).
Final ARM single-node VIP and x86 direct-node tests reject unauthorized clients,
reader mutations, wrong names/roots and plaintext while preserving healthy
progress and original deadlines. SEC-10 moves to **partial**, not corrected:
protected credential installation/custody, native resource replacement/revocation,
leader movement and broader fault/pressure qualification remain. Production stays
disabled; SEC-04, SEC-08, SEC-09 and SEC-15 retain their separate gaps. The
historical [engine component report](../reports/audits/2026-10-27-security-management-tls-engine.md)
continues to describe its earlier unwired checkpoint.

The [management startup follow-up](../reports/audits/2026-10-27-security-management-startup.md)
corrects a missing-profile launch regression: deployment's entry frame could
exhaust the general stack before credential rejection. Both launch paths now use
the same central management stack policy. It also fixes x86 host acceptance so
a failed identity probe cannot be masked by later healthy metrics. Native startup
evidence and its transport-observation limits are recorded separately from the
earlier positive HTTPS campaign. SEC-10 remains partial; finding counts and the
protected-installation, time and cluster-peer gaps are unchanged.

The [management rotation follow-up](../reports/audits/2026-10-27-security-management-rotation.md)
qualifies x86 signed-policy revision 7-to-8 rebuild/restart, retaining both client
CA roots while replacing reader/administrator leaf enrollments. Both old roles
reject on both interfaces with native client-policy events and healthy replacement
controls. A valid uninstalled successor, a retired module and an old-policy-bound
TLS resource reject at their exact pre-publication gates. The same persistent
firmware/data images also accept an older still-authorized kernel and its old
operators: compiled fixture state supplies no platform rollback resistance.
Five expected cases pass and one replay limitation is observed. This supersedes
the native report's pending x86 restart/revocation qualification only. Protected
installation/custody, ARM rotation, live session retirement,
multi-node movement and broad fault/pressure campaigns remain; SEC-04/10 remain
partial and production remains disabled.

The [management validity follow-up](../reports/audits/2026-10-27-security-management-validity.md)
qualifies fresh-handshake date enforcement on both native architectures. The
complete resource deliberately enrolls expired and future-dated readers and
administrators under the ordinary trusted CA. Sixteen date/interface controls
reject with exact native validity events and healthy-reader recovery. This
qualifies the existing validation path, not a discovered expired-credential
acceptance defect. Established-session/issuer/server expiry, native time-source
failure and authenticated time remain separate; SEC-10 stays partial and
SEC-09 stays open. No finding count changes.

The [management leader-loss follow-up](../reports/audits/2026-10-27-security-management-failover.md)
corrects fixed-window election jitter; it now scales with the configured base
timeout, with saturating deadline/backoff arithmetic. A deterministic delayed-link
regression fails on the parent and passes after correction. Native ARM survivors
elect term 2 after leader loss, accept a second signed policy through a follower
and both read back generation/sequence 2. Full and direct-only campaigns still
fail on subsequent TLS handshakes, including a healthy direct-node reader; only
two direct client-rejection/recovery controls complete. These failures remain
explicitly unresolved, alongside an intermittent pre-kernel firmware stall.
SEC-10 remains partial; no numbered finding or production gate is closed.

The [public-key input follow-up](../reports/audits/2026-10-21-security-public-key-input.md) corrects a host-tool input gap:
ordinary public-key file arguments now share trust-policy's bounded regular-file
admission and hex parser. Baseline oversized-comment and symlink controls fail;
corrected tests and actual operational CLI controls pass. This is host file
admission evidence. Management flood fairness, authenticated trust provisioning
and secret-state acceptance criteria retain their current finding status and
counts.

The [operational-input follow-up](../reports/audits/2026-10-21-security-operational-input.md) bounds ciphertext envelopes,
bundles and bundle-signing references before decoding or notification. The
existing binding maximum rejects before referenced reads. Baseline CLI controls
retain five accepted symlink paths and late size/count rejection; corrected
controls include no-output and no-ingress-connection checks. These host input
corrections leave management fairness, trust, secret-state and overall finding
counts unchanged.

The [control-input follow-up](../reports/audits/2026-10-21-security-control-input.md) applies the same bounded file admission
to deployment, release, shutdown and ingress inputs, including notify/apply and
release-signing references. Three baseline controls preserve accepted symlinks,
late size rejection and a referenced read before descriptor-count rejection.
Corrected controls pass without changing the signed formats or receiver checks.
Raw ELF/hash inputs, management fairness, trust and secret-state work remain
separate; overall finding counts are unchanged.

The [artifact-input follow-up](../reports/audits/2026-10-21-security-artifact-input.md) aligns host ELF input and signed-output
sizes with the existing 4 MiB launch bound and checks variant count before image
reads. The arbitrary-file hash command now streams through an erasing 16 KiB
owner. Baseline controls retain accepted aliases, late size/count rejection and
a signed output exceeding the launch limit. Independent Python vectors and
allocation observations qualify the hash loop. The artifact-edit follow-up
below addresses write redirection; atomicity, physical erasure, custody and
broader findings remain separate; counts are unchanged.

The [artifact-edit follow-up](../reports/audits/2026-10-21-security-artifact-edit.md) retains the writable descriptor and
prepared image through one consuming ELF rewrite. Replacing or removing the
path cannot redirect or recreate publication. Deterministic phase controls
preserve three baseline failures; corrected ownership, bounds and rejection
controls pass. Pathname continuity, concurrent inode writers, partial writes
and durable atomic publication remain separate. Finding counts are unchanged.

The [control-output follow-up](../reports/audits/2026-10-21-security-control-output.md) routes deployment, release and shutdown
signing through exclusive creation. Actual CLI controls preserve nine baseline
overwrite cases across regular files, symlinks and hard links; corrected runs
reject them and preserve their bytes. Fresh signed output bytes match baseline.
Boot fixtures use fresh private directories for signed records. The retention
follow-up below removes pathname cleanup; durable atomic publication remains open.

The [output-retention follow-up](../reports/audits/2026-10-21-security-output-retention.md) removes the signer's four failure-path
pathname deletions. Created outputs stay available for inspection, including
partial writes and private keys whose public output failed. One shared pair
writer reports incomplete generation. Actual CLI file-size-limit controls
retain fourteen baseline retention failures; corrected debug/release controls
pass. This removes deletion redirection by avoiding deletion; atomic publication,
disk erasure, trust custody and SEC-15 remain separate. Counts are unchanged.

The [fixture-publication follow-up](../reports/audits/2026-10-21-security-fixture-publication.md) stages the local deployment bundle
inside its existing private run directory before replacing the named output.
Host controls qualify link-target preservation, old readers, partial-write
failure and rejected publication. Both actual helper runs produce the same
signed bundle bytes as preceding evidence. This qualifies fixture preparation;
signer durability, production trust and finding counts remain unchanged.

The [deployment-generation follow-up](../reports/audits/2026-10-21-security-deployment-generation.md) binds descriptor-apply readiness
to each admitted generation. Stale readiness stays pending, matching readiness
completes and supersession fails. Malformed records stop without POST replay.
Live-CLI loopback controls preserve the baseline false successes. The
[release-generation follow-up](../reports/audits/2026-10-21-security-release-generation.md) carries a complete digest-bound component receipt from catalog apply through
local/follower admission and HTTP to the host. It also corrects capacity charges
that release builds skipped inside `debug_assert!`.
The [host deadline follow-up](../reports/audits/2026-10-21-security-ingress-deadline.md)
carries one checked budget from preparation through admission and observation,
uses numeric endpoints and bounds each connect/write/read by the time left.
Status polling, blocking host I/O and ingress peer authentication remain separate.
Finding counts are unchanged.

The [named-status follow-up](../reports/audits/2026-10-21-security-named-status.md)
uses the existing rollout codec for standalone read-only and waiting status.
Malformed observations stop immediately. Its shared reader also preserves body
suffixes after additional HTTP separators and rejects Unicode whitespace that
could previously be stripped before apply's codec. Invalid names reject before
waiting or connecting. Current-name observation remains separate from admission
binding and grants no mutation or cleanup authority. Counts are unchanged.

The [response-reader follow-up](../reports/audits/2026-10-21-security-ingress-response.md)
shares complete-body extraction across every signer ingress consumer. All scalar
notification success records require a positive generation; release preserves
its full receipt validation. Policy observations validate their complete current
record before output. Malformed success remains uncertain without POST replay;
record validation supplies no peer authentication or local UTC authority.
Finding counts are unchanged.

The [SEC-15 hash-state follow-up](../reports/audits/2026-10-19-security-hash-state-erasure.md)
adds erasure of shared SHA state/compression scratch and one owned HMAC result
path for SigV4 and SCRAM. The [bounded inventory](secret-state-lifetimes.md)
retains library state, scalar/compiler copies and wider connector/tool lifetimes
as open work. SEC-15 remains partial; finding counts are unchanged.

The [SCRAM phase follow-up](../reports/audits/2026-10-19-security-scram-phases.md)
then replaces the mutable exchange with consuming initial/final owners.
Challenge processing releases the exchange-local password; final verification
consumes its proof and expected signature on success or rejection. The existing
connector already returned on authentication errors; this strengthens its
library contract and shortens the transient copy's lifetime. Wider credential
and third-party state remain open.

The [HPKE erasure follow-up](../reports/audits/2026-10-19-security-hpke-erasure.md)
corrects a dependency feature gap: HPKE disabled X25519 defaults and the selected
private/shared owners retained their fields after Drop. Erasure is now explicit
and guarded at build time. Generated recipient private bytes also transfer as
a zeroizing owner. SEC-15 remains partial; library scratch, TLS session state,
compiler copies and recipient custody retain their separate acceptance work.

The [Base64 scratch follow-up](../reports/audits/2026-10-19-security-base64-scratch.md)
then puts the SCRAM helper output under zeroizing ownership before decoding or
encoding. Late rejection erases its prefix, and one reservation removes decoder
growth. Existing successful callers already wrapped their results; frame and
transport copies still need their own review. SEC-15 remains partial.

The [SASL frame follow-up](../reports/audits/2026-10-19-security-sasl-frames.md)
removes independent reply authentication copies and makes parsing consume its
complete erasing input frame. Requests reject an excessive complete frame
before copying and retain their owner during encoding. Actual transport
accumulation and TLS receive copies still remain; SEC-15 stays partial and
finding counts are unchanged.

The [receive-owner follow-up](../reports/audits/2026-10-19-security-receive-owners.md)
then owns TLS/plain local chunks and Kafka frame assembly through routing and
parsing. One reservation replaces accumulator growth; partial failures retain
an erasing owner and fence re-entry until stream discard/reset. The shared S3
adapter keeps the new buffer type. S3 HTTP growth, library state, kernel backing
and forced termination remain open; SEC-15 stays partial.

The [S3 response-buffer follow-up](../reports/audits/2026-10-19-security-s3-response-buffers.md)
then removes header and chunk-decoder output growth. Decoding returns its
reserved erasing owner; framing scratch erases after consumption and on Drop.
Malformed input fences that parsing episode. Request formatting/metadata copies,
library state, physical memory and forced termination remain separate; SEC-15
remains partial and overall finding counts are unchanged.

The [S3 request-owner follow-up](../reports/audits/2026-10-19-security-s3-request-owners.md)
owns signature/authorization text and the serialized request head, reserves
before writing and removes aggregate signature Debug formatting. The serializer
independently rejects invalid wire fields; existing profile validation already
rejects those inputs. Canonical signing/path/query/response-metadata copies,
library state and physical memory remain separate; SEC-15 stays partial with
unchanged overall counts.

The [canonical-signing follow-up](../reports/audits/2026-10-20-security-s3-canonical-owners.md)
then owns URI/query/header canonicalization copies and returned signing
metadata. Complete reservations precede writing; invalid header shapes reject
before normalization copies. Caller inputs/raw paths, response metadata,
library state and physical memory remain separate. SEC-15 remains partial;
overall counts are unchanged.

The [response-metadata follow-up](../reports/audits/2026-10-20-security-s3-metadata-owners.md)
then removes ETag/version/request-ID copies and consumes the complete response
input through rejection or borrowed metadata use. The accumulator transfers its
existing header owner. Original request/profile data, exported metadata, library
state and physical memory remain separate; SEC-15 stays partial with unchanged
finding counts.

The [request-key/raw-path follow-up](../reports/audits/2026-10-20-security-s3-path-owners.md)
then owns the decoded key and complete raw path before copying. Shared wire
validation and existing key/path rules are preserved, with no longer-lived IPC
mapping. Original caller/IPC inputs, profile data, exported metadata, library
state and physical memory remain separate; SEC-15 stays partial with unchanged
counts.

The [profile-owner follow-up](../reports/audits/2026-10-20-security-s3-profile-owners.md) keeps the service-local S3 profile in one
immutable erasing allocation and lends its fields through typed views. Its HTTP
authority output is also reserved and owned before copying. The original launch
mapping scope, wire validation and permissions are preserved. Original launch
memory, host provisioning/logging copies, library state and physical memory
remain separate; SEC-15 and the overall finding counts stay unchanged.

The [profile-encoding follow-up](../reports/audits/2026-10-20-security-s3-profile-encoding.md) makes the S3 wire encoder return its
credential-bearing frame under erasing ownership. The kernel's existing caller
already wrapped the plain result and now uses the returned owner directly.
Aggregate wire-profile Debug is removed. Explicit copies, caller inputs,
provisioning/logging and launch-memory backing remain separate; SEC-15 stays
partial with unchanged finding counts.

The [Kafka profile-encoding follow-up](../reports/audits/2026-10-20-security-kafka-profile-encoding.md) makes the encoder return the
credential-bearing CHKAFP7 frame under erasing ownership and removes aggregate
Debug from Profile and Authentication. The kernel caller already wrapped its
former result and now uses the returned owner directly. Original caller inputs,
explicit copies, connector-local profile data, digest/scalar intermediates and
launch backing remain separate; SEC-15 and overall finding counts are unchanged.

The [SCRAM initial-owner follow-up](../reports/audits/2026-10-20-security-scram-initial-owners.md) removes the growable plain escaped
username and puts the initial phase's nonce copy under erasing ownership. Checked
transcript sizing precedes allocation; consuming phase transitions and wire/proof
bytes are preserved. Usernames/nonces remain wire metadata, and original inputs,
configured credentials, TLS state and physical memory retain separate policies.
SEC-15 and overall finding counts remain unchanged.

The [host key-file follow-up](../reports/audits/2026-10-20-security-host-key-encoding.md) removes a newline-triggered growth from
credential-bearing generated-key text. The writer already erased its final
buffer; it now receives a complete reserved erasing record. CLI tests preserve
file modes, exclusivity, rollback and output privacy. Retired storage, source
keys, OS/file buffers, host custody and other copies remain separate; SEC-15
and overall finding counts stay unchanged.

The [host profile-input follow-up](../reports/audits/2026-10-20-security-host-profile-input.md) owns plaintext before fallible reads,
using a fixed 64 KiB-plus-sentinel destination. Unix file admission rejects
symlinks and non-regular sources; existing empty/oversized rejection moves
ahead of reading. Partial-read controls and an actual CLI envelope roundtrip
pass. Host custody, OS/file copies and filesystem failure qualification remain
separate; SEC-15 stays partial and overall finding counts are unchanged.

The [private-key input review](../reports/audits/2026-10-21-security-private-key-input.md) verifies full-capacity
erasure after read errors and size rejection. It also makes normalization return
an erasing owner directly. The existing private-key caller already wrapped the
former plain result; no earlier input leak is claimed. Host files/OS copies,
library/compiler state and custody remain separate. SEC-15 stays partial and
finding counts are unchanged.

The [AES dependency follow-up](../reports/audits/2026-10-21-security-aes-erasure.md) enables the existing round-key
erasure feature used by TLS record ciphers and requires it through production
trait bounds. The baseline graph compiled that wiping out. Complete TLS
traffic-secret/HKDF state and GCM authentication-state erasure remain open;
this does not close SEC-15 or change finding counts.

The [GCM feature follow-up](../reports/audits/2026-10-22-security-gcm-erasure.md) enables authentication-key scratch
wiping and POLYVAL software-state erasure. AArch64's build guard rejects a
backend without a destructor, including the pinned optional PMULL path. The
x86 dispatch owner still lacks Drop delegation. The
[freestanding x86 follow-up](../reports/audits/2026-10-25-security-x86-gcm-owner.md)
now selects the existing erasing software backend through target Cargo flags
and extends the production destructor guard. Removing the flag or erasure
feature rejects the deployed build. The signed security probe executes the same
public GCM known-answer/tamper/decrypt control as the host test. PCLMUL
authentication acceleration is disabled; AES selection and the TLS cipher stay
unchanged. Hosted x86, complete TLS traffic-key/IV/HKDF erasure, forced termination
and physical-memory copies remain open. SEC-15 and finding counts are unchanged.

The [network HMAC follow-up](../reports/audits/2026-10-26-security-network-hmac-erasure.md)
replaces ephemeral-port selection's remaining separate HMAC/SHA-2 state with the
existing shared erasing helper and its result owner. Initialization owns entropy
scratch before filling, including partial/error paths. Independent Python vectors
and the signed probe preserve the existing port sequence, label, counter and
range. TCP/IP still requires fresh entropy before interface publication. SEC-15
remains partial; complete TLS state and compiler/physical copies remain separate.

The [dependency gate and ctutils update](../reports/audits/2026-10-26-security-dependency-gate.md)
add a pinned, fresh, unfiltered advisory CI check with no ignores and explicit
rejection of successful reports containing notices. A fresh scan found two
October 23 advisories missed by the older snapshot. Updating `ctutils` 0.4.2 to
0.4.4 removes RUSTSEC-2026-0385. The subsequent
[Embassy TLS migration](../reports/audits/2026-10-26-security-embassy-tls.md)
replaces the old curve family with crypto-bigint 0.7.10, preserves typed mTLS
and erasing cipher ownership, and installs domain-owned entropy. Its complete
root graph passes the fresh advisory gate without ignores. License/release
provenance, required-check deployment, constant-time/complete-state erasure
qualification and production trust remain separate. SEC-14/15 stay partial;
finding counts are unchanged.

| Order | Finding and acceptance criterion |
| --- | --- |
| 1 | **SEC-04:** Authenticate bootstrap policy and the executable boot chain from a protected platform root. Provision independent artifact, deployment and operational authorities and a distinct recipient key; exclude every development fixture. Keep the recipient private key behind a privileged custody boundary, with recovery/rotation rules and rollback-resistant policy state. Test substituted boot images/policies, role reuse, downgrade, enrollment and custody failure. Production must remain disabled until those tests pass. |
| 2 | **SEC-08/10:** Enroll node/operator identities under that root, authenticate and encrypt node/control/data and management channels, authorize reads and writes, and bind messages to cluster, role, session and freshness. Test peer substitution, replay, revocation and reconnect/restart. A signed write payload does not authenticate the transport or a reader. |
| 3 | **SEC-09:** Provide authenticated security-time provenance, bounded uncertainty/freshness and a fail-closed expiry policy after rollback, loss of synchronization and restart. Integrate it before claiming time-based credential/replay checks secure. SNTP and a nonzero timestamp do not satisfy this boundary. |
| 4 | **SEC-12 (corrected in the documented scope):** Reserved mutation policy, persisted ownership/private reads and operator scope grants are implemented ([reserved policy](../reports/audits/2026-10-10-security-object-store-reserved-namespace.md), [ownership](../reports/audits/2026-10-10-security-object-store-ownership.md), [scope policy](../reports/audits/2026-10-10-security-object-store-scope-policy.md)). The [follow-up](../reports/audits/2026-10-10-security-object-store-follow-up.md) rejects metadata-read failure before authorization/data access and preserves explicit IDs during bounded automatic creation, with targeted host/guest checks and policy rejection/revocation evidence. Limits: policy authority is the signed `clusterctl` identity (operator enrollment is SEC-08/10), 64-entry bound, pre-ownership/duplicate-ID images must be recreated, no listing opcode exists, and failed physical I/O/power-loss atomicity is not qualified. |
| 5 | **SEC-06/14/15:** Finish management flood resistance/fairness, advisory/license and release-provenance enforcement, and the remaining secret-derived state/copy zeroization inventory. Retain the existing bounded-peer, pinned-build and temporary-buffer corrections. |

The first SEC-04 prerequisite is now a
[public trust-candidate preflight](../guides/signing-and-trust.md#public-trust-policy-preflight).
It checks keys and policy context and produces reviewable bytes. It supplies
neither authenticated bootstrap nor recipient custody and does not change any
finding's closure state.

[Signed bootstrap policy tooling](bootstrap-trust-policy.md) now authenticates
those public bytes under a separate caller-supplied bootstrap key and checks
explicit revision lineage. Protected key/state provisioning, atomic installation,
executable boot authentication and recipient custody remain required. Host
verification is not a replacement for those gates; finding states are unchanged.

The kernel now has a consuming, one-shot boot-policy handoff. DNS/deployment
manifests and the final kernel gate share its immutable public policy; recipient
key mismatch and later policy replacement reject. Development selection remains
explicit. The protected installer/production boot selector and recipient
custody are still missing; **SEC-04 remains partial**.

An [x86 QEMU Secure Boot matrix](../guides/qemu-secure-boot.md) now exercises
firmware-to-Limine/config/kernel/policy integrity and signed-module consumption
under disposable test authority. Boot requires the exact installed test state,
rejecting stale/conflicting/uninstalled policies before publication. This is
execution evidence for that test chain; production enrollment, persistent
anti-rollback state, production platform-service roots and custody remain open.
The [early artifact-authority follow-up](../reports/audits/2026-10-19-security-platform-artifact-authority.md)
now selects one policy before platform loading. The fixture uses its selected
test key for the whole bundle and rejects development-signed NS/NVMe
substitutions. This corrects key routing without supplying production enrollment.

The [DMA-map failure ownership follow-up](../reports/audits/2026-10-12-dma-map-failure-ownership.md)
continues C13/C18 and G2/G3/G7: per-call diagnostics retain map rejection rather
than collapsing it to zero, and failed shared-DMA setup retains its existing CPU
mapping without safe Rust access. Diagnostics and CPU unmap do not prove DMA
rollback or authorize retry. Public-operation readiness/IRQ qualification,
historical AMD causality, the retained Intel scheduled-close/ingress failure and
broader cleanup acceptance remain open. SEC-18 remains partial; finding counts
are unchanged.

The [Raft snapshot continuation correction](../reports/audits/2026-10-19-raft-snapshot-streams.md)
reproduces mixed-prefix/suffix installation across leader/election episodes.
The private buffer now binds its leader and RPC term and is discarded on local
election or higher-term observation. Host regressions and a small model retain
owner/offset failures. Authentication, same-episode byte-image stability and
general allocation/persistence failures remain separate; finding counts are
unchanged.

The [Raft per-read fence correction](../reports/audits/2026-10-19-raft-read-fences.md)
replaces the catalog query lease with a fresh no-op commit for each admitted
read. Exact node/domain, term, membership and application checks consume one
query owner; DNS retains original deadlines and owned reply/lookup phases while
servicing Raft traffic. The former stale-read host schedule now rejects, and
four model controls retain missing-fence/binding/prerequisite failures. Bounded
status/ingress freshness, raw transport readiness and broader admission remain
separate; numbered finding dispositions are unchanged.

The [Raft clock/lease-gap review](../reports/audits/2026-10-18-raft-clock-lease-gap.md)
replaces wake-count time with checked kernel-monotonic elapsed time in both
reactors. It also reproduces a stale application read after a delayed same-term
reply refreshes an obsolete leader. That historical gap is retained as host/model
evidence and subsequently corrected for catalog queries by the per-read fence
batch above. No read-safety or numbered finding closure is claimed by the clock
correction.

The [Raft read-prerequisite review](../reports/audits/2026-10-18-raft-read-prerequisites.md)
reproduces query admission before a newly elected leader commits in its term.
The gate now requires a current-term commit and application through that commit,
with a finite read model and two negative controls. This does not resolve
receive-time lease timing, delayed same-term responses or request correlation;
numbered finding dispositions remain unchanged.

The [Graft membership/parity follow-up](../reports/audits/2026-10-18-raft-membership-parity.md)
compares all three upstream implementations and the Jepsen scenario definitions.
It restores the automatic proposed-member catch-up gate and corrects retained
learner leadership without erasing the current-term vote. The model now includes
next-only voter elections and three required counterexamples. This does not
qualify mixed Charlotte/Graft clusters, authenticated networking or read leases;
numbered finding dispositions remain unchanged.

The [Raft response-ordering review](../reports/audits/2026-10-18-raft-response-fencing.md)
reproduces and corrects old-term acknowledgements committing a replaced prefix.
Append/snapshot progress now rejects mismatched or unadmitted sources and
out-of-tail success, retaining confirmed progress under reordering/backoff.
It also corrects the snapshot model's missing join-clear branches with explicit
coverage. Numbered finding dispositions remain unchanged; transport identity,
read-lease timing, membership drift and physical cleanup remain separate work.

The [snapshot acceptance model review](../reports/audits/2026-10-18-snapshot-restore-model.md)
checks application rejection before durable/progress/membership/join publication
and fallible restart exposure. Existing host tests pass; no runtime fix or finding
closure is claimed. Allocation, concurrent reads and application/disk atomicity
remain separate boundaries.

The [boot-trust model review](../reports/audits/2026-10-18-boot-trust-model.md)
checks existing fixture/one-shot publication separately from a conditional
protected-installation contract. Stale expectations, failed/interrupted commit
and reboot are explored only under explicit atomic-state/platform assumptions.
No protected installer, custody provider or production selector is added;
SEC-04 and finding counts remain unchanged.

The [NS publication model and generation correction](../reports/audits/2026-10-18-name-publication-model.md)
checks public/policy issuance, admitted watch ownership, exact death retirement
and fail-closed owner retention. It also reproduces and corrects publication
beyond the positive signed NS generation range before policy mutation.
The C18 model assumes kernel producer/identity authenticity and trusted launch
budget preparation; actual allocation/destructor contexts and physical cleanup
remain open. Finding counts and SEC-07/18 status do not change.

The [coherent-DMA remap correction](../reports/audits/2026-10-18-coherent-dma-remap-fence.md)
closes a remaining path from consuming failed shared-DMA CPU teardown to a
fresh safe CPU mapping. Every retained DMA pin now fences fresh mapping,
including read-only access. A controlled ARM regression failed before the
change; ARM/SMMUv3 and Intel/VT-d runs passed afterwards. Existing coherent
mappings retain their volatile protocol. This narrows C18's restriction boundary;
SEC-18 and wider cleanup/context qualification remain partial.

The [timed completion outcome correction](../reports/audits/2026-10-12-completion-wait-outcomes.md)
continues C16/C18 and G2/G7: kernel callers distinguish admission rejection from
deadline expiry, and the syscall reparks on unrelated notifications through the
shared deadline loop. Pending completion/buffer ownership remains intact. New
exact-generation fixture diagnostics do not establish historical Intel causality
or physical quiescence; SEC-18 remains partial and finding counts are unchanged.

The [discovery simplification validation](../reports/audits/2026-10-13-protocol-simplification-discovery.md)
again observed the completion-wait timer-sponsor equality failure: ARM reported
4 versus baseline 3 at `self_test/cq_wait.rs:268:9`, followed by incomplete
paired DNS progress. A fresh paired repeat with identical kernels passed
20/0/0 on ARM and 18/0/0 on x86. The aggregate mismatch identifies no specific
retained event and does not prove a production leak or its cause; the fixture's
private-account isolation premise is corrected by the Raft batch below;
individual timer identities and wider contexts remain unresolved. The failed sample is
retained separately, and no C16/C18 acceptance gate or finding count changes.

The [Raft persistence batch](../reports/audits/2026-10-13-protocol-simplification-raft.md)
records another x86 equality failure (5 versus 2) and the paired ARM discovery
deadline failure. Source review identifies the fixture's false isolation premise:
kernel threads share their address-space generation's timer sponsor, so local
IRQ masking does not exclude another LP's reservations or reclamation. The
still-unpublished fixture driver now receives a private trusted sponsor before
event capture; production root sponsorship and wait semantics remain unchanged.
This corrects the pressure fixture without identifying individual historical
timer deltas or closing C16/C18, broader context qualification or finding counts.
Corrected paired fresh-storage and disk-reuse boots passed 20/0/0 on ARM and
18/0/0 on x86; both single-voter storage fixtures advanced 1→2 on the first
boot and 3→4 after disk reuse. Failed samples remain separately recorded.


The [catalog snapshot/query batch](../reports/audits/2026-10-13-protocol-simplification-catalog.md)
removes snapshot versions 1–14 and missing query-section defaults. Explicit
application restore errors now stop Raft startup or reject network installation
before log, applied-state, membership or join-completion publication. This
closes the silent-rejection behavior at that boundary; descriptor-less launch
callers and packed deployment APIs were removed later in S04b2a below. It
adds no cleanup custody or physical completion proof and changes no numbered
finding status or count. Infallible snapshot allocations, concurrent reader
isolation and power-loss atomicity remain outside this batch. All 410 host tests and four matching-architecture deployment guests passed (22/0/0 twice on ARM, 20/0/0 twice on x86). Initial status-reader failures and the invalid mixed-architecture deployment pair remain separately recorded.


The [scoped launch API batch](../reports/audits/2026-10-13-protocol-simplification-launch.md)
removes packed scoped names and duplicate runtime wrappers. The kernel returns
the verified descriptor principal directly into the retirement owner; callers
cannot supply a different retirement name or select a singleton with zero.
This changes neither numbered finding counts nor physical cleanup qualification.
The later S04b2a administration batch removes descriptor-less launch and packed catalog operations.
All 411 host tests and final ARM 20/0/0 / x86 18/0/0 signed-ingress runs pass.
The initial x86 timer-event probe received a legitimate pre-admission timeout
under its 1 ms budget; its failed run is retained separately. The corrected
probe uses an isolated, pre-publication sponsor and observes its worker's exact
exit generation, without replacing live sponsorship or changing production
wait semantics. C16/C18 and broader cleanup qualifications remain open.


The [administration API removal](../reports/audits/2026-10-13-protocol-simplification-administration.md)
removes the unused descriptor-less launch syscall and its SDK/runtime wrappers,
controller unsigned deploy and DNS packed deploy/query adapters. Full-name
status queries now retain request/reply memory through the owning IPC APIs.
This removes the development-key deployment bypass identified historically
under SEC-11; the separate live-upgrade trust boundary remains unresolved, so
this batch changes no numbered finding status/count. Necessary capability
escrow, cleanup receipts, uncertain backing retention and rollback owners
remain unchanged. Redundant diagnostic deployment record fields are removed by S04b2b below.


The [lean deployment record batch](../reports/audits/2026-10-13-protocol-simplification-records.md)
makes signed descriptors the sole source of requested names, placement, keys
and digests. Catalog state retains generation and exact node/variant pins.
Current-only tags reject older query/snapshot/relay forms; Raft pair-v2 also
rejects log-only old stores before startup, without overwriting them. This
removes compatibility state and adds no cleanup custody, physical retry or
production trust qualification; numbered audit finding counts remain unchanged.


The [DNS registration/retraction batch](../reports/audits/2026-10-14-protocol-simplification-dns-registration-relay.md)
removes lossy identities, name-only reply matching and the missing-publication
leader fallback. Current relays retain captured peer/session/request correlation,
prepared reply bytes and owned registration connections/reply authority; explicit
catalog-only callers use the event operation. Final host, linked ARM/x86 and
HTTP checks pass. A separate mixed-run scheduled IPC endpoint-close exit-watch
timeout remains unresolved, with later Raft persistence errors retained and
exact-generation diagnostics added without relaxing completion requirements.
This changes no numbered finding status/count and grants no cleanup custody,
physical completion or production trust qualification.


The [IPC close/exit investigation](../reports/audits/2026-10-14-ipc-close-exit-investigation.md)
extends the ordinary scheduled CQ regression to bounded repeated same/cross-LP
real closes and exact-generation exit watches. Its initial two mixed pairs
passed 192 episodes. The subsequent
[NS generation batch](../reports/audits/2026-10-14-protocol-simplification-name-generations.md)
reproduced the timeout on x86 and ARM, with returned-worker, exact-source and
live/staged-generation diagnostics. Historical causality and the later Raft
persistence failures remain open; the initial passing episodes do not close them.

The [park-publication correction](../reports/audits/2026-10-14-park-deferred-delivery.md)
and [first-dispatch correction](../reports/audits/2026-10-15-first-dispatch-delivery.md)
then fix two distinct producer-LP delivery boundaries. Both have failing ARM
negative controls and passing corrected native execution. The latest batch
passes ARM/x86 security and linked deployment suites, including 256 exact IPC
close/exit episodes and 16 first-entry controls. These establish scoped ordering
corrections, not the historical interleaving or universal reaper progress.
Timeouts, physical-release proofs, custody and finding counts remain unchanged;
C16/C18 and G1/G7 remain open.

The [CI runtime/diagnostic correction](../reports/audits/2026-10-15-ci-sitas-runtime-pin.md)
pins all six Sitas checkouts to the checked-wait backend `d27f7e7b…` and preserves
target frame pointers under CI's warning policy. Old-source execution reproduces
the SITAS deadline failure and corrected local equivalent-profile execution
passes. This is separate from IPC/persistence attribution; a later GitHub job
is not verified here. Remaining failure families and next diagnostic work are
listed in the [process review](../reports/audits/2026-10-15-audit-process-review.md).

The [Raft persistence diagnostic follow-up](../reports/audits/2026-10-15-raft-persistence-diagnostics.md)
addresses that review's lost-error gap. Open failures retain per-call operation,
object, stage and typed cause; mutation fail-stop distinguishes encoding, write
and flush while retaining the enclosing record identity. It changes no stored
format, deadline, replay/cleanup policy or numbered finding disposition, and
does not establish the historical persistence or IPC failure cause.

The [automatic failure capture follow-up](../reports/audits/2026-10-15-automatic-failure-capture.md)
adds an independent timed-run watcher and private all-LP QMP/LLDB snapshots on
both architectures, retaining the actual boot-image kernel and per-run input
identities. Native positive and short-deadline controls validate capture;
historical IPC/persistence attribution and C16/C18/G1/G7 remain open. Snapshot
state is diagnostic, not cleanup completion or permission to replay.

The [captured park-fixture investigation](../reports/audits/2026-10-15-captured-park-fixture-panic.md)
uses that watcher to preserve a spontaneous ARM panic and a forced-switch
negative control. The fixture incorrectly required its publisher to remain
Running while an incoming context delivered its callback; Ready is also valid
before wait registration. Exact generation, IRQ policy and admission assertions
remain. This corrects a fixture premise, with no scheduler/cleanup policy or
numbered finding disposition change; historical IPC/stack/persistence causes
and C16/C18/G1/G7 remain open.

The [thread-exit watch preparation follow-up](../reports/audits/2026-10-15-thread-exit-watch-preparation.md)
continues R7-1/2 and C16/G1/G4. It removes fallible source/entry allocation from
thread-table/source serialization, retains original admission through post-guard
losing-preparation disposal and revalidates the captured generation/authority
before allocation-free publication. Scoped inventory and race/heap-held probes
do not complete the wider allocation/destructor audit, aggregate heap/principal
budgets or integrated pressure/physical qualification. SEC-07/18 remain partial;
historical exit-watch/stack/persistence causation and finding counts are unchanged.

The [stack-growth execution-owner follow-up](../reports/audits/2026-10-15-stack-growth-execution-owner.md)
continues C16/C17 and G1/G2 with the same exact-thread execution fence used by
abort sweeps. Original root/slot/maximum charge stays in its containing thread;
provisional backing and prefix progress run outside the master thread table.
Exact completed handback precedes fence release. Actual EL0 growth, deferred
abort and map-rejection rollback qualify that context while inner walker,
heap/image/constructor, aggregate-budget and integrated physical/pressure work
remain open. No historical timeout attribution, new recovery authority or
numbered finding disposition follows.

The [scheduler selection follow-up](../reports/audits/2026-10-15-scheduler-selection-storage.md)
continues R7-1/2 and C16/G1/G4. It removes temporary load/candidate allocation
from rebalance scans without changing deterministic ties or exact-generation
policy checks. The caller inventory also finds that the previously documented
x86 migration restriction was not enforced: its CPU-ownership predicate is a
stub while both yield tails invoke the sampler. x86 now rejects every already
dispatched context; Arm retains its assembly ownership qualification. Native
fixtures keep selection under the heap, both fixture LP locks and the master
table, and distinguish the architecture gate. Destination run-queue growth,
thread-table metadata, general heap/principal admission, full outer-context
qualification and historical failure attribution remain open. No new owner
family, recovery authority or numbered-finding disposition is added.

The [inline scheduler queue follow-up](../reports/audits/2026-10-15-inline-scheduler-run-queues.md)
continues R7-1/2 and C16/G1/G4 with queue links in the original complete thread
payload, removing growable ring allocation from admission, wake, requeue and
migration. Exact source/destination validation precedes an allocation-free
queue/state/affinity splice. Requested owner-LP abort unlinks before table
extraction; remote notification revalidates placement under the original
generation guard. Linked physical/unstarted release rejects with the complete
owner retained. Queue abandonment grants no force-clear, cleanup or replay.
Broader table/waiter/timer/heap storage, aggregate budgets, essential progress
and integrated physical qualification remain open. SEC-07/18 stay partial;
family/finding counts and historical failure attribution are unchanged.

The [scheduler wait-publication follow-up](../reports/audits/2026-10-15-scheduler-wait-publication.md)
continues R7-1/2 and C16/G1/G4. Captured generation/state/sponsor precede
fallible waker and source registration outside local LP/table guards. Early
notifications remain recorded through publication; source callbacks require the
currently installed waker as well as the thread generation. Ready admission
carries the old subscription past both guards before cancellation/destruction;
unused or rejected preparation is disposed there too. Original deadlines,
caller IRQ policy, accepted operation owners and the abort/execution fences
remain. Source registry/initialization guards, outer system/park masks, broader
metadata/principal admission and essential-progress qualification remain open.
No numbered finding changes disposition, and no historical timeout cause,
physical completion or recovery/replay authority is inferred.

The [waiter source-preparation follow-up](../reports/audits/2026-10-15-waiter-source-preparation.md)
continues R7-1/2 and C16/G1/G4. Lazy list admission/allocation leaves the source
slot mutex, with first-winner publication and post-guard losing-candidate
release. IPC/CQ entries own the captured list and original sponsor charge;
registry publication checks exact identity before readiness and links without
allocation. Ready/full/stale owners dispose after guard exit. CQ fixtures now
use the existing captured interest rather than a separate numeric-only
observable. Source teardown, other observer/record allocation, caller masks,
metadata/principal budgeting and pressure progress remain open. No finding
changes disposition, historical failure attribution or physical/replay
authority follows.

The [completion callback/source follow-up](../reports/audits/2026-10-15-completion-callback-preparation.md)
continues R7-1/2 and C16/G1/G4. Callback preparation retains the exact completion,
namespace/root, watch account and existing source outside registry/inner guards;
publication revalidates identity, retirement and terminal state before linking.
Losing/ready/rejected storage disposes after unlock and before immediate
notification. Successful namespace close/replacement and CQ replacement carry
the entire old payload beyond the registry before source/field destruction,
retaining original backing and charges. Authority/map metadata, staging/partial
errors, timer-specific producer disposal and broader caller/IRQ contexts remain open.
No numbered finding changes disposition, and no physical/quiescence proof,
retry authority or historical-failure attribution follows.

The [terminal observer-disposal follow-up](../reports/audits/2026-10-16-completion-terminal-disposal.md)
continues R7-1/2 and C16/G1/G4. One terminal transition retains event/worker
subscriptions through effective-result/CQ publication and registry unlock;
installation returns replaced/rejected owners for post-guard disposal. Captured
CQ source identity prevents destructor reentry from waking a replacement queue.
Cancellation parity, worker-exit ownership and accepted buffers are unchanged.
Timer installation/cancellation, staging/partial errors, record/authority/map
metadata and outer producer/IRQ contexts remain open. No finding changes
disposition; historical failure attribution and physical/replay authority do not
follow from these bounded controls.

The [CQ payload-preparation follow-up](../reports/audits/2026-10-16-cq-payload-preparation.md)
continues R7-1/2 and C16/G1/G4. CQ admission and list/backlog/heap-ring
preparation leave the completion registry, including partial-error disposal.
Borrowed publication rechecks exact namespace root/account/capacity and
retirement before physical alias checks and any frame write. Whole-namespace
preparation retains its private map node and complete payload through rejection;
old payload destruction remains post-guard. Fresh registry-map nodes, unified
authority, record construction, timer cleanup and outer producer/IRQ contexts
remain open. No finding changes disposition, no physical pin or retry owner is
added, and passing fixtures do not attribute historical intermittent failures.

The [detached timer-disposal follow-up](../reports/audits/2026-10-16-detached-timer-disposal.md)
continues R7-1/2 and C16/G1/G4. Detached terminal publication now retains the
complete timer-bearing record past registry unlock. Direct-result charges
release with that owner, while backlog entries retain their original charge
and slot; exact-source notification follows disposal. A firing event keeps its
separate queue/node charge through the active callback. Map removal, existing
backlog draining, submission construction, capability-backed timer installation
and outer producer/queue/IRQ contexts remain open. No finding changes
disposition, physical proof or replay authority, and historical intermittent
failure attribution remains open.

The [detached preparation follow-up](../reports/audits/2026-10-16-detached-submission-preparation.md)
continues R7-1/2 and C16/G1/G4. Detached plain/timer preparation retains original
record/event charges and complete node/observer state outside the completion
registry. Borrowed publication rechecks exact root/accounts/capacity, retirement
and CQ source; rejected preparations remain owned until post-guard unstarted
rollback. Accepted nodes enqueue once, preserving teardown-before-enqueue
cancellation and event backing admission. Map/authority metadata, backlog
reconciliation, capability-backed preparation and outer caller/queue/IRQ
contexts remain open. No finding changes disposition, physical proof or replay
authority, and historical intermittent failures remain unattributed.

That source review identified wrapping completion operation IDs as an
additional R7/C16 boundary. The [checked-exhaustion follow-up](../reports/audits/2026-10-16-operation-id-exhaustion.md)
now rejects permanently before new record/watch/timer or capability reservation,
excluding both zero and the syscall failure sentinel. Failed preparation never
refunds an identity; namespace replacement never resets the boot-lifetime
counter. Isolated boundary controls preserve existing operations and original
charges without changing the live counter. This closes that specific source gap,
not SEC-07/18, wider metadata preparation or historical failure attribution.

The [capability-backed preparation follow-up](../reports/audits/2026-10-16-capability-completion-preparation.md)
continues R7-1/2 and C16/G1/G4. Charged records, waiter lists, watch admission,
shared authority storage and timer cancellation/node/producer construction now
leave the completion registry. Hidden authority admission and final borrowed
batch publication revalidate exact root, original accounts/capacity and
retirement. One complete owner explicitly disposes ordinary unused resources
outside local guards; abandonment retains all fields. Timers carry their
original absolute deadline and preserve teardown-before-enqueue cancellation.
Registry/backlog metadata, close and terminal timer cleanup, aggregate admission
and outer contexts remain open. That checkpoint recorded a separate anonymous
timer cancellation-ID wrap gap, corrected by the exact-object cancellation
follow-up below. No finding changes disposition, physical/replay authority or
historical intermittent failure attribution.

The [exact timer-cancellation follow-up](../reports/audits/2026-10-16-timer-cancellation-identity.md)
removes the private numeric timer selector and its wrapping counter entirely.
Eager queue removal borrows the consuming handle's existing charged state Arc
and matches that exact retained allocation. A stale predecessor cannot select
a later event; no new exhaustion admission result is introduced. Busy/remote
flagging, cancellation-before-enqueue, IRQ preservation and original strong/weak
backing charges retain their existing policies. This closes the recorded timer
identity gap within C16; queue/outer-guard disposal, broader metadata admission,
physical proof, progress and historical failure attribution remain separate.
SEC-07/18 remain partial and the numbered dispositions are unchanged.

The [backlog refund follow-up](../reports/audits/2026-10-16-completion-backlog-refunds.md)
continues R7-1/2 and C16/G1/G4/G7. Delivered detached charges leave the completion
registry in at most two inline original-account/class owners. Admission and CQ
observation refund after local unlock; waits refund before parking and producers
before notification. Full FIFO flushing and slot updates remain serialized.
The initial Intel run captured a separate IPC fixture panic: registration was
mistaken for Blocked publication. The fixture now observes both phases for the
exact generation under its original five-second deadline, retaining mandatory
exit watches; a deterministic Ready control verifies the pre-publication window.
A later Intel deadline captured a separate heap/registry cycle: the heap-held
CQ waiter fixture acquired the completion registry after concurrent writers
started, while a detached-map insertion held the registry and needed the heap.
That fixture now runs in the serialized pre-scheduler phase, retaining its
heap-held controls. Both failures are preserved separately; the panic lacked
the exact sampled worker state. Historical worker-exit timeout causes remain
open. The captured production map allocation remains an explicit next boundary.
Broader map,
backlog/close/queue metadata, aggregate admission and outer contexts still need
qualification. No SEC disposition, physical proof, readiness/replay policy or
production IPC/scheduler behavior changes.


The [detached metadata follow-up](../reports/audits/2026-10-16-detached-completion-metadata.md)
continues R7-1/2 and C16/G1/G4/G7. Detached records prepare admitted map nodes
outside COMPLETIONS and relink only after exact root/account/capacity/CQ
validation. Terminal delivery retains the same node with its original charge
in a direct receipt or backlog. Flush batches free nodes after local unlock
before checked original-class refunds. Whole namespace/preparation/terminal
fallback retains every field; ordinary rejection and namespace close/replacement
explicitly finish after local guards. The preceding captured detached-map
allocation is removed without restoring concurrent heap-held probes. Other
capability/CQ/global maps, authority revocation, aggregate admission and wider
contexts remain open. No SEC disposition, physical proof, custody, readiness or
replay policy changes, and historical intermittent causes remain unattributed.


The [capability completion metadata follow-up](../reports/audits/2026-10-16-capability-completion-metadata.md)
continues R7-1/2 and C16/G1/G4/G7. Payload map storage prepares before hidden
authority reservation; final publication relinks after existing exact validation
and authority publication. Retirement keeps payload and detached authority in
one admitted node, then frees both outside local completion guards while the
charged completion remains live. Shared authority refund follows actual node
deallocation. Common close/abort/event rollback removes the exact pending CQ
row before returning its slot, correcting terminal-abort backlog accumulation.
Complete fallback retains fields/accounts without cleanup or retry custody.
Both per-operation maps now use admitted nodes; CQ/global maps, other consumers'
outer guards and wider timer/observer contexts remain open. No SEC disposition,
physical proof, readiness/replay or historical failure attribution changes.

The [CQ metadata follow-up](../reports/audits/2026-10-16-cq-metadata.md)
continues R7-1/2 and C16/G1/G4/G7. Fallible queue-node preparation joins the
complete existing payload owner outside COMPLETIONS. Fresh publication and
replacement only relink admitted nodes; the retired owner carries the complete
source/ring/backlog and original charge through post-guard node deallocation,
then payload destruction. Whole namespace disposal explicitly drains queues;
guarded fallback retains every field. Native controls cover all 32 fresh nodes,
node rejection, full admission, physical publication and original-account reuse.
The global namespace map, aggregate metadata/principal bounds and wider
timer/observer/IRQ/caller contexts remain open. No finding disposition,
physical proof, recovery custody, readiness/replay or historical attribution
changes.

The [global completion namespace metadata follow-up](../reports/audits/2026-10-16-completion-namespace-metadata.md)
continues R7-1/2 and C16/G1/G4/G7. A complete preparation fallibly owns the
global admitted node and four original accounts outside COMPLETIONS, alongside
the private payload and any default CQ. Publication relinks after validation;
there is no later admission after physical-ring initialization. Close/replacement
detaches the global node and complete namespace, then explicitly frees metadata
before payload/source/account disposal after local unlock. Fallback retains all
fields. Controls cover five allocation-stage rejections, 64 heap-held fresh
namespaces, reentrant disposal, complete timer/backlog/authority replacement and
real root-generation reuse. Aggregate namespace/account admission, wider
timer/observer/IRQ/caller qualification and historical attribution remain open.
No finding disposition, physical proof, readiness/replay or custody change.

The [timer queue-disposal follow-up](../reports/audits/2026-10-16-timer-queue-disposal.md)
continues R7-1/2 and C16/G1/G4/G7. Filtering, exact cancellation, purging,
expiry and rejected enqueue return complete original nodes and inline keyed
payloads without freeing them under the queue borrow. All production adapters
leave that borrow before explicit iterative disposal; original event/node and
cancellation backing admission follows actual release. Receipt/queue fallback
retains all remaining fields. Controls cover 64 heap-held exact removals,
purge/rejection, anonymous/keyed expiry, original weak-only charges and guarded
abandonment. Callback/waiter notification remains under the queue; outer
scheduler/producer guards, IRQ execution and unpublished preparation are still
open. No finding disposition, physical proof, recovery custody, readiness/replay
or historical attribution changes.

The [timer callback-delivery follow-up](../reports/audits/2026-10-16-timer-callback-delivery.md)
continues R7-1/2 and C16/G1/G4/G7. One cutoff captures the already-due prefix;
one complete non-Send/non-Sync owner retains selected and purged original
payloads through queue unlock. Callback/waiter delivery, temporary observer
destruction and disposal then run on the captured LP with IRQs masked. The
existing queue's inline claim fences nested drains; reentrant enqueue waits for
the next pass, selected cancellation rechecks shared state, and explicit
handback reconciles the current comparator. Fallback retains fields and claim.
Controls cover 64 ordered selections, cancellation, nested drains, quantum ties,
final observer destruction and private guarded retention. IRQ execution, outer
guards, unpublished preparation, final handles and aggregate pressure remain
open. No finding disposition, physical proof, replay/custody or historical
attribution changes.


The [unpublished timer ownership follow-up](../reports/audits/2026-10-16-timer-unpublished-ownership.md)
continues R7-1/2 and C16/G1/G4/G7. Completed private nodes retain every field and
original admission on abandonment. Enqueue or explicit unused disposal consumes
that owner once; scheduler early returns and both completion submission finish
paths dispose unused nodes after local guards. Sleep prepares before its local
publication mask while preserving outer masks and the atomic Blocked/enqueue
transition. Raw events/partial construction, final cancellation handles, waiter
preparation and outer IRQ/guard contexts remain open. No finding disposition,
physical proof, replay/custody or historical attribution changes.

The first unpublished-node ARM/Intel samples detected an introduced timed-wait
error-arm ordering bug: a match-scrutinee scheduler read guard survived local
mask release and unused disposal. Binding registration before matching releases
that guard first. Both panic samples and the correction are preserved in the
linked batch report; this does not attribute earlier historical incidents.


The [timer cancellation handback follow-up](../reports/audits/2026-10-16-timer-cancellation-handback.md)
continues R7-1/2 and C16/G1/G4/G7. The exact cancellation handle and any detached
node/source remain in one complete result until explicit finish after local and
enclosing queue/registry guards. Bare handles use explicit reference-only
release; their fallback retains without flagging. Handback fallback retains all
fields and original charges. Stale busy-queue final-reference and 64 heap-held
removal controls distinguish handback from actual release. Raw Arc/Weak and
partial construction, completion containing destructors, outer IRQ/guard and
aggregate pressure remain open. No finding disposition, physical proof,
replay/custody or historical attribution changes.


The [timer node-construction follow-up](../reports/audits/2026-10-16-timer-node-construction.md)
continues R7-1/2 and C16/G1/G4/G7. Partial node preparation retains the already
charged event, source/callback and outside admission before allocating empty
storage. The allocator owns no payload; rejection cancels explicitly after its
return, and successful storage initializes once into the existing private node
owner. Abandonment retains either phase without cleanup. Raw event/admission
construction, raw references, waiter/containing destructor and outer IRQ/guard
contexts remain separate. No finding disposition, physical proof, retry/custody
or historical attribution changes.


The [event/shared-holder construction follow-up](../reports/audits/2026-10-16-timer-event-construction.md)
continues R7-1/2 and C16/C18/G1/G4/G7. Event preparation retains original source
and admission through shared holder and cancellation-state allocation. Holder
factories own no charge; owning rejection cancels after allocator return while
parent-borrowed rejection retains its exact reservation for payload-first
cleanup. Inert phase fallback retains all fields without notification/refund.
Completion records, timers and observer lists share the same primitive and
unchanged lifetime accounting. Completed raw events, resource-bearing Arc
payloads, containing destructors and wider waiter/IRQ/guard contexts remain
open. No finding disposition, physical proof, retry/custody or historical
attribution changes.


The [shared Arc payload-construction batch](../reports/audits/2026-10-16-charged-arc-payload-construction.md)
continues C16/C18/G1/G4/G7 and R7-1/2. The common helper retains the complete
value and original allocator across empty storage allocation; rejection disposes
explicitly after callback return, initialization consumes the same storage once,
and phase abandonment retains every field without cleanup. Records, lists and
cancellation state share this path. Outer factories, published reference and
containing destructors, waiter/IRQ contexts and aggregate pressure remain
separate. Finding dispositions and historical failure attribution are unchanged.


The [completion record-construction batch](../reports/audits/2026-10-16-completion-record-construction.md)
continues C16/C18/G1/G4/G7 and R7-1/2. Original buffer, operation and admission
now share one owner before holder/list allocation. Rejection disposes payload
before refund; abandonment retains all phase fields, and successful transfer
preserves the same buffer/identity through shared Arc construction. Outer
submission factories, published/containing destructors, authority rollback and
wider IRQ/guard qualification remain separate. Finding dispositions and
historical failure attribution are unchanged.


The [initial completion admission batch](../reports/audits/2026-10-16-completion-initial-admission.md)
continues C16/C18/G1/G4/G7 and R7-1/2. The containing submission owner now
captures its original buffer before identity/record/watch admission, retains
early record charges on rejection and lends itself to record factories. Explicit
pre-record rollback destroys buffer before refund; all three early phase
fallbacks retain complete fields and original accounts. Target capture, timer/
observer factories, published/containing destruction and wider IRQ/guard
contexts remain separate. Finding dispositions and historical failure
attribution are unchanged.


The [shared observer-entry construction batch](../reports/audits/2026-10-16-observer-entry-construction.md)
continues C16/C18/G1/G4/G7 and R7-1/2. Original weak observer, waiter/watch
charge and empty backing stay with one inert preparation during allocation.
Rejection cancels after allocator return; initialization consumes private storage
once, and partial abandonment retains every field. Completed preparation/token/
source destruction, callback-containing owners and wider contexts remain
separate. Finding dispositions and historical failure attribution are unchanged.

The [completion callback-owner batch](../reports/audits/2026-10-16-completion-callback-ownership.md)
continues C16/C18/G1/G4/G7 and R7-1/2. Exact target, original weak callback
and partial watch/list/entry preparation now share one inert owner before
admission. Entry factories borrow it; ordinary outcomes finish explicitly after
local guards and before ready notification. Four phase fallbacks retain the
complete original buffered record and charges across namespace replacement.
Published observation/token/source disposal, other caller/IRQ contexts and
aggregate admission remain separate. Finding dispositions and historical
failure attribution are unchanged.

The [thread-exit watch owner batch](../reports/audits/2026-10-16-thread-exit-watch-ownership.md)
continues C16/C18/G1/G4/G7 and R7-1/2. Captured candidate source, original
weak observer and admitted watch charge now precede list/entry preparation in
one inert owner. The scheduler preparation hook borrows it; ordinary outcomes
finish explicitly after local guards, candidate source last. Four phase
fallbacks retain every field without cleanup. Initial capture/admission,
generic completed preparation, published token/source destruction, aggregate
admission and wider contexts remain separate. Finding dispositions and
historical failure attribution are unchanged.

The [waiter preparation owner batch](../reports/audits/2026-10-16-waiter-preparation-ownership.md)
continues C16/C18/G1/G4/G7 and R7-1/2. Exact source, sponsor and weak
observer now precede admission in one inert owner. Entry factories borrow it;
ordinary preparation/publication outcomes explicitly finish after local guards,
source before sponsor and after transient entry/admission disposal. Three phase
fallbacks retain complete fields through source closure and sponsor retirement.
Initial source acquisition, generic completed entry, published token/source/batch
destruction, aggregate admission and wider contexts remain separate. Finding
dispositions and historical failure attribution are unchanged.

The [completed observer-entry batch](../reports/audits/2026-10-16-completed-observer-entry-ownership.md)
continues C16/C18/G1/G4/G7 and R7-1/2. Shared completed preparation now
retains its entry on abandonment. Direct registration and callback/exit-watch/
waiter containing owners explicitly consume child finish after local guards;
publication still transfers the same entry once. Ordinary private disposal keeps
weak/node-before-charge order. Active linked-entry/token/source/batch destruction,
aggregate admission and wider contexts remain separate. Finding dispositions
and historical failure attribution are unchanged.

The [lazy waiter-source owner batch](../reports/audits/2026-10-16-lazy-waiter-source-ownership.md)
continues C16/C18/G1/G4/G7 and R7-1/2. Original sponsor/weak observer,
candidate, actual first winner and shared waiter child now form one inert
transaction. Losing storage still releases after the slot guard and before
entry admission; ordinary results finish explicitly. Six handoff-stage
fallbacks retain complete fields, while published tokens retain entry authority.
Inner list-factory, active source/token/batch destruction, aggregate admission
and wider contexts remain separate. Finding dispositions and historical failure
attribution are unchanged.
