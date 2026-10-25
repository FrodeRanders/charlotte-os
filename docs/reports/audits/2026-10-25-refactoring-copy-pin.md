# Audit follow-up: copy-pin lifetime boundary

Date: 2026-10-25. Reviewed baseline: `899e339b`.

This follows the [mapping retirement consolidation](2026-10-25-refactoring-mapping-retirement.md).
Copy-pin ownership, scoped cleanup, admission, snapshot publication and explicit
unpin now live in [`object/copy.rs`](../../../crates/catten/src/memory/object/copy.rs).
The object registry falls from 1158 to 1075 lines. DMA pins and their distinct
hardware-completion context remain in their original family; the shared snapshot
preparation and deferred-backing policy remain unchanged.

## Boundary and preserved policy

`CopyPin` keeps its object identity and snapshot private. `ScopedCopyPin` keeps
its optional complete owner private. Constructors only assemble the same fields
after the existing admission/fence has been established; they do not validate
or increment a pin. Borrowed frame/identity access adds no lookup, allocation,
root, reservation or authority. Scoped inspection borrows its original Option;
taking it consumes that same whole pin and disarms the same active fallback.

Copy admission still checks original ownership, rights, mapping/loan/DMA fences
and pin capacity under memory serialization. Snapshot preparation remains
fallible and pin-free outside the registry. Final publication rechecks the same
generation, object, size and predicates, then fills and increments under the
same guard. Rejection disposes unused preparation after unlock. Captured trusted
classification, snapshot admission and original charges are unchanged.

Ordinary explicit unpin still disposes the snapshot first, retains the armed
pin through the existing observer seam, then decrements its exact fence under
memory serialization. Deferred backing leaves that guard before physical
release. Bare `CopyPin::Drop` remains non-releasing and retains its snapshot and
backing fence; `ScopedCopyPin::Drop` still actively calls ordinary unpin.

Source escrow keeps its separate original path: its already-fenced empty-snapshot
pin moves out of the scoped wrapper during containing transfer cleanup, and the
same locked helper decrements that fence while authority/fence restoration stays
serialized. No full snapshot disposal moves beneath that guard. The original
containing transfer owner still retains metadata, authority and backing until
their existing cleanup points; no raw resource adoption or manual cleanup ladder
is introduced.

These distinct Drop/cleanup contexts remain separately qualified. Moving them
together does not make scoped/containing destruction inert, authorize physical
replay or add recovery custody. C18/G1 remains qualification work. Limits,
register/wire ABI, userspace and model inputs are unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-copy-pin.json) records frozen
source/artifact hashes, comparisons and native executions. Every moved owner,
admission, factory, locked decrement, explicit unpin and Drop body matches after
family scope and module routing. Constructors match former field assembly;
slice/identity/Option methods borrow or take the same fields. Remaining registry,
DMA, mapping, close and backing code matches after extraction/facades. Transfer
and snapshot-fixture code matches after those constructor/accessor changes,
including the original locked source-escrow take/decrement position. These
comparisons aid review, not formal refinement.

The copy fixture moves byte for byte; its aggregate hook retains the original
order. The retired private storage-module alias is removed. Strict base-kernel
Clippy passes for AArch64 and x86_64 with signed bundle inputs and
`--no-default-features --features acpi -- -D warnings`. No mirror test or alternate
implementation is introduced.

Native four-LP runs used fresh storage and the previously rebuilt signed
userspace bundles, sequentially on AArch64 and x86_64 with Intel VT-d. Both
runners finished with `reason: complete`:

| Architecture | Deferred passed / failed / pending | HTTP checks passed | Security publication generations |
| --- | --- | --- | --- |
| AArch64 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

On both architectures the existing copy fixture passed 128 unlocked prefix
rejections, overflow rejection, source-close/write-map/DMA and real ASID-reuse
revalidation, and 64 heap-held snapshot fills/pin publications. It confirmed
post-guard snapshot-before-unpin disposal. Its guarded abandonment control
retained the unused snapshot and real pin/snapshot/data page/original charge
through source-root retirement and ASID reuse. Containing transfer rejection,
hidden move/copy/result use, receive rollback, cancellation and endpoint/root
cleanup controls also passed. These executions preserve the distinctions above;
they do not qualify every outer guard, IRQ or destructor context.

Each architecture recorded four `ConnectionResetError` retry diagnostics: one
after the EOF probe, one after abortive peer closes and two during the idle-client
probe. Every affected probe subsequently passed. The evidence preserves all
eight diagnostics and their eventual results; successful repeats do not
establish a cause. No native run failed in this batch. Historical intermittent
failures remain unattributed.

Formatting, whitespace, frozen Rust hashes and local Markdown targets pass.
The audit ledger remains **29 corrected / 6 partial / 3 open**; SEC-07 and
SEC-18 remain partial. This is a refactoring and encapsulation batch, not a new
cleanup qualification or finding closure.
