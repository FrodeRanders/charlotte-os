# Security remediation status and completion criteria

Updated 2026-10-09, following the two security audits and subsequent fixes.
Source and tests remain authoritative. Historical reports preserve the state at
their own revision; their remaining-work paragraphs are not a current backlog.

## Finding ledger

| State | Findings | Meaning |
| --- | --- | --- |
| Corrected in the documented scope | SEC-01, 02, 03, 05, 11, 13, 16, 17, 19–30 | The identified defect has a correction and recorded validation. This is not a claim of complete subsystem security. |
| Partly corrected or mitigated | SEC-04, 06, 07, 14, 15, 18 | Specific paths are strengthened; the broader finding still has required work. |
| Open | SEC-08, 09, 10, 12 | The required security boundary is not implemented. |

There are **30 numbered findings: 20 corrected, six partial and four open**.
The earlier SEC-01–25 accounting was 15 corrected, six partial and four open;
it excluded the five additional, corrected findings SEC-26–30. Finding counts
measure bookkeeping, not percentage of effort or production readiness.

The [first audit](../reports/audits/2026-10-03-security-audit.md),
[renewed audit](../reports/audits/2026-10-05-security-audit.md),
[SEC-23–25 follow-up](../reports/audits/2026-10-05-security-follow-up.md), and
[report index](../reports/README.md) retain the evidence. This page gives the
remaining acceptance criteria, so a completed narrow fix need not reopen an
unbounded audit loop.

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
fallback, while retaining explicit ordinary cancellation. Published stack/backend
contexts, growth's enclosing thread-table guard and the complete call-chain
inventory remain required.
An initial Intel user-stack lease timeout is recorded as an unresolved progress
concern; atomic phase observations and timeout snapshots now improve subsequent
evidence. Fresh-run success does not close C16/G1/G2/G7 or SEC-18.

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
| 4 | **SEC-12:** Attenuate object-store authority to an enforced object set/namespace at the store boundary. Test denied reads/writes/listing and capability delegation; IDs and typed owners alone provide no tenant isolation. |
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
