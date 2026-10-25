# Audit follow-up: completion namespace boundary

Date: 2026-10-25. Reviewed baseline: `43528bd9`.

This follows the [producer consolidation](2026-10-25-refactoring-completion-producers.md).
The complete namespace owner, account construction, authority removal and
post-registry payload disposal now live in
[`completion/namespace.rs`](../../../crates/catten/src/completion/namespace.rs),
alongside its preparation/retirement child. The completion core falls from
1152 to 1010 lines. This groups one lifecycle; it does not reduce its admission
or cleanup obligations.

## Boundary and preserved policy

`Namespace` names the complete owner, formerly `AsCompletions`. Its private
`ManuallyDrop` payload retains all records, detached operations, queues and
original accounts on abandonment. `NamespaceState` names the shared contents
view used by existing guarded cross-family transactions. Those fields retain
completion-family access through the same dereference interface; this is not
full encapsulation of every namespace mutation. The core retains its single
`COMPLETIONS` registry. No new lock, root, registry or owner instance is added.

Account construction remains fallible, with the same capacity clamp and ordered
timer, record, CQ and watch allocations. Preparation still captures the exact
generation before allocation, validates retirement under the existing registry
hold, and publishes only its prepared node. The private mandatory fixture
constructor and public open/close facades use the same implementation; no
compatibility helper remains.

Authority removal is now a method on the complete owner. Its caller still
retains that owner and the registry hold through removal. Explicit retirement
frees the global node after unlock, then disposes event observations, waiter
lists and callbacks, capability records/authority, detached nodes, queues and
finally the remaining accounts in their original order. Captured identities
are not root leases. Physical CQ mapping ownership and consumer quiescence
requirements remain unchanged, including the original `Send`/`Sync` argument.

Namespace/preparation/retirement abandonment remains inert. Payload destruction,
source cancellation and containing rollback still have active disposal paths;
this relocation does not qualify arbitrary outer IRQ/guard contexts or supply
physical completion, custody or retry authority. C18/G1 remains qualification
work. Configured limits, public register/wire ABI and userspace sources are
unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-completion-namespace.json) records
source/artifact hashes, comparisons and native executions. Constructor,
authority-removal, finish and open/close bodies match after role naming, method
receivers and module routing. The complete owner/view/dereference structure
matches after visibility and type naming; the unsafe contract is unchanged.
Preparation matches after its additional module level and factory/method
routing. Remaining core, publication, wait and deadline code matches after
extraction/plumbing. These comparisons aid review, not formal refinement.

The namespace fixture moves byte for byte. Three existing fixture call sites
use the owner method or named fixture constructor; assertions and order remain.
An initial strict compile caught the `Deref` import previously inherited by
capability metadata from the root. That module now imports its own trait.
Final strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle
inputs and `--no-default-features --features acpi -- -D warnings`.
No new mirror test or alternate implementation is introduced.

Four-LP native results on the frozen Rust source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs exercise all five unlocked allocation-stage rejections, 64 heap-held
fresh namespace publications/removals, complete source/timer/backlog/authority
handback, disposal that reenters the registry after unlock, retired-generation
rejection and original-account reuse. Existing capability/detached/CQ metadata,
callback and terminal-disposal controls also pass. The guarded abandonment
fixtures retain their complete owners without resetting counters; this does
not establish aggregate exhaustion or fairness. Both runners finish with
`reason=complete`. Signed bundles are reused with unchanged userspace sources.
Formatting, whitespace, frozen Rust hashes and local references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. x86_64 also records **one TimeoutError retry** before its
successful initial metrics request. All nine diagnostics and eventual passing
probe results are preserved separately in the evidence. They are retried
attempts within these runs, not separate clean executions. No controlled
comparison attributes them to this refactoring, host scheduling or historical
failures.

No model input or finding status changes. SEC-07/18 remain partial; the ledger
is **29 corrected / 6 partial / 3 open**. Historical scheduler, metrics,
root-close and mapping failures remain unattributed.
