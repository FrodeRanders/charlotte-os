# Audit follow-up: IPC readiness boundary

Date: 2026-10-25. Reviewed baseline: `dabc20b5`.

This follows the [memory backing consolidation](2026-10-25-refactoring-memory-backing.md).
Endpoint/reply waits, endpoint-close watches, observable bindings and complete
reply-CQ subscriptions now live in
[`ipc/readiness.rs`](../../../crates/catten/src/ipc/readiness.rs), alongside its
waiter-source child and existing fixtures. The IPC core falls from 2456 to
2050 lines; its registry, message/attachment transfer, result observation and
reply/close publication remain in their existing modules.

## Boundary and preserved policy

Observable target bindings and `ReplyCqInterest`'s observer/registration fields
are private to readiness. The pending call still retains that complete owning
interest in its existing admitted record; no second registry or cleanup ticket
is added. Cancellation and result-observation policy are unchanged. External
cancellation fixtures construct observables from the same captured scalar IDs
through field-assembly constructors. These add no allocation, lookup, root,
reservation or registration. Shared endpoint-readiness and authorization
predicates retain their existing IPC-family scope for guarded producers.

The waiter-source child still captures the original retained list, prepares its
owning registration outside IPC, then revalidates source identity and readiness
under the registry before linking. Ready/full/stale preparation stays owned
through post-guard disposal. Source teardown, entry lifetime accounting and
callback notification order are unchanged.

Timed replies retain the same scheduler absolute-deadline loop and distinct
timeout/admission outcomes, without observing results or releasing pending
memory borrows. Untimed receive/reply admission pressure retains its existing
cooperative retry policy; this relocation does not add a missing producer
notification or qualify every outer wait/IRQ context. CQ binding still captures
the caller's original queue and sponsor, prepares outside IPC, revalidates the
pending capability and duplicate binding under IPC, and notifies only after
unlock. Endpoint-close watches still use the complete `EventSubmission`,
revalidate the connection after staging, and install against the exact captured
completion. Readiness grants no result/adoption, replay or physical completion
authority.

Public/kernel entry points, fixture entry points and the debugger-visible
`IPC_WAIT_ADMISSION_RETRIES` symbol remain available through `ipc`. Active
registration cancellation, producer submission fallback and containing
pending-call/source destruction retain their existing policy; the containing
registry fields still serve serialized cross-family transactions. This does
not qualify broader destructor/guard contexts. C08/C18/G1 remains qualification
work; no ABI, configured limit, userspace source or model input changes.

## Verification

The [evidence](evidence/2026-10-25-refactoring-ipc-readiness.json) records frozen
source/artifact hashes, comparisons and native executions. All moved production
wait/watch/CQ/predicate/counter bodies match after family visibility and module
plumbing. Remaining registry, transfer, receive, reply and close code matches
after facade changes. Observable constructors only assemble the same fields.
The waiter-source implementation and its two fixture files move byte for byte.
Cancellation fixture assertions/order match after constructor routing and
explicit imports; the delivery fixture only routes its private binding helper.
These comparisons aid review, not formal refinement.

Initial formatting/strict compilation caught an import before a fixture's
module documentation, two remaining shorthand observable literals against
private fields, and an unused fixture import. The import now follows module
documentation; those literals use constructors and the unused import is removed.
Final strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle
inputs and `--no-default-features --features acpi -- -D warnings`. No mirror
test or alternate implementation is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs pass heap-held receive/reply/full/ready/foreign/closed-source
publication, owning rejection disposal, close-watch cancellation and generation
reuse, and reply-CQ reply/cancel/late binding without result observation. Source
bounds, cancellation/rearm, message/reply/close notifications, loan revocation
and retired sponsorship pass. Scheduled controls pass 64 reply/readiness timeout
cleanups, forced untimed admission recovery with a live loan, exact pending
reply wakes and 64 claim/endpoint-close episodes with exact exit watches. Both
runners finish with `reason=complete`; signed bundles are reused with unchanged
userspace sources. Formatting, whitespace, frozen Rust hashes and local
references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. x86_64 also records **one TimeoutError retry** before its
successful initial metrics request. All nine diagnostics and eventual passing
probe results are preserved separately in the evidence. They are retried
attempts within these runs, not separate clean executions. No controlled
comparison attributes them to this refactoring, host scheduling or historical
failures.

No finding status changes. SEC-07/18 remain partial; the ledger is **29 corrected
/ 6 partial / 3 open**. Historical scheduler, metrics, root-close and mapping
failures remain unattributed. Broader pressure, fairness and outer-context
qualification remain open.
