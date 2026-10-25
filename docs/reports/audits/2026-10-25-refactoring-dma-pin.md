# Audit follow-up: DMA-pin lifetime boundary

Date: 2026-10-25. Reviewed baseline: `a2712d8d`.

This follows the [copy-pin consolidation](2026-10-25-refactoring-copy-pin.md).
DMA pin ownership, admission, snapshot publication and explicit unpin now live
in [`object/dma.rs`](../../../crates/catten/src/memory/object/dma.rs).
The object registry falls from 1075 to 1015 lines. The private storage-module
alias is removed; shared snapshot preparation and deferred-backing policy remain
in their existing families.

## Boundary and preserved policy

`DmaPin` keeps its object identity, snapshot and exclusivity private to the DMA
module. Existing backend callers retain its complete owner and borrow the same
frame slice or object identity through unchanged methods. The access descriptor
and snapshot factory are now private. The aggregate fixture hook and crate-level
pin/unpin entry points keep their existing contracts.

Admission still checks direction, rights, loan/retirement state, CPU mappings,
copy/DMA exclusion and counter overflow under memory serialization. The snapshot
factory runs fallibly outside that guard and before any source pin or frame
identity crosses the preparation interval. Final admission rechecks the captured
namespace generation, object, length and predicates, fills the snapshot and
publishes shared/exclusive counters under the same hold. Rejected unused storage
is disposed after unlock with its original classification and charge.

Explicit unpin consumes the original pin, disposes its snapshot first and keeps
the pin armed through the existing observer seam. Shared and, when applicable,
exclusive counters decrement under the original memory guard. Existing deferred
backing release follows guard exit. Bare fallback remains inert and retains the
snapshot, counters, backing and original charge without allocator/registry work.

Only unused preparation or the original backend's confirmed hardware cleanup
permits unpin. `DmaOperation`, `PendingPin`, maintenance, engine/domain claims and
backend sources are unchanged. No root lease, registry, allocation, reservation,
retry owner, readiness or hardware proof is added. Containing rollback and outer
IRQ/guard/destructor contexts remain separate qualification work under C14/C15 and G1/G4.
Limits, public/register/wire ABI, userspace and model inputs are unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-dma-pin.json) records frozen
source/artifact hashes, comparisons and native executions. Token comparisons
preserve strings and normalize only whitespace/comments/trailing commas, module
routing and narrowed visibility. Every moved owner, admission, factory,
publication, unpin and Drop body matches. Remaining registry, copy, mapping,
close and backing code matches after extraction/facades. These comparisons aid
review, not formal refinement.

The DMA fixture moves byte for byte and runs in its original aggregate position.
The initial strict checks on both architectures rejected the revocation fixture's
direct `pin.frames[0]` access (`E0616` at `revocation/tests.rs:154`). It now borrows
the same frame through `pin.frames()[0]`; comparison confirms no other fixture
change. Final strict base-kernel Clippy passes for AArch64 and x86_64 with signed
bundle inputs and `--no-default-features --features acpi -- -D warnings`.
No mirror test or alternate implementation is introduced.

Native four-LP runs used fresh storage and the previously rebuilt signed
userspace bundles, sequentially on AArch64 and x86_64 with Intel VT-d. Both
runners finished with `reason: complete`:

| Architecture | Deferred passed / failed / pending | HTTP checks passed | Security publication generations |
| --- | --- | --- | --- |
| AArch64 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

On each architecture the existing DMA fixture passed 128 unlocked shared/
exclusive prefix rejections, invalid-direction/overflow rejection, source-close/
CPU-map/DMA/copy/loan and real ASID-reuse revalidation, and 64 heap-held fills
with dual-counter publication. It confirmed snapshot disposal before unpin.
Its guarded abandonment control retained the exclusive pin, snapshot, data page
and original charge through source-root retirement and ASID reuse. These
serialized pin controls make no hardware claim and retain no root lease.

The existing coherent CPU-remap fence controls passed for three directions,
two shared pins and last-unpin reuse with original-charge cleanup. Copy snapshot,
memory transfer and IPC delivery/cancellation controls also passed. These runs
exercise the unchanged containing paths; they do not qualify every outer guard,
IRQ, destructor or uncertain hardware-completion context.

Each architecture recorded four `ConnectionResetError` retry diagnostics: one
after the EOF probe, one after abortive peer closes and two during the idle-client
probe. Every affected probe subsequently passed. The evidence preserves all
eight diagnostics and their eventual results; passing repeats do not establish
a cause. No native run failed in this batch. Historical intermittent failures
remain unattributed.

Formatting, whitespace, frozen Rust hashes and local Markdown targets pass.
The audit ledger remains **29 corrected / 6 partial / 3 open**; SEC-07 and
SEC-18 remain partial. This refactoring narrows the ownership boundary without
claiming a new cleanup qualification or finding closure.
