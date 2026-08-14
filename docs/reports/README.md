# Historical reports

Everything below is a **point-in-time record**. Reports retain original
symptoms, findings, commit references, and validation evidence even after the
code changes. For current behavior, consult source/tests, the manual's status
appendix, and the living [`architecture/`](../architecture/README.md) and
[`reference/`](../reference/README.md) documents.

## Audits

- [2026-08 code/documentation cross-check](audits/2026-08-code-documentation-cross-check.md)
- [2026-08 distributed-systems audit](audits/2026-08-distributed-systems.md)
- [2026-07 functionality and logic audit](audits/2026-07-functionality-and-logic.md)

## Investigations

- [Live-upgrade stall](investigations/live-upgrade-stall.md)
- [Scheduler investigation](investigations/scheduler.md)
- [Intermittent AArch64 SMP context-switch panic](investigations/smp-context-switch-panic.md)

## Milestone records

- [Async-syscall demonstration](milestones/async-syscall-demo.md)

When a report produces a durable invariant, copy that invariant into the
appropriate reference document and link back to the report as evidence.
