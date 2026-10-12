# Historical reports

Everything below is a **point-in-time record**. Reports retain original
symptoms, findings, commit references, and validation evidence even after the
code changes. For current behavior, consult source/tests, the manual's status
appendix, and the living [`architecture/`](../architecture/README.md) and
[`reference/`](../reference/README.md) documents.

## Audits

- [Current security remediation ledger and completion criteria](../reference/security-remediation.md)
  — living status; historical reports below retain their original scope.
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
