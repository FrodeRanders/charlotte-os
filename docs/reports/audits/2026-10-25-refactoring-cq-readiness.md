# Audit follow-up: CQ readiness boundary

Date: 2026-10-25. Reviewed baseline: `acafa0e7`.

This follows the [namespace consolidation](2026-10-25-refactoring-completion-namespace.md).
Captured queue interests, waiter registration, checked waits, explicit wakes
and detached source notification now live in
[`completion/delivery/readiness.rs`](../../../crates/catten/src/completion/delivery/readiness.rs),
beneath the queue payload owner. The completion core falls from 1010 to 671
lines; it retains the shared registry, namespace slot reconciliation and
capability operations/waits.

## Boundary and preserved policy

`CqState`'s work generation, consumed cursor and admitted waiter source are now
private to delivery and its children. Producers call `advance_work` at the same
point under their existing registry hold; it performs the original wrapping
increment and returns that value for the original trace. Read-only accessors
return the same generation or borrow the original admitted list. Source clones
remain at their original positions, including detached-target capture and
terminal notification. No lock, clock read, account, allocation or owner
instance is added. Ring/FIFO/backing and refund policy are unchanged.

The captured `CqInterest` binding is private to readiness. Existing
completion-family fixtures borrow its list through an accessor; they cannot
replace the binding. Waiter preparation still occurs outside the registry,
then publication matches both the prepared source and captured interest before
readiness recheck/linking. Ready/full/stale rejection retains preparation until
explicit post-guard disposal.

Checked waiting still flushes backlog with its original refund owner, retains
the captured source and delegates event-only or finite waiting to the same
scheduler loop. The finite path keeps its original absolute deadline. Final
registry revalidation checks exact list identity before consuming a generation;
queue replacement, deadline and admission rejection keep distinct results.
The queue-wide consumed cursor remains shared; this does not create per-waiter
progress guarantees or remove the documented Sitas backstop.

Explicit wake preparation still advances work and detaches notifications under
the registry without invoking callbacks. Captured-interest wakes reject a
successor at the same numeric ASID/CQ. Producers finish refunds and terminal
disposal before signalling the exact source; callbacks run after local guards
leave. Root kernel/public entry points remain reexported with the same ABI.
The same admitted source retains no ring backing, root lease, result-observation
or physical quiescence authority. Wider IRQ/guard/destructor qualification
remains C18/G1 work.

## Verification

The [evidence](evidence/2026-10-25-refactoring-cq-readiness.json) records frozen
source/artifact hashes, source comparisons and native executions. Moved
readiness, wake, waiter-publication, checked-wait and integrated device-fixture
bodies match after module routing. The separate waiter fixture moves byte for
byte. Existing delivery/storage/flush/Drop code matches after field-scope and
facade changes. Producer and external fixture comparisons normalize only the
new pure increment/accessor calls; assertions, tracing and order remain.
The new increment uses the same wrapping arithmetic and accessors borrow/read
the original fields. These comparisons aid review, not formal refinement.

Strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`. No mirror test,
alternate wait implementation or compatibility path is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs pass the heap-held waiter success/full/ready/stale-source controls,
retained preparation rejection and post-guard cancellation, captured wake
fencing across numeric reuse, and deferred IRQ callbacks reentering device/CQ
registries. Scheduled controls pass completion/explicit/per-queue/endpoint
wakes, reply-CQ binding, and 64 completion/CQ timeout cleanups. Terminal disposal
and queue metadata replacement controls remain enabled. Both runners finish
with `reason=complete`; signed bundles are reused with unchanged userspace.
Formatting, whitespace, frozen Rust hashes and local references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. Each also records **one TimeoutError retry** before its
successful initial metrics request. All ten diagnostics and eventual passing
probe results are preserved separately in the evidence. They are retried
attempts within these runs, not separate clean executions. No controlled
comparison attributes them to this refactoring, host scheduling or historical
failures.

No model input, configured limit, public register/wire ABI or finding status
changes. SEC-07/18 remain partial; the ledger is **29 corrected / 6 partial /
3 open**. Historical scheduler, metrics, root-close and mapping failures remain
unattributed. Broader pressure, fairness and outer-context qualification remain
open.
