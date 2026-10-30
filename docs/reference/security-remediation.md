# Security remediation status and completion criteria

Current status consolidated 2026-10-30. This page records implemented scope,
remaining acceptance gates and unresolved incidents. Finding dispositions are
unchanged by documentation consolidation. Source and executable tests define
current behavior; this edit performs no new security qualification.

Start with [security engineering lessons](../guides/security-engineering.md) for
the reusable reasoning and [subsystem contributor contracts](contributor-contracts.md)
for implementation requirements. [Historical audits](../reports/README.md#audits)
retain their original findings and execution records. Their old remaining-work
paragraphs are not the current backlog.

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

Current contracts: [management TLS](management-tls.md),
[status observation](status-observation.md), [entropy](entropy.md),
[deployment ingress](deployment-ingress.md) and [ingress availability](ingress-availability.md).

Still required:

- Sustained healthy-client progress under hostile connections, packets and
  admitted-client pressure, including the serial status listener.
- Qualification of rendering, listener setup, receiver-held cancellation and
  teardown beyond their individual observation/input/output budgets.
- Integrated essential-service allocation and latency checks under R7-4/R7-5.
  Finite rejection/recovery probes do not establish flood fairness.

## SEC-07: bounded resource admission

| Implemented scope | Remaining boundary |
| --- | --- |
| Domain/node admission for capabilities, IPC, endpoints, CQs/completions, timers, watches/waiters and observer lists | Full application-triggerable allocation inventory, aggregate metadata/general-heap admission and exceptional caller contexts |
| Prepared admitted nodes for root, authority, IPC, device and memory publication/retirement; byte/count pools for selected metadata and scratch/snapshot storage | Allocator overhead, remaining nested/control-block storage and all-family principal accounting |
| Shared verified-principal limits for object/heap/image/private-table backing, full user-thread stack capacity and context/retirement metadata | Other families and general kernel heap; independent principals can still reach ordinary node limits |
| Original-charge retention through moves, weak references, quarantine, root retirement and generation reuse | Concurrent pressure/restart qualification and sustained essential-service progress |
| Central typed compile-time resource policy and reserved platform shares | Capacity sizing and fairness across complete multi-pool operations |

Contracts: [resource policy](resource-policy.md),
[principal backing](principal-backing-admission.md),
[principal history](principal-history-admission.md),
[thread metadata](thread-metadata-admission.md),
[translation](translation-admission.md), [stacks](stack-admission.md),
[memory-capability nodes](memory-capability-node-admission.md) and
[snapshot buffers](snapshot-buffer-admission.md).

SEC-07 remains partial until all five deliverables are accepted:

| Deliverable | Completion evidence |
| --- | --- |
| R7-1: allocation inventory | Enumerate application-triggerable kernel allocation sites and identify their byte/count limits, sponsor, lifetime owner, rejection behavior and reserve classification. Include thread nodes, scheduler queues/migration storage, registry entries, namespace/sponsor metadata, callback captures and allocator bookkeeping. Explicitly justify bounded boot-only exclusions. |
| R7-2: remaining scheduler/registry storage | Prepare storage fallibly before publication. Every application-triggerable allocation rejects without a kernel allocation panic or partially published authority. Admission follows the actual allocation through detached work and weak references. Thread contexts and retirement nodes now have shared requested-byte/count admission by signed principal and node; table capacity, observer/weak storage and other registry families still need admission. |
| R7-3: heap and aggregate sponsor budgets | Shared signed-principal object/heap/image/private-table, full user-thread-stack and context/retirement-node admission is implemented; complete other families and bound general kernel heap bytes and aggregate logical-principal consumption across domain creation/restart and outstanding work. Nested allocations and retained quarantine consume the original limits; ASID reuse, transfer, promotion and logical retirement cannot refund live backing or gain platform classification. |
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
| SEC-14, partial | Locked/pinned build inputs and a fail-closed complete-lockfile advisory CI job; the October 26 TLS migration removed the then-recorded vulnerable curve graph | Required-check enforcement on the remote repository, periodic scanning, license policy and protected release provenance. A recorded scan is point-in-time evidence. |
| SEC-15, partial | Specific credential/profile/SCRAM/hash/AES-GCM owners erase named storage; domain-owned entropy and selected backend guards | Complete TLS/library state and copy inventory, compiler/allocator/physical copies, forced termination and custody qualification. Ordinary Drop cannot promise erasure after abort. |

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
| October 28 Intel root `157:8`, object `35194`: `OperationDrainTimedOut`, one retirement pin, 17 passed/1 failed | Unresolved. On recurrence correlate the pin with the [captured map/unmap phase, generation and typed cause](memory-object-retirement.md#mapping-rejection-and-maintenance). The original capture cannot authorize a refund or attribute the failure to the RNG fix. [Captured failure](../reports/audits/2026-10-28-security-rng-ring-access.md#separate-x86-fixture-and-retirement-failures). |
| October 25 ARM scheduled first-entry exit-watch fixture deadline | Unresolved. The four-LP capture lacks the exact awaited target tuple. Failure-only target/generation/typed-wait diagnostics were added; passing repeats establish no cause. [Recorded scope and capture](../reports/audits/2026-10-25-security-memory-capability-preparation.md#preserved-initial-arm-failure). |
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
