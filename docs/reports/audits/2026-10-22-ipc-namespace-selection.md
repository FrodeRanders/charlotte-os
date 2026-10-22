# IPC namespace selection recheck

Date: 22 October 2026. Reviewed baseline: `be7a8912`.

A deterministic kernel control reproduces stale Pending during namespace
cleanup. The selected token can disappear in the unlocked interval before
cancellation preparation. That preparation returns Pending for a missing token,
and the old drain returned it without checking whether work remained. This
could turn observed peer completion into a supervisor timeout at its deadline.
It is a separate source-confirmed defect; no causal link to the earlier live
`flood.stop()` reclamation timeout is established. The numbered ledger remains
29 corrected, six partial and three open; production trust stays disabled.

## Recheck through the existing owner

After rejected cancellation preparation, namespace cleanup now rechecks the
exact selected token under IPC. Token IDs never reuse; their allocator checks
overflow. Confirmed absence permits a rescan through the same borrowed
`ClosingAddressSpace`. Every still-present token returns Pending, including a
live or changed-epoch claim or another preparation/admission rejection. No claim
is cleared and no accepted cancellation or physical operation is submitted again.

This is a progress recheck, not a wait/poll loop. Thread quiescence and retired
namespace/endpoint admission prevent fresh work for this root; each extra rescan
follows actual removal of its selected token. A still-present busy selection
returns immediately. The original root/generation, namespace nodes, authority,
backing and charges remain with their existing owners. Missing one token does
not prove namespace completion, root quiescence, physical release or a refund.
Ordinary authority drain, memory cleanup, zero leases, sealing and invalidation
still provide their own proofs before root release.

The production wrapper supplies an empty pre-preparation hook. Serialized kernel
fixtures use it to execute a real competing peer close after token selection,
with IPC/lifecycle/table guards released. It exposes no userspace control or
alternate cleanup policy. The correction adds no root, pin, registry, family,
custody owner, callback in Drop or IRQ change. C02 records these boundaries.
No deadline is widened or restarted.

## Reproduction and ownership controls

The [version-controlled evidence](evidence/2026-10-22-ipc-namespace-selection.json)
keeps the failed old-path run separate from corrected runs, with source/artifact
hashes and selected diagnostics.

The old x86 path fails the new control at
`ipc/cancellation/tests.rs:1061` with
`removed selected token returned stale Pending`. Its real peer endpoint close
has already consumed the selected queued token. This reproduces the stale
namespace result, without injecting a fabricated completion or waiting for a
timeout. The intermediate tree was not committed; its exact kernel/image and
the relevant pre-fix branch are recorded in evidence.

The corrected controls cover both queued and delivered moved memory. A queued
peer endpoint close releases that backing before namespace completion. A
delivered peer reply close leaves the memory with its server owner; it remains
charged to the original caller even after caller-root retirement, and only
actual server-side memory close ends that charge. In each case the removed
selection is handled within the same drain call, without another outer poll.

A further control removes the first delivered token while a different token
has an owned cancellation claim. The drain selects both in order, returns
Pending for the second token at epoch 1, retains the exact closing root and
leaves its result unobserved. Only that existing claim owner's completion
permits further close. Existing abandonment, failure, loan/mapping and retained
charge controls remain in the same boot suite.

| Run | Result | Observation |
|---|---|---|
| Instrumented x86 baseline | 17 passed / 0 failed / 0 pending | Final reclamation completed after 17,132 cancellation requests; historical timeout did not recur |
| Old-path x86 regression | Failed at the new control | Real peer close removed the selected queued token; drain returned stale Pending |
| Corrected x86 security | 17 / 0 / 0 | Both recheck controls; final reclamation after 7,968 requests |
| Corrected AArch64 security | 19 / 0 / 0 | Both recheck controls; final reclamation after 12,156 requests |

Strict kernel Clippy passes on both targets. The signed service bundle is
unchanged; only kernel, fixture, model and living documentation sources change.
The manual PDF remains unchanged.

## Finite model

`CharlotteNamespaceDrain` checks one poll of an already-fenced namespace with
two nonreused tokens. Peer close/claim completion can remove a selection before
preparation. Rejected preparation rechecks absence before rescan; a live
selection returns Pending. Namespace completion requires all represented tokens
to be drained. The safe configuration explores 58 distinct states at depth 6
(71 generated). Both negative controls must fail: restoring stale Pending
violates `NoStalePending`, and skipping live work violates
`CompleteOnlyDrained`.

Peer completion assumes its own genuine cleanup proof. The enclosing root,
loan/backing/charge ownership remains a separate input, as do native allocator
failure, claim epoch transitions, later wakes/reentry and elapsed deadlines.
The projection neither proves eventual reclamation nor identifies the cause of
the historical timeout. The complete suite passes 42 safe configurations and
120 expected counterexamples across 39 modules, including required positive
action coverage and expected-failure reasons.

## Remaining investigation

The preceding instrumented baseline boot passes again, so the historical live
reclamation timeout still has no causal capture. This correction removes one
stale-result boundary without resolving every possible root lease, live or
abandoned claim, memory blocker or scheduling delay. A genuine pending owner
must remain retained through timeout; neither this rescan nor an error grants
authority to retry an abandoned close or release uncertain backing. SEC-06/07/18
remain partial.

The synchronous supervisor helper still uses bounded timer polling for Pending
root cleanup. This correction adds no root-wait readiness subscription; that
notification/progress qualification remains separate.
