# Historical reports

Everything below is a **point-in-time record**. Reports retain original
symptoms, findings, commit references, and validation evidence even after the
code changes. For current behavior, consult source/tests, the manual's status
appendix, and the living [`architecture/`](../architecture/README.md) and
[`reference/`](../reference/README.md) documents.

## Audits

- [Bounded structured control inputs](audits/2026-10-21-security-control-input.md) — Covers deployment, release, shutdown and ingress file reads, with early release descriptor-count rejection.
- [Bounded operational file inputs](audits/2026-10-21-security-operational-input.md) — Bounds ciphertext and bundle references before decoding or notification, with early binding-count rejection and actual CLI controls.
- [Bounded public-key file inputs](audits/2026-10-21-security-public-key-input.md) — Shares trust-policy admission and decoding with operational and ingress-policy file arguments; preserves baseline rejection failures.
- [SEC-15 host plaintext profile input](audits/2026-10-20-security-host-profile-input.md) — Owns the bounded read destination before I/O and rejects unsafe Unix file sources before sealing.
- [SEC-15 host key-file encoding](audits/2026-10-20-security-host-key-encoding.md) — Reserves the complete erasing hex-plus-newline buffer for key generation, avoiding output growth.

- [SEC-15 SCRAM initial owners](audits/2026-10-20-security-scram-initial-owners.md) — Removes escaped-username scratch and owns the phase-local nonce through consuming transitions.

- [SEC-15 Kafka profile encoding](audits/2026-10-20-security-kafka-profile-encoding.md) — Returns the credential-bearing frame under an erasing owner and removes aggregate profile/authentication Debug.

- [SEC-15 S3 profile encoding](audits/2026-10-20-security-s3-profile-encoding.md) — Returns the credential-bearing frame under an erasing owner and removes aggregate profile Debug.

- [SEC-15 S3 profile owners](audits/2026-10-20-security-s3-profile-owners.md) — Keeps the service-local profile in one erasing frame and owns reserved HTTP authority text.

- [SEC-15 request-key/raw-path owners](audits/2026-10-20-security-s3-path-owners.md) — Owns the decoded key and reserved raw path while preserving the IPC mapping boundary and path rules.

- [SEC-15 response metadata owners](audits/2026-10-20-security-s3-metadata-owners.md) — Consumes the response header and lends metadata ranges, removing independent metadata string copies.

- [SEC-15 canonical signing owners](audits/2026-10-20-security-s3-canonical-owners.md) — Owns and reserves canonical URI/query/header copies and returned signing metadata.

- [SEC-15 S3 request owners](audits/2026-10-19-security-s3-request-owners.md) — Owns authorization/signature text and reserves the complete erasing HTTP request header.

- [SEC-15 S3 response buffers](audits/2026-10-19-security-s3-response-buffers.md) — Removes header/decoder output growth and erases consumed or abandoned chunk-framing scratch.

- [SEC-15 receive owners](audits/2026-10-19-security-receive-owners.md) — Owns TLS/plain receive chunks and Kafka partial frames, removes frame growth, and fences failed source episodes.

- [SEC-15 SASL frame ownership](audits/2026-10-19-security-sasl-frames.md) — Removes reply authentication copies, consumes complete frames on rejection, and validates request size before guarded encoding.

- [SEC-15 Base64 scratch](audits/2026-10-19-security-base64-scratch.md) — Reproduced decoded-prefix retention on rejection and decoder growth; helpers now create and transfer zeroizing output owners.

- [SEC-15 HPKE erasure](audits/2026-10-19-security-hpke-erasure.md) — Reproduced missing X25519 Drop erasure, required its dependency feature, and transferred generated private bytes in a zeroizing owner.

- [SEC-15 SCRAM phase ownership](audits/2026-10-19-security-scram-phases.md) — Consuming challenge/final phases release transient secrets and prevent reuse after rejection, with host and broker-fault execution evidence.

