# Audit follow-up: IPC transaction boundary

Date: 2026-10-25. Reviewed baseline: `1c37593b`.

This follows the [admission/memory consolidation](2026-10-25-refactoring-consolidation.md).
Call/connection preparation, call publication, scalar reply publication and
post-IPC retirement now live together in
[`ipc/transaction.rs`](../../../crates/catten/src/ipc/transaction.rs).
The IPC core falls from 3020 to 2456 lines. Registry data, queue/receive policy,
waiters and public entry points remain in the core; claimed loan replies keep
their existing root-owning operation in `reply.rs`.

## Ownership and interfaces

- `PreparedCall` retains its exact captured caller, root, authority, pending and
  reply storage, record charges, staged transfers and optional delegated
  connection. Constructors, atomic commit and explicit finish move together.
- `PreparedCall::stage` replaces `IpcRegistry::stage_call`. It runs the same
  revalidation under the caller's existing IPC write guard; all 17 production/
  native-fixture sites migrate. Captured identity/root and incomplete factory
  fields are private to the owner module. No compatibility adapter remains.
- `ReplyCompletion::publish` replaces the free scalar reply helper. Its returned
  memory/connection, detached reply node, retired authority and notifications
  stay private to the completion owner through explicit post-guard finish.
  Source validation, claim fences, loan handling and publication order are
  unchanged. Scalar front ends still select the existing claimed path for loans.
- `PreparedConnection` keeps target, endpoint, rights and charge private; its
  authority remains available only within the existing IPC family for joint
  publication. Storage aliases and retirement-list operations keep that same
  private family scope. No public kernel/runtime API or wire grammar changes.
- Capability and endpoint preparation import `ManuallyDrop` explicitly instead
  of obtaining it implicitly from their parent. The first compilation identified
  those two former import dependencies; both architecture checks pass after the
  explicit imports are installed.

Fallback remains inert for these complete preparation/retirement owners.
Ordinary finish retains its existing ordering: unused connection/authority and
attachments, pending/reply storage and original charges, then the call root;
scalar reply disposes its detached token before notifying. The extraction adds
no physical replay, retry, root/account or cleanup authority. Broader outer
IRQ/guard/implicit-destructor contexts remain separately qualified work.

## Verification

The [evidence](evidence/2026-10-25-refactoring-ipc-transactions.json) records
source hashes, execution results and native diagnostics. Source comparison
confirms constructor/publication/finish bodies match the baseline after
normalizing IPC-family visibility, formatting and optional trailing commas.
The moved stage and scalar-reply bodies separately match after receiver-name
substitution. The other IPC files differ only in staging calls and the two
explicit imports. These comparisons are review aids, not executable-refinement
or arbitrary-context proofs.

Existing native controls exercise partial constructor and attachment rejection,
heap-held publication, cancellation/drop, returned capabilities, hidden delivery,
original charge lifetimes and captured-generation reuse. No new mirror test or
alternate implementation is introduced. Strict base-kernel Clippy passes for
AArch64 and x86_64 with the signed bundle inputs and
`--no-default-features --features acpi -- -D warnings`.

Four-LP native results on the frozen source:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs include 320 call-stage and 64 authority-factory rejections, 64
heap-held scalar call commits and revalidations, pending/reply storage rejection
and abandonment, paired retirement, original-charge reconciliation and
captured-root reuse. Scalar, moved-memory, loan and returned-connection callers
retain their existing native controls. Both runners finish with `reason=complete`.
Previously rebuilt signed service bundles are reused; userspace sources do not
change. Workspace formatting, whitespace and local references pass.

Host probes retain **four ConnectionResetError retries on AArch64**: one each
before successful EOF and abortive-peer recovery checks, then two before the
successful idle-client recovery check. x86 has those **four reset retries plus
one TimeoutError retry before its successful initial metrics request**. These
are retried probe attempts within the passing runs, not separate clean runs;
their exact diagnostics and eventual probe results are in the evidence. The
native invocations overlap locally. No controlled comparison attributes the
retries to this extraction, host contention or any historical failure.

No configured capacity, userspace artifact source, ABI, formal-model input or
finding status changes. SEC-07/18 remain partial; the ledger remains
**29 corrected / 6 partial / 3 open**. Passing executions do not attribute the
historical scheduler, metrics, root-close or mapping failures.
