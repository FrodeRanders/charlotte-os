# Audit follow-up: completion record boundary

Date: 2026-10-25. Reviewed baseline: `36e44506`.

This follows the [CQ delivery consolidation](2026-10-25-refactoring-cq-delivery.md).
Capability-backed record state, buffer and subscription custody now live in
[`completion/record.rs`](../../../crates/catten/src/completion/record.rs).
The completion core falls from 1763 to 1449 lines. Registry identity, CQ
publication, producer adapters and checked waits remain in the core.

## Ownership boundary

`CompletionInner`, lifecycle `OpState`, buffer, callbacks and event/worker/timer
subscription fields are private to the record owner. Allocation, capability
submission and callbacks are child modules, so their existing atomic state
borrows need no exported mutex guard or buffer-pointer inspection API.
Readiness waiter sources retain their existing completion-family access for
namespace close and diagnostics. No new account, lock, record or charge is added.

The record returns a complete `TerminalTransition`; only its effective result
is visible to registry publication. Its detached event/worker subscriptions
stay private until the complete receipt is disposed after CQ publication and
local registry unlock. `ObservationInstallation` likewise keeps replaced/
rejected custody through post-guard finish. Buffer observation, cancellation
and terminal idempotency are unchanged.

The public cancellation transaction moves with the record, retaining its
exact-object lookup, timer/event detachment, post-inner disposal and checked
registered completion. The worker installation adapter assembles the same
`EventObservation` from its existing observer/registration before entering the
same setter/guard. No ownership is reconstructed from scalar identifiers.

Public `Completion`, cancellation and callback entry points are reexported
through the existing completion facade. Private submission factories use
explicit `submit_with_ids` / `submit_timer_with_ids` names. Captured callback
preparation remains owner-local; unused root-only helper/type aliases are
removed. Native callback boot entry points retain their crate facade.
No compatibility implementation or alternate allocator is kept.

The initial integration compile caught factory helper names shadowing public
`submit` in moved fixtures (`E0061`: four arguments expected, three supplied),
plus unused root-only imports. Explicit factory names and removal of unused
aliases correct that routing. These are resolved draft compiler diagnostics;
native executions use the final compiled source.

Record payload `Drop`, timer cancellation, terminal/installation disposal and
the containing `EventSubmission::Drop` remain active under their existing
policies. Unpublished preparation fallbacks still retain their complete fields.
Private state and module relocation do not qualify arbitrary containing
destructors, outer IRQ/guard contexts or physical quiescence. C18/G1 remains work.

## Verification

The [evidence](evidence/2026-10-25-refactoring-completion-record.json) records
source/artifact hashes, source comparisons and native executions. All moved
state/buffer/subscription, observation, cancellation and destruction bodies
match after family visibility, constructor routing and formatting are normalized.
The worker adapter matches the former owner assembly exactly. Allocation,
submission and callback child implementations retain their original bodies.
Remaining registry/wait/deadline code matches after module/worker routing.
These comparisons aid review, not executable-refinement proofs.

Seven native fixture bodies move byte for byte: allocation, three submission
fixtures, two callback fixtures and terminal disposal. The callback boot driver
changes two private module routes; its assertions and execution order remain
unchanged. No new mirror test or alternate implementation is introduced.

Strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs execute the moved allocation, initial/capability submission, callback
preparation and terminal-disposal controls. They retain constructor-prefix
rejection, original buffer identity/charge checks, heap-held publication,
reentrant post-publication disposal, cancellation parity, duplicate completion,
exact queue/root replacement and guarded abandonment coverage. Both runners
finish with `reason=complete`. Signed bundles are reused with unchanged
userspace sources. Formatting, whitespace and local references pass.

AArch64 records **four ConnectionResetError retries**: one before successful EOF
recovery, one before abortive-peer recovery and two before idle-client recovery.
x86 records those **four reset retries plus one TimeoutError retry before its
successful initial metrics request**. All nine diagnostics and eventual passing
probe results are preserved in the evidence. They are retried attempts within
the reported runs, not separate clean executions. No controlled comparison
attributes them to this refactoring, host scheduling or historical failures.

This changes no configured capacity, public register/wire ABI, userspace source,
formal-model input or finding status. SEC-07/18 remain partial; the ledger is
**29 corrected / 6 partial / 3 open**. Historical scheduler, metrics, root-close
and mapping failures remain unattributed.