- [SEC-15 hash-state erasure](audits/2026-10-19-security-hash-state-erasure.md) — Reproduced unfinished SHA state retention, erased owned hash scratch, and shared owned HMAC results across SigV4/SCRAM.

- [SEC-04 early platform artifact authority — 19 October 2026](audits/2026-10-19-security-platform-artifact-authority.md): one selected key before platform loading, wrong-root NS/NVMe controls, and extended boot-trust models; protected state and custody remain open.

- [SEC-04 exact boot-verification receipts](audits/2026-10-19-security-boot-receipts.md) — Shared exact-installed verification and a distinct kernel input, retaining production refusal and protected-state/custody limits.

- [Raft snapshot continuation ownership](audits/2026-10-19-raft-snapshot-streams.md) — Reproduces mixed-stream installation, binds buffered chunks to leader/term, and adds owner/offset model controls.

- [Raft per-read quorum fences](audits/2026-10-19-raft-read-fences.md) — Replaces the query lease; the former stale read rejects, with owned asynchronous DNS routing and four model controls.

- [Raft elapsed time and reproduced lease gap](audits/2026-10-18-raft-clock-lease-gap.md) — Corrects reactor clock accounting; an honest delayed reply still enables a stale application read.

- [Raft read prerequisites](audits/2026-10-18-raft-read-prerequisites.md) — Rejects reads before current-term commit/application; timing and correlation remain open.

- [Graft membership parity and demoted leadership](audits/2026-10-18-raft-membership-parity.md) — Restored proposed-member catch-up, immediate learner step-down with vote retention, and a dated Java/C++/Rust and Jepsen scope comparison.

- [Raft response term and progress fencing](audits/2026-10-18-raft-response-fencing.md) — Reproduced stale-term unsafe commit, corrected response admission/progress, and repaired snapshot join-completion coverage.

- [Application snapshot acceptance model](audits/2026-10-18-snapshot-restore-model.md) — Rejected restore preserves live state and Raft publication; recovery requires acceptance, with seven required counterexamples.

- [Boot trust and conditional installation model](audits/2026-10-18-boot-trust-model.md) — Exact installed boot acceptance, immutable shared policy and conditional commit/reboot ordering; protected production boundaries remain absent.

- [NS publication and endpoint-death model](audits/2026-10-18-name-publication-model.md) — Public/policy gates, admitted watch ownership, exact death retirement and corrected signed generation exhaustion.

- [Coherent-DMA CPU remap fence](audits/2026-10-18-coherent-dma-remap-fence.md) — Reproduced and closed fresh CPU mapping while coherent DMA pins remain; ARM and Intel execution evidence.
- [DMA operation and readiness model](audits/2026-10-18-dma-operation-model.md) — Complete unit claims, map/unmap admission versus completion, restricted failure owners and non-replayable cleanup.

- [Capability and IPC publication model](audits/2026-10-18-ipc-publication-model.md) — Captured record admission, source escrow, hidden delivery, namespace reuse and detached metadata charges.

- [Security model synchronization](audits/2026-10-18-security-model-sync.md) — Root-retirement and fallible-wait safety models, required counterexamples and remaining formal coverage gaps.

- [Telemetry snapshot preparation and status-page lifetime](audits/2026-10-18-telemetry-snapshot-preparation.md) — Bounded fallible capture/encoding, exact-generation heap reads and borrowed closing-root sampling.

- [Manual and living-documentation source comparison](audits/2026-10-18-documentation-reality-sweep.md) — Corrects capability/API, recovery, admission, trust, bootstrap and build/CI claims; records the source scope and verification limits.

- [Principal history preparation](audits/2026-10-18-principal-history-preparation.md) — Prepares admitted history storage before root publication and retains its original metadata through retirement.

- [Signed-authority slot preparation](audits/2026-10-18-root-authority-preparation.md) — Prepares identity storage with the root; exact generation installation and receipt retention through invalidation.

