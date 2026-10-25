# Audit follow-up: CQ delivery boundary

Date: 2026-10-25. Reviewed baseline: `85b8a3a1`.

This follows the [detached lifecycle consolidation](2026-10-25-refactoring-completion-detached.md).
Ring publication, FIFO backlog ownership and refund extraction now live in
[`completion/delivery.rs`](../../../crates/catten/src/completion/delivery.rs),
with the existing queue payload/retirement preparation as its child module.
The completion core falls from 1981 to 1763 lines.

## Ownership boundary

`CqState` keeps its raw ring address, authoritative producer cursor/capacity,
backlog, retained limit, heap backing and original storage charge private to
delivery and preparation. `BacklogEntry` is private. Producers call `post` and
namespace backlog reconciliation calls `flush`; both borrow the same existing
queue and caller-owned `budget::Releases` batch under the existing registry
hold. No new lock, reservation or refund owner is introduced.

Read-only pointer/geometry/backlog inspections replace external field access.
The raw pointer remains a borrow: every dereference still requires its original
mapping/registry/quiescent-consumer contract. Capability retirement calls
`discard_capability(operation, cap)` with the same exact-row predicate, which
never discards a detached owner. Queue replacement counts its retained detached
rows through `detached_backlog_len`; logical namespace-slot reconciliation stays
in the core and supplies no physical-release proof.

The preparation child initializes those private fields and retains complete
partial, published and retired payloads. Its original completion-family scope
is preserved through explicit visibility/reexports; the former standalone
`queue_preparation` path is removed without a compatibility alias. Waiter source
and work/last-seen generations retain their existing family access, so captured
source handling and checked waits remain in the core.

Ring entry writes, volatile consumer-tail sanitization, release publication,
FIFO behavior, one-time detached-node transfer and work-generation updates are
unchanged. Detached charges/nodes still move into the original refund batch;
node disposal and refunds follow local guard exit before notification/watchdog
preparation. Caller guard/IRQ obligations are unchanged.

Full preparation/retirement fallbacks remain retaining owners. The payload's
existing `CqState::Drop` remains active: it closes sources and drains detached
rows during explicit payload destruction. Its relocation/private fields do not
qualify arbitrary containing destructors, outer IRQ masks, allocator/callback
contexts, physical rings or physical backing release. These remain C18/G1 work.

## Verification

The [evidence](evidence/2026-10-25-refactoring-cq-delivery.json) records source
hashes, comparisons and native executions. Moved state/ring/Drop bodies match
the baseline after family visibility, comments and formatting are normalized.
Post/flush bodies match after receiver substitution. Capability-row filtering,
detached-row counting and the read-only inspections preserve their original
expressions. Preparation bodies retain their former scope and logic.

Both preparation fixture bodies move byte for byte. Other native fixtures keep
their assertions and order, changing only private module routing or read-only
inspections. The remaining core matches after extraction, module plumbing,
producer/flush routing and those inspections. Checked waits, source notification
and deadlines are unchanged. These comparisons aid review; they are not
executable-refinement or arbitrary-context proofs. No new mirror tests or
alternate implementation is introduced.

Strict base-kernel Clippy passes for AArch64 and x86_64 with the signed bundle
inputs and `--no-default-features --features acpi -- -D warnings`.

Four-LP native results on the frozen source, with the architectures run
sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs execute full/FIFO backlog flushing while registry/domain/node/heap
guards are held, ordinary/platform refund aggregation, original-account
replacement and both completion producers. Existing CQ controls cover unlocked
list/ring/node rejection, partial and guarded-retaining preparation, heap-held
competing/stale/retired/aliased publication, unchanged physical bytes, complete
backlog/source handback and exact successor fencing. Both finish with
`reason=complete`. Previously rebuilt signed bundles are reused; userspace
sources do not change. Formatting, whitespace and local references pass.

AArch64 logs **four ConnectionResetError retries**: one before successful EOF
recovery, one before abortive-peer recovery, and two before idle-client recovery.
x86 has those **four reset retries plus one TimeoutError retry before its
successful initial metrics request**. All nine diagnostics and eventual passing
probe results are preserved in the evidence. These are retries within the
reported passing runs, not separate clean executions. No controlled comparison
attributes them to this extraction, host scheduling or historical failures.

This changes no configured capacity, ring layout, public ABI, userspace source,
formal-model input or finding status. SEC-07/18 remain partial; the ledger is
**29 corrected / 6 partial / 3 open**. Historical scheduler, metrics, root-close
and mapping failures remain unattributed.
