# Audit follow-up: completion producer boundary

Date: 2026-10-25. Reviewed baseline: `bf7e1367`.

This follows the [record consolidation](2026-10-25-refactoring-completion-record.md).
`EventSubmission`, worker/exit/timer observers and their installation,
publication/commit and rollback paths now live in
[`completion/producers.rs`](../../../crates/catten/src/completion/producers.rs).
The completion core falls from 1449 to 1152 lines.

## Boundary and preserved policy

Producer binding fields and the timer cancellation handle are private. Record
timer preparation uses a constructor with its original ASID, cap, result and
typed weak completion. Both timer paths construct cancellation custody from
the same existing owned handle. Constructors only assemble fields: no lookup,
allocation, reservation, guard, clock read or ownership re-adoption is added.
A completion-family identifier accessor returns borrowed metadata, not a grant.

`EventSubmission` still stages its captured record and watch charge, fences
installation by exact pointer identity under the namespace guard, and disposes
rejected/replaced installation custody after that guard. Commit consumes the
same owner. Active fallback retires only the captured object, leaves registry
serialization before subscription/metadata disposal, and cannot revoke a reused
successor. The explicit abort path moves with the same owner policy.

Worker publication still installs the exit registration before thread admission.
Worker cancellation retains producer-lifetime observation until actual exit;
external joins keep their distinct local cancellation policy. EL0 exit watches
still retain their exact caller root and generation-scoped authorization;
trusted cross-domain observation remains separate. Timer completion still
upgrades the captured weak record before checked registered publication.

Root public/kernel entry points and crate-visible fixture modules remain
facades over those same implementations. Private timer-factory test callers
use the producer path; no former private implementation is retained as a
compatibility adapter. The raw installation hook retains only its former
completion-family scope for the existing serialized controls.

Timer cancellation and `EventSubmission::Drop` remain active. Their relocation
does not qualify broader containing/outer IRQ or guard contexts, authorize
physical cleanup or establish worker/thread quiescence. Timer charge release
still follows actual queue-node/cancellation ownership, including remote LP
retention. C18/G1 remains qualification work.

## Verification

The [evidence](evidence/2026-10-25-refactoring-completion-producers.json) records
source/artifact hashes, comparisons and native executions. Moved installation,
rollback, observer, timer/exit/worker and Drop bodies match after visibility and
formatting normalization. Constructor bodies match the former field assembly;
the identifier accessor reads the same field. Remaining registry, CQ, record
publication, wait and deadline code matches after module plumbing.

Exit and timer fixture bodies move byte for byte. Other caller/fixture changes
only route the private factory, typed constructors and identifier inspection;
assertions and order remain. The integration compiler rejected one fixture's
direct `observer.cap` access (`E0616`); it now uses the read-only accessor.
Final strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle
inputs and `--no-default-features --features acpi -- -D warnings`.
No new mirror test or alternate implementation is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs exercise event staging and rollback, exact-object installation,
already-gone/stale/foreign thread targets, reentrant cancellation/cleanup,
fast-worker publication and actual producer-exit completion. Timer-observer
allocation rejection preserves original record/event/list/capability counts
and retained weak-reference charges. Terminal-disposal and detached timer
controls remain enabled. Both runners finish with `reason=complete`.
Signed bundles are reused with unchanged userspace sources. Formatting,
whitespace and local references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. All eight diagnostics and eventual passing probe results
are preserved in the evidence. They are retried attempts within the reported
runs, not separate clean executions. No controlled comparison attributes them
to this refactoring, host scheduling or historical failures.

No configured capacity, public register/wire ABI, userspace source, model input
or finding status changes. SEC-07/18 remain partial; the ledger is
**29 corrected / 6 partial / 3 open**. Historical scheduler, metrics, root-close
and mapping failures remain unattributed.