- [Root policy and telemetry metadata preparation](audits/2026-10-18-root-policy-preparation.md) — Fallible limit/usage nodes before publication; exact nodes retained in the root retirement receipt through invalidation.

- [Memory sponsor metadata preparation](audits/2026-10-18-memory-sponsor-preparation.md) — Fallible account/sponsor preparation before root publication; exact generation lifetimes and post-ledger metadata disposal.

- [Kernel stack range ownership](audits/2026-10-17-kernel-stack-ownership.md) — Removes infallible guard-map allocations; exact stack owners fence teardown while retaining the containing root and admission.

- [Security status source review](audits/2026-10-17-security-status-review.md) — Current finding accounting, sampled corrections, concrete remaining allocation/destructor boundaries and fresh host validation.

- [Detached submission preparation](audits/2026-10-16-detached-submission-preparation.md) — Record/event admission and node/observer allocation leave the registry; exact namespace/CQ revalidation and once-only enqueue retain original charges.

- [Detached timer terminal disposal](audits/2026-10-16-detached-timer-disposal.md) — Complete timer-bearing records leave the registry before disposal; direct/backlog charge lifetimes and exact-source notification remain distinct.

- [CQ payload preparation](audits/2026-10-16-cq-payload-preparation.md) — CQ admission, list/backlog/ring storage and partial rejection leave the registry; exact namespace revalidation precedes physical writes and replacement.

- [Terminal completion observer disposal](audits/2026-10-16-completion-terminal-disposal.md) — Detached event/worker owners leave local completion guards; installation disposal and terminal wakes preserve exact source identity.

- [Completion callback preparation and source disposal](audits/2026-10-15-completion-callback-preparation.md) — Exact-operation callback preparation outside registry/inner guards; complete successful namespace/CQ detachments dispose after unlock.
- [Waiter source preparation](audits/2026-10-15-waiter-source-preparation.md) — Lazy list allocation and IPC/CQ entries leave local source guards; exact-source revalidation, readiness races and post-guard disposal.
- [Scheduler wait preparation and publication](audits/2026-10-15-scheduler-wait-publication.md) — Registration and cancellation leave local LP/table guards; early hints and exact installed-wait identity close publication races.
- [Inline scheduler run queues](audits/2026-10-15-inline-scheduler-run-queues.md) — Ready storage follows each complete thread; no ring growth, migration rejects before source removal, and linked owners reject physical cleanup.
- [Scheduler selection storage and x86 dispatch gate](audits/2026-10-15-scheduler-selection-storage.md) — Allocation-free load/candidate scans, explicit caller inventory and never-dispatched-only x86 migration; run-queue growth remains open.
- [Retained stack-growth execution ownership](audits/2026-10-15-stack-growth-execution-owner.md) — Original thread/root/stack charges retained through growth outside the thread table; real EL0 growth, deferred-abort and foreign-leaf rollback evidence.
- [Thread-exit watch preparation](audits/2026-10-15-thread-exit-watch-preparation.md) — Captured-generation preparation outside the thread table/source mutex, allocation-free publication and exact-episode rejection; scoped C16/R7/G1/G4 evidence.
- [Captured park-delivery fixture panic and current-build matrix](audits/2026-10-15-captured-park-fixture-panic.md) — Spontaneous all-LP failure capture, forced-switch reproduction of an invalid Running-only assertion, and bounded cross-architecture follow-up.
- [Automatic guest failure capture](audits/2026-10-15-automatic-failure-capture.md) — Independent ARM/x86 watcher, private all-LP snapshots and immutable run inputs; native deadline controls and explicit capture limitations.
- [Raft persistence failure diagnostics](audits/2026-10-15-raft-persistence-diagnostics.md) — Per-call typed errors and distinct write/flush fail-stop context through existing owners; historical attribution remains open.
- [2026-10-15 audit process and outstanding failure review](audits/2026-10-15-audit-process-review.md) — Current SEC accounting, corrected versus unattributed panics, diagnostic gaps and ordered next work; fresh host validation, no new guest claims.
- [First-dispatch retirement/readiness correction](audits/2026-10-15-first-dispatch-delivery.md) — Incoming-stack handoff, failing negative control and passing ARM/x86 suites; historical IPC/persistence attribution remains open.
- [CI Sitas runtime pin and panic diagnostics](audits/2026-10-15-ci-sitas-runtime-pin.md) — Reproduced obsolete-wait failure, corrected dependency pin and restored frame pointers.
- [IPC close-worker exit investigation](audits/2026-10-14-ipc-close-exit-investigation.md) — Exact-source/phase diagnostics and bounded four-LP repeated closes; historical timeout remains open.
- [Correlated DNS registration/retraction](audits/2026-10-14-protocol-simplification-dns-registration-relay.md) — Full names, prepared replies and owned registration phases; mixed/HTTP execution and separately retained IPC exit timeout.
- [Current security remediation ledger and completion criteria](../reference/security-remediation.md)
  — living status; historical reports below retain their original scope.
