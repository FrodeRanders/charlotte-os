# Security remediation status and completion criteria

Current status updated 2026-11-04. This page records implemented scope,
remaining acceptance gates and unresolved incidents. Finding dispositions are
unchanged by documentation consolidation. Source and executable tests define
current behavior; qualification records cover only their executed scope.

Start with [security engineering lessons](../guides/security-engineering.md) for
the reusable reasoning and [subsystem contributor contracts](contributor-contracts.md)
for implementation requirements. [Historical audits](../reports/README.md#audits)
retain their original findings and execution records. Their old remaining-work
paragraphs are not the current backlog.

The [SEC-07/18 closure plan](security-closure-plan.md) defines eight work packages:
Q1's six-site snapshot/scratch batch and Q2's launch/handoff/cache batch are
accepted, with six packages remaining and all four aggregate gates open. It also defines the combined acceptance
matrix. Its compiler census is a review baseline, not a completed
allocation/caller inventory or a new readiness percentage. Q3 is in progress;
its [boot allocation sub-batch](boot-allocation-boundaries.md) corrects CPU-count
validation and optional framebuffer ownership/failure cleanup while retaining
the unqualified geometry and remaining factory review. Finding dispositions
and existing unresolved incidents remain unchanged.

## Finding ledger

| State | Findings | Meaning |
| --- | --- | --- |
| Corrected in the documented scope | SEC-01, 02, 03, 05, 11, 12, 13, 16, 17, 19–38 | The identified defect has a correction and recorded validation; this does not qualify the entire subsystem. |
| Partly corrected or mitigated | SEC-04, 06, 07, 10, 14, 15, 18 | Implemented protections have the remaining acceptance boundaries below. |
| Open | SEC-08, 09 | The required security boundary is not implemented. |

There are **38 findings: 29 corrected, seven partial and two open**. These counts
are not an effort estimate or readiness percentage. Production trust remains
disabled. Historical SEC-12 reopening and recorrection, and SEC-10's transition
to partial after native mTLS, remain in their dated reports.

| Boundary | Current contract |
| --- | --- |
| Application isolation and authority | Exact mappings, captured identities, admitted capability publication and hidden IPC delivery; see [capability admission](capability-admission.md), [page-table lifetime](page-table-lifetime.md) and [IPC receive](ipc-receive.md). |
| Signed launch and placement | One signed descriptor, exact variant/runtime ABI and actual kernel execution profile; see [execution targets](execution-targets.md) and [scoped deployment](scoped-deployment-launch.md). |
| Persistent ownership and state | Tagged object-store/catalog records reject unsupported or unreadable metadata before mutation; see [object-store format](object-store-format.md), [catalog snapshots](catalog-snapshots.md) and [Raft persistence](raft-persistence.md). |
| Resource ownership and cleanup | Complete operation owners retain authority, roots, pins and original charges; completion proofs and continuation limits remain category-specific. See [cleanup coverage](cleanup-recovery.md). |
| Management and connector credentials | Enrolled management roles, owned TLS/entropy and scoped connector profiles; see [management TLS](management-tls.md), [TLS client](tls-client.md) and [secret-state lifetimes](secret-state-lifetimes.md). |

## SEC-31–38: renewed-audit findings

These eight findings remain corrected within scope. Current protections include
pre-admission fragment ceilings, caller-funded TCP/IP receive/status backing,
fallible signed launch, admitted platform classification, bounded discovery,
public-lookup policy separation and an upgrade-image ceiling. The
[renewed audit](../reports/audits/2026-10-10-security-audit.md) and its
[first](../reports/audits/2026-10-10-security-audit-remediation.md) and
[second corrections](../reports/audits/2026-10-10-security-audit-remediation-2.md)
retain the original defect descriptions.

The remaining implications belong to existing findings: bounded discovery does
not authenticate peers (SEC-08), and caller-funded pages do not establish
aggregate heap admission or service fairness (SEC-06/07).

## SEC-06: management progress

Implemented protections include bounded headers/bodies, one operation deadline,
owned CQ-driven deployment input/controller/output/close phases, native TLS 1.3
and exact reader/administrator authorization. Status dependency collection
shares one five-second observation budget across lookups and replies.
RNG reads use caller-funded memory, owned deadlines, endpoint readiness and a
fenced descriptor owner with checked halfword ring-index accesses. The frame
router observes its existing receive/forward/control interests on one CQ;
committed ingress eligibility excludes a silent backend from new flows after
projection convergence.
Status ingress now rotates four owned client positions across CQ-bound setup,
TLS/header input, output and logical close. Snapshot collection/rendering remains
synchronous; bounded concurrency alone does not establish flood fairness.
Deployment shutdown cancels incomplete input, continues accepted controller/output
work under original budgets, and drains all five client phases before acknowledging.
An [ARM idle-client shutdown control](../reports/audits/evidence/2026-10-30-management-drain.json)
checks this narrow boundary; stalled dependencies and wider cleanup contexts remain open.

Current contracts: [management TLS](management-tls.md),
[status observation](status-observation.md), [entropy](entropy.md),
[deployment ingress](deployment-ingress.md) and [ingress availability](ingress-availability.md).

Still required:

- Sustained healthy-client progress under hostile connections, packets and
  admitted-client pressure, including full status-pool occupancy and synchronous
  snapshot/rendering work.
- Qualification of rendering, listener setup, receiver-held cancellation and
  teardown beyond their individual observation/input/output budgets.
- Integrated essential-service allocation and latency checks under R7-4/R7-5.
  Finite rejection/recovery probes do not establish flood fairness.

## SEC-07: bounded resource admission

| Implemented scope | Remaining boundary |
| --- | --- |
| Domain/node admission for capabilities, IPC, endpoints, CQs/completions, timers, watches/waiters and observer lists | Full application-triggerable allocation inventory, aggregate metadata/general-heap admission and exceptional caller contexts |
| Prepared admitted nodes for root, authority, IPC, device and memory publication/retirement; byte/count pools for selected metadata and scratch/snapshot storage | Allocator overhead, remaining nested/control-block storage and all-family principal accounting |
| Shared verified-principal limits for object/heap/image/private-table backing, full user-thread stack capacity, context/retirement metadata and retained/candidate [thread-table](thread-table-admission.md)/[ASID-table backing](root-table-admission.md), with four-LP ARM/Intel/AMD qualification | Other families and general kernel heap; independent principals can still reach ordinary node limits |
| Original-charge retention through moves, weak references, quarantine, root retirement and generation reuse | Concurrent pressure/restart qualification and sustained essential-service progress |
| Central typed compile-time resource policy and reserved platform shares | Capacity sizing and fairness across complete multi-pool operations |
| One fixed 64-slot deployment registry reserves pending and published principals before launch; publication allocates no record backing | Native concurrent EL0 launch pressure, real connector-profile OOM and general heap/principal admission remain separate. See [deployment record admission](scoped-deployment-launch.md#deployment-record-admission). |
| [Inline node/device shutdown plans](live-address-space-operations.md#node-shutdown-plan-ownership) retain twelve service phases and three hardware owners without plan allocation or active abandonment cleanup; fallible worker preparation and fresh pinned admission precede roster transfer | Later cleanup admission, readiness and progress under real backing pressure remain open. Worker rejection retains the roster but still follows the existing syscall/agent fail-closed policy. |

Contracts: [resource policy](resource-policy.md),
[principal backing](principal-backing-admission.md),
[principal history](principal-history-admission.md),
[thread metadata](thread-metadata-admission.md), [thread-table backing](thread-table-admission.md),
[ASID-table backing](root-table-admission.md),
[translation](translation-admission.md), [stacks](stack-admission.md),
[memory-capability nodes](memory-capability-node-admission.md) and
[snapshot buffers](snapshot-buffer-admission.md).

SEC-07 remains partial until all five deliverables are accepted:

| Deliverable | Completion evidence |
| --- | --- |
| R7-1: allocation inventory | Enumerate application-triggerable kernel allocation sites and identify their byte/count limits, sponsor, lifetime owner, rejection behavior and reserve classification. Include thread nodes, scheduler queues/migration storage, registry entries, namespace/sponsor metadata, callback captures and allocator bookkeeping. Explicitly justify bounded boot-only exclusions. |
| R7-2: remaining scheduler/registry storage | Prepare storage fallibly before publication. Every application-triggerable allocation rejects without a kernel allocation panic or partially published authority. Admission follows the actual allocation through detached work and weak references. Thread contexts/retirement nodes and thread/ASID-table retained/candidate capacity have shared requested-byte/count admission by signed principal and node; table buffer preparation/disposal leaves local publication guards. Generation-owned memory-account nodes share root-metadata byte/count admission through final object-charge release, including ASID reuse. [Waiter-account bundles](waiter-account-admission.md) have independent count admission through final weak/allocator release, with immutable original backing classification. [Scheduler/completion timer-account bundles](timer-account-admission.md) also retain independent backing admission through final weak/allocator release. [Completion/CQ/watch account bundles](completion-account-admission.md) share independent backing admission through final weak/allocator release. [Capability/IPC/mailbox namespace accounts](namespace-account-admission.md) also share backing admission through final weak/allocator release with immutable captured storage classification. [Global namespace nodes](namespace-node-admission.md) retain requested-byte/count admission through preparation, detachment and actual deallocation. [Individual authority-record nodes](authority-node-admission.md) independently admit candidates before allocation and retain original byte/count charges through actual deallocation. [Mailbox endpoint nodes](mailbox-node-admission.md) independently admit candidate/retained backing and refund backing/resource charges only after node deallocation, including final-root invalidation rejection. [Deferred source-disposal nodes](deferred-source-admission.md) independently admit IPC/memory-close/DMA-readiness candidates and retain original bytes/count through producer-LP queueing and actual deallocation. [Observer/waker handles](observer-ownership.md) admit allocation bundles and actual Arc-layout bytes through final weak/private-holder release; runtime producers use captured sponsorship and borrowed notification. [Completion registry nodes](completion-node-admission.md) independently admit capability-backed, detached-operation and CQ candidates and retain original byte/count charges through backlog, retirement and actual deallocation. [Device namespace/payload nodes](device-node-admission.md) independently admit candidates before unified authority/hardware work and retain generation-captured backing class through actual deallocation; existing grant/close/namespace fallback remains inert under the pool guard. [IOMMU backend nodes](iommu-node-admission.md) independently admit domain/requester/context candidates and retain original class through complete grant, domain and unit lifetimes; persistent fences/context metadata survive domain destruction. Domain-owned DMA mapping/quarantine and SMMU walker-cache nodes use the same requested-byte/count pool with immutable grant-captured classification; rejection precedes allocation/publication and refund follows confirmed completion, unpin and actual node deallocation. [Shared typed admission and observer inventory](metadata-admission.md) cover these 24 independent metadata pools while preserving local ownership and cleanup authority. IOMMU table-ledger buffers share that pool's byte dimension, retaining original class and full old/candidate capacity through actual deallocation; physical uncertainty retains both page and metadata budgets. Memory-object registry records share the existing memory-metadata byte/count pool, retaining original backing-sponsor class through moves, physical failure and actual node disposal; allocation/copy third-node rejection precedes frame allocation and publication. Object frame-vector capacity shares the memory-metadata byte dimension with zero node count, retaining original class and full allocated capacity through popped identities, moves and physical failure; confirmed cleanup frees the buffer before refund. Pending-call and reply-token nodes have independent [IPC backing admission](ipc-node-admission.md), preserving original record class and both reservations through actual node disposal; complete call/reply fallback is inert under the new pool guard. Capability and endpoint payload nodes share that pool through actual disposal, with recipient namespace storage class separate from grantor/requester logical sponsorship. IPC backing limits accommodate the expanded layouts while preserving logical limits. Attachment retirement catalogs also retain byte-only admission through queueing, post-IPC delivery disposal and FIFO child handoff; callback interruption retains remaining owners and full allocated capacity. Parsed-descriptor, cap-ID, transfer-staging and reply-token loan vectors retain original byte admission through full old/candidate overlap and actual disposal; the complete vector-send parent retains partial transfers and roots on abandonment. IPC descriptor byte snapshots retain captured byte admission through the shared source-validation/copy boundary and actual disposal before submission roots complete; quota rejection precedes parsing/transfers and preserves source/descriptor ownership. Reply/cancellation revocation work retains original token byte admission and full capacity through actual disposal before root completion; complete parents retain returned-memory escrow, detached storage and notifications on abandonment. Cleanup capacity is not pre-reserved at call acceptance, so saturation can postpone reply/cancellation and root retirement. [Primitive syscall image/descriptor/pickup/plaintext/shutdown storage](snapshot-buffer-admission.md#primitive-syscall-inputs) shares captured byte admission and complete private-prefix cleanup; plaintext erases before actual free/refund. The reviewed snapshot byte defaults rise to 12/16 MiB ordinary/total to preserve protocol maxima. [Supervisor launch/handoff and store image ownership](service-launch-ownership.md) retain complete root/profile/transfer prefixes with explicit one-shot cancellation and inert parent abandonment. Handoff arrays share original-source IPC bytes; fixed store-cache publication reserves independent image counts/bytes through kernel-lifetime ownership, and denied/unreadable seeding stops instead of repairing. The image pool appends the 24th catalog entry in statistics version 11; both architectures require current signed bundles. Earlier loader/store-I/O helper contexts, other weak/control storage and remaining allocations still need review/admission. |
| R7-3: heap and aggregate sponsor budgets | Shared signed-principal object/heap/image/private-table, full user-thread-stack, context/retirement-node and thread/ASID-table backing admission is implemented; complete other families and bound general kernel heap bytes and aggregate logical-principal consumption across domain creation/restart and outstanding work. Nested allocations and retained quarantine consume the original limits; ASID reuse, transfer, promotion and logical retirement cannot refund live backing or gain platform classification. |
| R7-4: essential-service progress | Reserve usable allocation/admission capacity for trusted platform work. Demonstrate management, timer/scheduler and teardown progress during sustained ordinary pressure; separate pools alone are insufficient evidence of fairness. Rejected work must be bounded rather than busy-looping. |
| R7-5: pressure matrix | Run concurrent EL0 clients on all supported QEMU targets against ordinary and aggregate ceilings, allocation rejection, cancellation, weak retention, domain restart and generation reuse. Verify exact charges after confirmed cleanup and retained charges after uncertain cleanup. Record progress/latency against fixture deadlines chosen before the run. Include real backing pressure as well as counter-only saturation. |

R7-1 must leave no unexplained application-triggered allocation bypass. Explicitly
document fixed boot-only exclusions. New features extend the inventory and
pressure matrix. Existing table/stack admission and sponsor preparation must not
be reopened merely because an early audit predates them.

## SEC-18: quiescence and recovery

The [cleanup map](cleanup-recovery.md) identifies C01–C18 owners, continuation
rules and G1–G7 gaps. [Focused C16/C18 contracts](cleanup-recovery-details.md)
describe thread/stack and publication boundaries without the remediation history.
Safety retention is implemented across many failure paths; general recovery and
end-to-end caller qualification remain incomplete.

| Deliverable | Completion evidence |
| --- | --- |
| R18-1: locking and destructor inventory | Partial owner-family/context baseline: [C01–C18 and G1/G3/G4](cleanup-recovery.md). Complete every production call chain and implicit field drop, including wider scheduler/observer/allocation paths. Remove allocation, arbitrary callbacks, blocking hardware maintenance and physical destruction from masking serialization. Include ARM's enclosing reaper IRQ state. No destructor may assume that its caller holds no unrelated lock. |
| R18-2: cooperative shutdown ownership | Implemented runtime-page retention: exact root admission and fixed image-leaf identity checks precede cooperative drain/status access; force publication borrows its sweep lease. Deployment claims release registry guards before admission. Closing/reused roots and mismatched frames reject before access; ordinary completion releases the lease and abandonment retains image backing. See the [runtime-page contract](live-address-space-operations.md#supervisor-runtime-page-access) and [execution report](../reports/audits/2026-10-08-security-cooperative-pages.md). |
| R18-3: retained-owner recovery | Partial implementation: eight fixed slots retain complete detached roots rejected by final invalidation, with exact serial tickets, two explicit retries, unlocked physical work, masked-caller rejection, terminal abandonment/partial-release states and capability-scoped observer counts. See [final-root recovery](root-recovery.md). [G2/G5/G6](cleanup-recovery.md#named-gaps-and-acceptance-gates) track typed owner continuity, supervisor reconciliation and common authorized custody. Other receipt kinds and stronger reset/reboot qualification remain required; never clear fences or decrement counts to manufacture progress. |
| R18-4: QEMU device coverage | Extend reset contracts to supported VirtIO, AHCI and NIC paths where applicable. Exercise active outstanding I/O, withheld hardware completion, rejected reset, old-MMIO authority, restart and reassignment. The existing NVMe fixture enables real queues but submits no outstanding I/O; that gap remains. Unsupported devices must remain fenced and explicitly excluded from recovery support. |
| R18-5: concurrent recovery matrix | On Intel VT-d, AMD-Vi and Arm SMMUv3, combine peer/root close, loan/copy/DMA activity, thread switching, actual lost/stale CPU acknowledgements, hardware timeout, interrupted publication and partial physical-release rejection. Verify guard availability, no early pin/refund/reuse, later essential progress, and exact baseline recovery where confirmed completion permits it. |
| R18-6: physical-platform qualification | For each claimed production device/platform, establish CPU/device reset and drain guarantees, interrupt remapping/ATS policy and negative execution evidence on that hardware. QEMU evidence cannot satisfy this criterion. |

R18-1–5 define the **QEMU milestone**. R18-6 is a separate physical-platform
milestone; completing the QEMU work cannot close the broader SEC-18 finding.
Preserve exact owners, fences and charges throughout either milestone.

## Production security milestones

| Finding | Implemented boundary | Remaining acceptance |
| --- | --- | --- |
| SEC-04, partial | One immutable kernel trust policy, signed policy/key binding and an x86 disposable Secure Boot fixture | Protected executable boot/enrollment, rollback-resistant installed state, independent authorities and recipient-key custody. Public fixture keys/state never qualify production. |
| SEC-08, open | Bounded discovery and reliable-message session/sequence checks | Enrolled cryptographic node identities; authenticated/encrypted node/control/data channels bound to cluster, role, session and freshness; substitution/replay/revocation/restart controls. |
| SEC-10, partial | Native TLS 1.3 management, enrolled reader/administrator roles, signed mutations, x86 restart revocation, native date/session checks and scoped post-leader-loss direct/VIP progress | Protected credential installation/custody, ARM rotation, broader live replacement/revocation, session interruption and pressure/fault qualification. |
| SEC-09, open | SNTP/holdover, uncertainty reporting and fail-closed management checks on observed UTC loss/rollback | Authenticated security-time provenance and freshness/uncertainty across loss, rollback and restart. Deployment-agent-supplied expiry time and nonzero SNTP cannot establish this. |
| SEC-14, partial | Locked/pinned build inputs, a fail-closed complete-lockfile CI gate and a weekly/manual advisory workflow; the October 30 fresh 205-package scan has zero vulnerabilities/warnings | Hosted workflow activation and required-check enforcement, license policy and protected release provenance. Local workflow validation is not hosted execution; a recorded scan is point-in-time evidence. See [advisory operation](dependency-advisories.md). |
| SEC-15, partial | Specific credential/profile/SCRAM/hash/AES-GCM owners erase named storage; management and connector TLS share erasing SHA-256/HMAC contexts, including unfinished contexts and forks; domain-owned entropy and selected backend guards | Complete TLS/library state and copy inventory, remaining connector key-schedule/other hash state, compiler/allocator/physical copies, forced termination and custody qualification. Ordinary Drop cannot promise erasure after abort. See [secret-state lifetimes](secret-state-lifetimes.md). |

Trust installation/custody precedes production enrollment and authenticated
channels; authenticated security time is required before relying on expiry as a
security boundary. QEMU cleanup progress cannot discharge these milestones.
Use [bootstrap trust](bootstrap-trust-policy.md),
[management TLS](management-tls.md), [time service](time-service.md),
[dependency advisories](dependency-advisories.md) and
[secret-state lifetimes](secret-state-lifetimes.md) for the current contracts.

## Unresolved incidents

Keep recent unexplained failures visible and historical attribution limits
explicit. Similar symptoms are not evidence of one cause. A later successful run
does not resolve an earlier failure.

| Observation | Current status and next useful information |
| --- | --- |
| November 1 host TLS server/client-identity round trip: `Err(Handshake(32))` | Captured reproduction identifies an unknown SM2 offer in OpenSSL 4's CertificateRequest; the old parser aborts before reading the server certificate. Controlled same-binary OpenSSL 3.6/4.0 comparisons explain the reproduced class; original captures lacked peer versions/wire data. Corrected offer/selection checks pass the full host suite, nine parser controls and ARM Kafka mTLS/Intel S3 TLS campaigns. [Wire investigation and preserved failures](../reports/audits/2026-11-01-tls-signature-offers.md). |
| November 1 credential-free S3/Kafka campaigns: DHCP verifier waits for disabled httpd | Corrected: DHCP requires its actual status magic/address; listener/authentication and disabled-ingress assertions remain in the dedicated HTTP/management-startup probes. Credential-free ARM Kafka passes 20/0/0 and Intel S3 passes 18/0/0 with explicit disabled-startup assertions; a transport timeout remains unqualified. [Coverage map](boot-modes.md#network-verifier-boundaries), [current evidence and preserved failures](../reports/audits/evidence/2026-11-01-dhcp-verifier.json). No attribution of older NIC incidents. |
| October 30 AMD-Vi, eight CPUs: adversarial-probe deadline, 17 passed/1 failed | Fixture budget collision reproduced at `091ecfe8`. Separately bounded rollover and grant probes take about 13 and 37 seconds respectively under cancellation traffic; their sum exceeds the former shared 45-second budget. Separated AMD/8 and Intel/4 campaigns pass 18/0/0; ARM/4 passes 20/0/0, each with 20 host check groups and both repetitions/teardown required. Combined fixture allowance is larger; production deadlines and invalidation policy are unchanged. Sustained fairness and latency scaling remain unqualified. [Failed comparisons and qualification](../reports/audits/evidence/2026-10-30-security-fixture-budgets.json); no attribution of the historical retirement-pin failure. |
| October 28 Intel root `157:8`, object `35194`: `OperationDrainTimedOut`, one retirement pin, 17 passed/1 failed | Unresolved. On recurrence correlate the pin with the [captured map/unmap phase, generation and typed cause](memory-object-retirement.md#mapping-rejection-and-maintenance). The original capture cannot authorize a refund or attribute the failure to the RNG fix. [Captured failure](../reports/audits/2026-10-28-security-rng-ring-access.md#separate-x86-fixture-and-retirement-failures). |
| October 25 ARM scheduled first-entry exit-watch fixture deadline | Unresolved. The four-LP capture lacks the exact awaited target tuple. Failure-only target/generation/typed-wait diagnostics were added; passing repeats establish no cause. [Recorded scope and capture](../reports/audits/2026-10-25-security-memory-capability-preparation.md#preserved-initial-arm-failure). |
| October 30–31 ARM and October 31 Intel host signing subprocesses killed before guest boot | Cause unconfirmed. Preserve the separate SIGKILL/exit-137 captures; successful repeats do not explain them or qualify the failed bundles. No guest behavior was observed. [ARM capture](../reports/audits/evidence/2026-10-30-deployment-record-admission.json), [Intel capture](../reports/audits/evidence/2026-10-31-shutdown-worker-admission.json), [October 31 ARM capture](../reports/audits/evidence/2026-10-31-boot-modes.json). |
| IPC-close and memory-close exact-worker exit timeouts on historical builds | Historical attribution remains open. Park/first-dispatch corrections have their own regression evidence. On recurrence retain exact generation, exit publication, wait result, dispatch and staged-owner state. [Incident register](../reports/audits/2026-10-15-audit-process-review.md#unresolved-incident-register). |
| Intel/AMD user-stack retirement lease timeouts | Historical attribution remains open. Bounded pre-release invalidation reconfirmation addresses a possible transient class; it does not identify the earlier phase. [Investigation](../reports/audits/2026-10-10-security-user-stack-lease-timeout.md). |
| Historical Raft persistence fail-stop | Historical cause remains open. Write/flush and object-store operation diagnostics have since been separated; October 29 restart/process-loss successes qualify their captured cases only. [Persistence diagnostics](../reports/audits/2026-10-15-raft-persistence-diagnostics.md), [native recovery limits](../reports/audits/2026-10-29-raft-native-validation.md#limits). |
| Earlier NIC/storage startup, AMD router deadline, ARM stall and HTTP resets/EOF | Retain the separate episodes and missing-capture limitations. DMA/readiness/host-gate corrections explain their specified boundaries, not every older failure. [Historical register](../reports/audits/2026-10-15-audit-process-review.md#unresolved-incident-register). |

The incident register preserves original captures and verification limits. Do not
convert absent ASIDs, worker-return markers, wakes or timeouts into quiescence or
permission to restart cleanup.

## Maintenance and stopping criteria

- Update this page's current scope, acceptance gate or incident row when it
  changes. Put durable design knowledge in the relevant contract or the
  [engineering guide](../guides/security-engineering.md).
- Keep dated reports for investigations that need a historical record. Do not
  append a patch-by-patch narrative, routine build receipts or repeated finding
  counts to this ledger.
- Choose a finite target/workload/fault matrix and deadlines before a campaign.
  Keep failures separate from repeats and qualify only the executed scope.
- Prioritize reachable authority/admission bypasses, observed failures and
  integrated service progress. Turn a general concern into a specific call path,
  required property and acceptance check before expanding a refactor.
- Preserve the remaining QEMU and production gates. Consolidating documentation
  or archiving history never closes a finding or supplies missing physical proof.
