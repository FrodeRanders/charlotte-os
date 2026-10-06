# Historical reports

Everything below is a **point-in-time record**. Reports retain original
symptoms, findings, commit references, and validation evidence even after the
code changes. For current behavior, consult source/tests, the manual's status
appendix, and the living [`architecture/`](../architecture/README.md) and
[`reference/`](../reference/README.md) documents.

## Audits

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