- [One scoped launch owner](audits/2026-10-13-protocol-simplification-launch.md) — Signed launch returns its verified principal; packed/name aliases and zero-principal singleton retirement are removed.
- [Current-only catalog snapshots](audits/2026-10-13-protocol-simplification-catalog.md) — Retired snapshot decoders removed; explicit restore rejection precedes Raft log/membership publication; deployment queries require complete sections.
- [Current-only Raft persistence](audits/2026-10-13-protocol-simplification-raft.md) — Tagged two-object pair, strict recovery, owned IPC transport and future migration policy.
- [Current-only discovery advertisements](audits/2026-10-13-protocol-simplification-discovery.md) — Complete declared-length payloads replace optional cluster fields and base-only fallbacks.
- [Current-only deployment descriptors](audits/2026-10-13-protocol-simplification-deployment.md) — First simplification batch: retired signed grammars reject; current writers, callers and living documentation agree.
- [Timed completion outcomes (SEC-18 / C16, C18)](audits/2026-10-12-completion-wait-outcomes.md) — Explicit kernel wait admission errors, shared deadline loop and exact-generation close-fixture diagnostics.
- [DMA-map failure ownership (SEC-18 / C13, C18)](audits/2026-10-12-dma-map-failure-ownership.md) — Per-call rejection diagnostics and restricted shared-DMA failure ownership.
- [DMA startup preparation and typed failures (SEC-18 / C01, C15, C18)](audits/2026-10-12-dma-startup-preparation.md) — Boot-driver grant ordering, retained-root preparation and per-return cause/cancellation diagnostics.
- [Exact IPC close-claim readiness (SEC-18 / C07–C09)](audits/2026-10-12-ipc-close-readiness.md) — Retained-root and claim-epoch waits, rollback/failure notifications and owned endpoint-front readiness.
- [Exact transfer-close readiness (SEC-18 / C18)](audits/2026-10-12-memory-close-readiness.md) — Retained-root waits, charged source disposal and shared producer-LP deferred waiter delivery.
- [Event-driven LP reapers (SEC-18 / C16)](audits/2026-10-12-reaper-readiness.md) — Owned retirement-head waits and deferred exact-LP wakes replace one-millisecond idle polling.
- [Pending reply CQ readiness](audits/2026-10-12-reply-cq-readiness.md) — Owning reply interests, exact CQ identity and lifecycle notification for multi-source controllers.
- [Thread metadata retention (SEC-18 / C16 / G1)](audits/2026-10-12-thread-metadata-retention.md) — Complete guarded fallback retention and explicit metadata disposal after confirmed stack-pair release.
- [Weekly wait/readiness sweep](audits/2026-10-12-wait-readiness-sweep.md) — Reply polling replaced by owned timed readiness waits; remaining cleanup, reaper and shootdown notification gaps documented.
- [Object-store readiness and unreadable mount metadata](audits/2026-10-12-security-object-store-readiness.md) — Explicit serving state, immutable first-I/O diagnostics and fail-closed metadata read policy; historical Intel failure retained.
- [Deployment controller RPC deadlines](audits/2026-10-12-security-controller-deadlines.md) — Seven bounded ingress RPCs, scalar/moved cancellation and original-sponsor page/queue recovery under pressure.
- [Platform publication deadlines](audits/2026-10-12-security-publication-deadlines.md) — Bounded TCP/IP registration, five further owner-based startup paths, accepted-call cancellation and full-queue/silent-endpoint recovery evidence.
- [Network endpoint publication under admission pressure](audits/2026-10-12-security-network-publication.md) — Retained AMD startup failure, NIC diagnostics, owned publication and exact delegated admission status.
- [Deployment ingress and scoped publication progress](audits/2026-10-11-security-deployment-ingress.md) — Connection-local setup/accept failure, fallible parser backing, ownership-preserving identity publication and deployment HTTP recovery probes.
- [HTTP listener progress under security queue pressure](audits/2026-10-10-security-http-progress.md) — Failed packet/phase captures, exact admission errors and listener-preserving retries.
- [TCP/IP caller buffers and negative audit probes](audits/2026-10-10-security-tcpip-caller-buffers.md) — Receive/status sponsorship, retention and lifecycle evidence; legacy lookup, upgrade and fragment rejection.
- [2026-10-10 object-store isolation fixes, policy revocation and mutex-context probes](audits/2026-10-10-security-object-store-follow-up.md)
- [2026-10-10 commit progress verification and SEC-12 isolation follow-up](audits/2026-10-10-security-progress-verification.md)
- [2026-10-10 object-store operator scope policy (SEC-12)](audits/2026-10-10-security-object-store-scope-policy.md)
- [2026-10-10 object-store per-object ownership (SEC-12)](audits/2026-10-10-security-object-store-ownership.md)
- [2026-10-10 object-store reserved-namespace authority (SEC-12)](audits/2026-10-10-security-object-store-reserved-namespace.md)
- [2026-10-10 intermittent user-stack lease timeout investigation](audits/2026-10-10-security-user-stack-lease-timeout.md)
- [2026-10-10 live-upgrade IOMMU boot initialization regression](audits/2026-10-10-security-live-upgrade-iommu-init.md)
- [2026-10-10 renewed-audit remediation (SEC-36–38)](audits/2026-10-10-security-audit-remediation-2.md)
- [2026-10-10 renewed-audit remediation (SEC-31–35)](audits/2026-10-10-security-audit-remediation.md)
- [2026-10-10 renewed security audit (SEC-31–38)](audits/2026-10-10-security-audit.md)
- [2026-10-10 mailbox word-ring backing admission](audits/2026-10-10-security-mailbox-backing-admission.md)
- [2026-10-10 mailbox word-queue storage](audits/2026-10-10-security-mailbox-queue-storage.md)
- [2026-10-10 final-root mailbox and authority metadata retirement (SEC-07/18)](audits/2026-10-10-security-root-metadata-retirement.md)
- [2026-10-10 mailbox publication and explicit close contexts (SEC-07/18)](audits/2026-10-10-security-mailbox-publication.md)
- [2026-10-10 admitted DMA mapping records and SMMU walker cache (SEC-07/18)](audits/2026-10-10-security-dma-metadata-storage.md)
- [2026-10-09 admitted IOMMU backend registry storage (SEC-07/18)](audits/2026-10-09-security-backend-registry-storage.md)
- [2026-10-09 device authority preparation and retirement contexts (SEC-07/18)](audits/2026-10-09-security-device-authority-context.md)
- [2026-10-09 prepared unified capability authority records (SEC-07/18)](audits/2026-10-09-security-capability-record-storage.md)
- [2026-10-09 prepared unified capability namespace storage (SEC-07/18)](audits/2026-10-09-security-capability-namespace-storage.md)
- [2026-10-09 prepared device registry storage and post-guard disposal (SEC-07/18)](audits/2026-10-09-security-device-registry-storage.md)
- [2026-10-09 DMA close claim without allocating reinsertion (SEC-18)](audits/2026-10-09-security-dma-close-claim.md)
- [2026-10-09 complete-unit DMA creation outside local serialization (SEC-18)](audits/2026-10-09-security-dma-creation-phases.md)
- [2026-10-09 PCI reset claim retained through DMA publication (SEC-18)](audits/2026-10-09-security-pci-reset-claim.md)
- [2026-10-09 complete DMA map/unmap ownership through unlocked maintenance (SEC-18)](audits/2026-10-09-security-dma-mapping-maintenance.md)
- [2026-10-09 claimed IOMMU boot initialization (SEC-18)](audits/2026-10-09-security-iommu-unit-initialization.md)
- [2026-10-09 typed private DMA construction rollback (SEC-18)](audits/2026-10-09-security-dma-private-rollback.md)
- [2026-10-09 concurrent abort-sweep executor ownership (SEC-18)](audits/2026-10-09-security-abort-executor.md)
- [2026-10-09 abort-sweep root completion before caller retirement (SEC-18)](audits/2026-10-09-security-abort-handoff.md)
- [2026-10-09 IOMMU table/region preparation abandonment (SEC-18)](audits/2026-10-09-security-iommu-preparation-abandonment.md)
- [2026-10-09 explicit published stack-pair retirement (SEC-18)](audits/2026-10-09-security-published-stack-retirement.md)
- [2026-10-09 initial/growth stack preparation abandonment (SEC-18)](audits/2026-10-09-security-stack-preparation-abandonment.md)
- [2026-10-09 table/raw-frame abandonment and stack-retirement diagnostics (SEC-18)](audits/2026-10-09-security-table-abandonment.md)
- [2026-10-09 heap/image and kernel-range abandonment without cleanup (SEC-18)](audits/2026-10-09-security-preparation-abandonment.md)
- [2026-10-08 cross-category cleanup and recovery strategy (SEC-07/18)](audits/2026-10-08-security-recovery-strategy.md)
- [2026-10-08 bounded final-root recovery receipts (SEC-18)](audits/2026-10-08-security-root-recovery.md)
- [2026-10-08 cooperative shutdown runtime-page ownership (SEC-18)](audits/2026-10-08-security-cooperative-pages.md)
- [2026-10-08 QEMU Secure Boot chain and signed-policy consumption (SEC-04)](audits/2026-10-08-security-qemu-secure-boot.md)
- [2026-10-07 one-shot kernel boot trust handoff (SEC-04)](audits/2026-10-07-security-boot-trust-handoff.md)
- [2026-10-07 signed bootstrap policy and revision lineage (SEC-04)](audits/2026-10-07-security-signed-trust-policy.md)
- [2026-10-07 public trust preflight and remediation criteria (SEC-04/07/18)](audits/2026-10-07-security-trust-preflight.md)
- [2026-10-07 whole-domain thread abort ownership (SEC-07/18)](audits/2026-10-07-security-domain-thread-abort.md)
- [2026-10-07 prepared deferred thread retirement (SEC-07/18)](audits/2026-10-07-security-thread-retirement.md)
- [2026-10-07 prepared slot-return storage (SEC-07/18)](audits/2026-10-07-security-slot-return-storage.md)
- [2026-10-07 fallible timer observer allocation (SEC-07/18)](audits/2026-10-07-security-timer-observer-allocation.md)
- [2026-10-07 provisional kernel-frame preparation (SEC-18)](audits/2026-10-07-security-kernel-preparation.md)
- [2026-10-07 observer-list backing admission (SEC-07)](audits/2026-10-07-security-observer-lists.md)
- [2026-10-07 timer backing admission (SEC-07)](audits/2026-10-07-security-timer-backing.md)
- [2026-10-07 completion backing admission (SEC-07)](audits/2026-10-07-security-completion-backing.md)
- [2026-10-07 bounded kernel-frame release (SEC-18)](audits/2026-10-07-security-kernel-release.md)
- [2026-10-07 IOMMU backing admission (SEC-07)](audits/2026-10-07-security-iommu-admission.md)
- [2026-10-07 runtime stack admission (SEC-07)](audits/2026-10-07-security-stack-admission.md)
- [2026-10-06 shared kernel-table admission (SEC-07)](audits/2026-10-06-security-kernel-table-admission.md)
- [2026-10-06 private translation-table admission (SEC-07)](audits/2026-10-06-security-table-admission.md)
- [2026-10-06 staged-copy rollback and QEMU quiescence/recovery (SEC-18)](audits/2026-10-06-security-staged-quiescence.md)

