# Audit follow-up: IPC close-policy boundary

Date: 2026-10-25. Reviewed baseline: `812baf55`.

This follows the [message consolidation](2026-10-25-refactoring-ipc-message.md).
Common capability close dispatch, loan preflight, queued/result cleanup and
serialized namespace draining now live in
[`ipc/close.rs`](../../../crates/catten/src/ipc/close.rs). The IPC core falls
from 1773 to 1288 lines. Its registry, authority lookup, submission/reply entry
points, shared loan-revocation adapter and notification helper remain in place.

## Boundary and preserved policy

`CloseMode::Owned` replaces the former `true` selector. It uses ordinary visible
capability admission, retains the existing captured claim-wait owner and hands
loan cleanup to the existing root-owning endpoint/cancellation operations after
leaving IPC. `CloseMode::Serialized` replaces `false`: cleanup may consume hidden
grants after namespace/claim drain, uses the same serialized loan adapter, and
rejects an unexpected claim with Pending instead of admitting fresh roots or
spinning. Production namespace close still borrows its exact closing owner and
drains per-token owners before reaching this capability-drain phase. Raw boot
fixtures retain their separate serialized adapter.

The enum names execution policy; it supplies no caller authority, root lease,
claim completion or quiescence proof. Only the same two boolean predicates are
translated. No extra mode, fallback or retry is introduced. Root public close
and crate-visible fixture cleanup remain facades; private fixture hooks use
explicit close-module imports instead of root implementation aliases.

Pending/failed loan cleanup still leaves live token, call and capability state
fenced. Unobserved returned capabilities retire; observed results retain their
caller's ownership. Complete reply/capability/endpoint/pending nodes and returned
memory stay in the same local or typed owners until their original post-IPC
release points. Reply-node, metadata, pending-node, result/queued memory and
queue backing disposal still precede the same watch/CQ/receiver notifications.
Original charges return after actual backing/node disposal. Namespace drain
still reuses admitted storage, releases the complete namespace after unlock,
then detaches otherwise unreferenced closed endpoints without a snapshot.

The containing destruction, serialized revocation and active callback contexts
retain their existing qualification boundaries. This relocation and enum do
not make the loose local cleanup state inert, add recovery custody or authorize
physical retry. C08/C10/C18/G1 remains qualification work. Configured limits,
public register/wire ABI, userspace sources and model inputs are unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-ipc-close.json) records frozen
source/artifact hashes, comparisons and native executions. Every moved dispatch,
preflight, namespace-drain, queue/result retirement, release and notification
body matches after family visibility and the explicit two-way mode mapping.
Remaining registry/send/receive/reply/authority code matches after module
facades. Production namespace close only changes its private drain path.
Fixture assertions/order match after private imports and replacing the existing
serialized selector with its named variant. Import changes are reviewed
separately. These comparisons aid review, not formal refinement.

Strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`. No mirror test,
alternate implementation or compatibility mode is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs pass queued/delivered cancellation, competing close/reply/receive,
staged close, preparation rollback, partial failure and abandonment. Bulk
failure retains queued authority and exact closing roots; namespace ownership
and selection/recheck controls preserve pending token/epoch diagnosis and
retained failed roots/backing. Backing retirement covers maximum vectors,
bulk queues, calls/results and post-IPC physical release. Runtime endpoint and
namespace retirement with secondary LPs online, hidden connection/memory
rejection, DMA-loan preservation and scheduled claim-close controls also pass.
Injected bulk-revocation failure diagnostics are preserved separately from
transport retries. Both runners finish with `reason=complete`; signed bundles
are reused with unchanged userspace sources. Formatting, whitespace, frozen
Rust hashes and local references pass.

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
