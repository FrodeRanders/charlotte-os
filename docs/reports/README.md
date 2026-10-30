# Historical reports

Everything below is a **point-in-time record**. Reports retain original
symptoms, findings, commit references, and validation evidence even after the
code changes. For current behavior, consult source/tests, the manual's status
appendix, and the living [`architecture/`](../architecture/README.md) and
[`reference/`](../reference/README.md) documents.

## Audits

The [audit archive](audits/README.md) indexes the dated reports and their retained
execution records. For current knowledge and work, use:

| Need | Document |
| --- | --- |
| Engineering lessons and failure patterns | [Security engineering](../guides/security-engineering.md) |
| Current dispositions, remaining gates and unresolved incidents | [Security status](../reference/security-remediation.md) |
| Ownership, cleanup and recovery boundaries | [Cleanup contracts](../reference/cleanup-recovery.md) |
| Detailed contributor requirements | [Subsystem contracts](../reference/contributor-contracts.md) |

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