- [2026-10-06 IPC backing release outside serialization (SEC-18 follow-up)](audits/2026-10-06-security-ipc-backing-release.md)

- [2026-10-06 returned-memory source qualification (SEC-18 follow-up)](audits/2026-10-06-security-source-qualification.md)

- [2026-10-06 IPC connection delivery visibility (SEC-30)](audits/2026-10-06-security-connection-delivery.md)

- [2026-10-06 IPC owned-memory delivery visibility (SEC-29)](audits/2026-10-06-security-memory-delivery.md)

- [2026-10-06 owned memory retirement and unmapped revocation peers (SEC-18/28)](audits/2026-10-06-security-memory-retirement.md)

- [2026-10-06 owned device retirement and DMA loan revocation (SEC-18/27)](audits/2026-10-06-security-device-retirement.md)

- [2026-10-06 owned namespace loan cleanup and assembly permissions (SEC-18/26)](audits/2026-10-06-security-namespace-close.md)
- [2026-10-06 owned explicit endpoint close (SEC-18 follow-up)](audits/2026-10-06-security-endpoint-close.md)
- [2026-10-06 bulk IPC cleanup failure propagation (SEC-18 follow-up)](audits/2026-10-06-security-bulk-cleanup.md)
- [2026-10-05 security follow-up and remediation (SEC-23–25)](audits/2026-10-05-security-follow-up.md)

