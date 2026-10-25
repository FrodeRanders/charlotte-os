# Audit follow-up: detached completion lifecycle

Date: 2026-10-25. Reviewed baseline: `4125c0be`.

This continues the [IPC transaction consolidation](2026-10-25-refactoring-ipc-transactions.md).
Capability-free completion preparation and retirement now share
[`completion/detached.rs`](../../../crates/catten/src/completion/detached.rs).
The completion core falls from 2210 to 1981 lines. Its registry, CQ backing/
backlog, capability-backed operations, waits and deadline policy remain in place.

## Boundary and preserved contracts

The new private module contains the existing captured `Target`, complete
`Preparing` prefix owner, `DetachedOp`, typed timer observer, terminal
`RetiredDetached` receipt and their preparation/publication/cancellation/finish
functions. The former standalone `detached_preparation` module is removed.
Root public entry points are reexported with the same signatures and behavior.
Private factory callers use the current explicit `submit_with_ids` and
`submit_timer_with_ids` helpers; the former paths have no compatibility alias.

Detached operation correlation/CQ identity, cancellation flags, timer observer
and cancellation custody are private to the lifecycle module. Only its original
record-charge field remains visible to the completion family for existing CQ
backlog-to-refund transfer. The terminal receipt is private and keeps its record,
typed timer owners, refund batch and captured notification source together.
No new owner, registry, account or resource reservation is introduced.

Preparation still captures the exact root/accounts/capacity/classification and
queue source before unlocked factories. Publication revalidates that capture,
then transfers the same complete node once; accepted timers enqueue once after
unlock. Ordinary rejection finishes after local guards. Preparation and terminal
receipt abandonment retain their complete fields without cleanup.

Direct, backlog and missing-CQ results retain their existing charge lifetimes.
Retirement disposes the remaining node, timer/observer custody and original
charge after registry unlock, before signaling the captured source. Non-timer
cancellation still waits for the real producer. Cancellation or readiness is no
physical completion, refund or replay authority. Shared CQ ordering, work
generations, ring sanitization and wait/deadline behavior are unchanged.

## Review and validation

The [evidence](evidence/2026-10-25-refactoring-completion-detached.json) records
source hashes, comparisons and native executions. Comparison of all moved
factory, publication, completion/cancellation and retirement bodies matches the
baseline after module/private-name routing, visibility, comments, whitespace
and optional trailing commas are normalized. The remaining completion core
matches after removing the extracted blocks and new module/import plumbing.
These are review aids, not executable-refinement or arbitrary-context proofs.

All three native fixture bodies move byte for byte: preparation, metadata
admission and retirement/timer disposal. Their order is unchanged: retirement,
preparation, then metadata. Other fixture source changes only update module/
private-helper routing and rustfmt wrapping. No new mirror test or alternate
implementation is introduced. Strict base-kernel Clippy passes for AArch64 and
x86_64 with the signed bundle inputs and
`--no-default-features --features acpi -- -D warnings`.

Four-LP native results on the frozen source, with the architectures run
sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs execute the moved preparation, metadata-retention and timer-disposal
controls. They cover unlocked factory rejection, heap-held publication/removal,
direct/backlog/missing-CQ terminal paths, original charges, duplicate rejection,
cancellation/firing producers, queue/root replacement and guarded abandonment.
Operation-ID exhaustion and shared CQ backlog/wait controls remain enabled.
Both runners finish with `reason=complete`. Previously rebuilt signed service
bundles are reused; no userspace source changes. Workspace formatting,
whitespace and local references pass.

Each architecture logs **four ConnectionResetError retries**: one before the
successful EOF recovery probe, one before abortive-peer recovery, and two before
idle-client recovery. The evidence preserves all eight diagnostics with their
eventual passing probe results. These are retried attempts inside the reported
runs, not separate clean executions. No controlled comparison attributes them
to this refactoring, host scheduling or the historical metrics failures.

This refactoring changes no configured capacity, userspace source, public ABI,
wire grammar, formal-model input or finding status. SEC-07/18 remain partial;
the ledger remains **29 corrected / 6 partial / 3 open**. Broader containing/
outer IRQ, guard and implicit-destructor contexts remain qualification work.
Passing runs cannot attribute historical scheduler, metrics, root-close or
mapping failures.
