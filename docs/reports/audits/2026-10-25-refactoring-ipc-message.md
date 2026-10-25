# Audit follow-up: IPC message delivery boundary

Date: 2026-10-25. Reviewed baseline: `d322ec0e`.

This follows the [IPC readiness consolidation](2026-10-25-refactoring-ipc-readiness.md).
Queue publication, the complete notification receipt and scalar/vector receive
transactions now live in
[`ipc/message.rs`](../../../crates/catten/src/ipc/message.rs). The IPC core
falls from 2050 to 1773 lines. Its registry, call/attachment preparation,
reply/result observation and close control remain in their existing modules.

## Boundary and preserved policy

`Delivery` keeps its receiver-notification batch and optional CQ wake private.
Callers consume the same complete receipt through `notify()` after their
existing guard and cleanup boundary. This replaces the free `deliver` helper;
no compatibility helper remains. CQ wake still precedes receiver notification.
The endpoint-close and cancellation adapters assemble the same receipt through
a constructor with their original conditional expressions and evaluation order.
It only assembles fields: no allocation, lookup, guard, root, reservation or
owner instance is added.

Enqueue still captures sender generation/principal/roles at publication, checks
the same endpoint state/capacity, installs the same queued attachments, updates
the high-water mark, and coalesces CQ wakes on the same empty-to-nonempty edge.
The receipt retains its existing numeric ASID/CQ wake destination; this batch
does not replace it with a captured queue interest or qualify additional
notification contexts.

`ReceiveResources` and `PreparedReceive` are private to the message module.
Scalar/vector receive share the existing complete transaction through the
resources' associated receive method. The mutable IPC borrow still excludes
concurrent delivery while speculative reply authority exists. Result-page
write precedes dequeue; invalid/read-only/insufficient storage leaves the
message, token and hidden attachments queued. Ordinary rejection removes only
the speculative reply capability through the existing borrowed retirement
list. Commit dequeues once and publishes connection/memory visibility under
the same IPC hold. Retired metadata and unused authority preparation finish
after guard exit, in their original order.

The notification receipt's field destruction, `PreparedReceive::Drop` and
containing transfer/receive disposal retain their existing policy. Their
relocation does not qualify arbitrary outer IRQ/guard/destructor contexts or
authorize physical cleanup, replay or recovery custody. C18/G1 remains
qualification work. Root scalar/vector receive entry points, attachment limits,
register/wire envelope, userspace sources and model inputs are unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-ipc-message.json) records frozen
source/artifact hashes, comparisons and native executions. Moved enqueue,
receive, speculative authority, result-page write, commit and Drop code matches
after family scope, module facades and receive-method association. The consuming
notification method matches the former free helper. Its constructor matches
former field assembly; both producer adapters retain their expressions and
argument order. Remaining registry/send/reply/readiness/close code and all
notification callers match after that routing. Fixture assertions/order remain
unchanged; no fixture implementation is duplicated or replaced. These
comparisons aid review, not formal refinement.

Strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`. No mirror test or
alternate implementation is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs pass hidden connection/memory rejection, receive rollback, vector
delivery, cancellation, queue/result cleanup, repeated observation and delivered
ownership/attenuation. All four vector transfer modes retain their private
cancellation and loan-revocation controls. The 64 IPC/heap-held prepared scalar
publications pass enqueue/reply/observation and original count reconciliation;
paired reply-storage controls remain enabled. Existing waiter preparation and
reply-CQ controls also pass. Both runners finish with `reason=complete`; signed
bundles are reused with unchanged userspace sources. Formatting, whitespace,
frozen Rust hashes and local references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. All eight diagnostics and eventual passing probe results
are preserved separately in the evidence. They are retried attempts within
these runs, not separate clean executions. No controlled comparison attributes
them to this refactoring, host scheduling or historical failures.

No finding status changes. SEC-07/18 remain partial; the ledger is **29 corrected
/ 6 partial / 3 open**. Historical scheduler, metrics, root-close and mapping
failures remain unattributed. Broader pressure, fairness and outer-context
qualification remain open.
