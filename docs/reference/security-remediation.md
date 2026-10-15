# Security remediation status and completion criteria

Reviewed 2026-10-15 against `1109d05a`, following the security audits and
subsequent fixes; numbered finding dispositions are unchanged. Source and tests
remain authoritative. Historical reports preserve the state at their own
revision; their remaining-work paragraphs are not a current backlog.

## Finding ledger

| State | Findings | Meaning |
| --- | --- | --- |
| Corrected in the documented scope | SEC-01, 02, 03, 05, 11, 12, 13, 16, 17, 19–38 | The identified defect has a correction and recorded validation. This is not a claim of complete subsystem security. |
| Partly corrected or mitigated | SEC-04, 06, 07, 14, 15, 18 | Specific paths are strengthened; the broader finding still has required work. |
| Open | SEC-08, 09, 10 | The required security boundary is not implemented. |

There are **38 numbered findings: 29 corrected, six partial and three open**.
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

The [HTTP listener correction](../reports/audits/2026-10-10-security-http-progress.md)
retains listeners through queue-full admission. The
[deployment ingress follow-up](../reports/audits/2026-10-11-security-deployment-ingress.md)
extends connection-local rejection to deployment setup/accept and makes its
pre-authentication parser allocation fallible without a second body copy.
Both endpoints have total request budgets; deployment remains serial. The same follow-up corrects shared identity creation ownership, the catalogue verification stack budget and scoped readiness observation. The read-only namespace query returns a live publication generation without granting invocation authority; legacy lookup stays denied. Final signed upload/release/pull/verification/launch/readiness passes. Earlier failed signed runs and intermittent AMD frame-router startup remain recorded separately; a passing repeat does not resolve that intermittence.

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

Remaining acceptance requires bounded concurrent admission and progress for
healthy clients while hostile clients occupy slots, bounded controller/RPC
completion, and sustained connection/packet flood, NIC failure and lifecycle
shutdown evidence on supported targets. Signatures checked after receipt do
not protect listener availability. Passing deadline/rejection probes do not
complete these fairness gates or SEC-07's essential-service pressure matrix.

## SEC-07: bounded resource admission

Implemented owners now cover capabilities, memory objects, heap/image frames,
private and fresh shared translation tables, runtime stacks, IOMMU backing,
IPC records/queues, CQ/completion records, timers, waiters, watches and observer
lists. Their individual contracts describe limits and retained backing. Stack
and translation-table admission listed as missing in the renewed audit has
since been implemented; do not repeat those historical gaps as current ones.

SEC-07 remains partial until all of the following deliverables are accepted:

| Deliverable | Completion evidence |
| --- | --- |
| R7-1: allocation inventory | Enumerate application-triggerable kernel allocation sites and identify their byte/count limits, sponsor, lifetime owner, rejection behavior and reserve classification. Include thread nodes, scheduler queues/migration storage, registry entries, namespace/sponsor metadata, callback captures and allocator bookkeeping. Explicitly justify bounded boot-only exclusions. |
| R7-2: remaining scheduler/registry storage | Prepare storage fallibly before publication. Every application-triggerable allocation rejects without a kernel allocation panic or partially published authority. Admission follows the actual allocation through detached work and weak references. Thread-node preparation is fallible today but does not yet have its own byte/principal pool. |
| R7-3: heap and aggregate sponsor budgets | Bound general kernel heap bytes and aggregate logical-principal consumption across domain creation/restart and outstanding work. Nested allocations and retained quarantine consume the original limits; ASID reuse, transfer, promotion and logical retirement cannot refund live backing or gain platform classification. |
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

Implemented paths retain exact roots and ownership through staged copy rollback,
IPC delivery/reply/cancellation, endpoint and namespace cleanup, memory/device
retirement, thread abort, cooperative runtime-page access and final root destruction. Backing is released only
after confirmed invalidation/hardware completion; uncertainty retains charges
and fences. QEMU CPU epochs, VT-d/AMD-Vi/SMMUv3 maintenance and the supported NVMe
reset path have execution evidence. Aborting threads is request submission, not
a quiescence receipt.

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

The next priority is trust and authenticated communication. Work on SEC-07/18
continues against the finite criteria above; it does not substitute for these
missing boundaries.

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
explicit. The protected installer/boot selector, initial platform-service roots
and recipient custody are still missing; **SEC-04 remains partial**.

An [x86 QEMU Secure Boot matrix](../guides/qemu-secure-boot.md) now exercises
firmware-to-Limine/config/kernel/policy integrity and signed-module consumption
under disposable test authority. Boot requires the exact installed test state,
rejecting stale/conflicting/uninstalled policies before publication. This is
execution evidence for that test chain; production enrollment, persistent
anti-rollback state, non-fixture platform-service roots and custody remain open.

The [DMA-map failure ownership follow-up](../reports/audits/2026-10-12-dma-map-failure-ownership.md)
continues C13/C18 and G2/G3/G7: per-call diagnostics retain map rejection rather
than collapsing it to zero, and failed shared-DMA setup retains its existing CPU
mapping without safe Rust access. Diagnostics and CPU unmap do not prove DMA
rollback or authorize retry. Public-operation readiness/IRQ qualification,
historical AMD causality, the retained Intel scheduled-close/ingress failure and
broader cleanup acceptance remain open. SEC-18 remains partial; finding counts
are unchanged.

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