- [2026-10-05 security remediation](audits/2026-10-05-security-remediation.md)
- [2026-10-05 renewed security audit](audits/2026-10-05-security-audit.md)
- [2026-10-03 security audit](audits/2026-10-03-security-audit.md)
- [2026-10-03 security remediation and remaining work](audits/2026-10-03-security-remediation.md)
- [2026-08 code/documentation cross-check](audits/2026-08-code-documentation-cross-check.md)
- [2026-08 distributed-systems audit](audits/2026-08-distributed-systems.md)
- [2026-07 functionality and logic audit](audits/2026-07-functionality-and-logic.md)

## Investigations

- [Kafka soak scratch-window exhaustion](investigations/2026-09-14-kafka-soak-scratch-window-exhaustion.md)
- [DMA isolation and hostile-device security](investigations/2026-08-30-dma-security.md)
- [Frame-allocator interrupt-masking latency](investigations/2026-08-16-frame-allocator-irq-latency.md)
- [Live-upgrade stall](investigations/live-upgrade-stall.md)
- [Scheduler investigation](investigations/scheduler.md)
- [Intermittent AArch64 SMP context-switch panic](investigations/smp-context-switch-panic.md)

## Milestone records

- [Async-syscall demonstration](milestones/async-syscall-demo.md)

When a report produces a durable invariant, copy that invariant into the
appropriate reference document and link back to the report as evidence.
