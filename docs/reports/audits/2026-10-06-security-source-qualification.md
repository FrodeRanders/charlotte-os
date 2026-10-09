# Returned-memory source qualification — 2026-10-06

Baseline: `026b87bb`, following
[connection delivery](2026-10-06-security-connection-delivery.md).
This is an SEC-18 follow-up. It identifies no new vulnerability and does not
complete SEC-18.

## Remove global qualification work under IPC

Split-phase returned-memory preparation previously visited the global pending
call and endpoint registries, inspecting the serving domain's unobserved results
and queued attachment vectors to decide whether a source was reclaimable. This
held the global IRQ-masking IPC write guard and depended on records unrelated to
the selected source. Current admission budgets bound the registries; the concern
is avoidable nonlocal work, not an unbounded allocation or a measured timeout.

SEC-29 already makes queued/unobserved owning memory inaccessible through its
payload's `delivery_pending` state. `PreparedTransfer` uses that ordinary lookup,
then validates ownership, transfer rights, mapping/pin state and exact live
namespace identities before source escrow. The queue/result scan is therefore
removed. Qualifying a returned source no longer traverses unrelated IPC records.
No timing benchmark or complete interrupt-latency guarantee is claimed.

Hidden source rejection now reports `MemoryTransferFailed` from transfer
preparation instead of the removed scan's `Pending`. It occurs before preparing
or mutating input loans. Unpublished output admission is refunded and admitted
root leases complete on rejection. Internal interfaces have no compatibility
requirement; the old qualification path is not retained as a fallback.

Delivered/observed owning memory keeps its existing source escrow, backing pin,
joint publication and root-retention transaction. Direct source close still
waits outside the registry/IPC until transfer completion. A transferred source
cannot be reclaimed by an earlier queue/result: receive removes that queue
receipt, and observation makes result cleanup relinquish ownership. Those
transitions and source qualification remain monotonic.

## Loan review and regression coverage

Queued loans retain their existing call-scoped revocation contract. Their
borrower has mapping rights but cannot move the backing; copy preparation also
requires object ownership. Cancellation/reply owns exact roots and revocation
receipts. Rejected DMA/physical cleanup keeps the call/backing live rather than
publishing terminal success. This review does not establish a new unsafe loan
ownership path or remove its physical-quiescence obligations.

Cancellation fixtures now check that both queued and delivered read/write loans
reject copy and move preparation before their existing mapped-cleanup tests.
A returned-memory fixture rejects a visible borrowed source while two input
loans remain mapped, lender restrictions remain active, destination admission
is unchanged and the pending result remains empty. Ordinary reply then revokes
both loans successfully. Existing hidden-source fixtures verify unchanged
admission and retry after receive/poll; their expected error follows transfer
preparation. Existing source-close, staged root close, publication rollback,
partial failure and abandonment fixtures remain enabled.

No additional deliberately quarantined roots or pages are introduced. These
tests exercise kernel ABI boundaries and existing guest suites; they do not
inject real DMA timeout or rejected hardware IPI delivery.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including slot/scratch ownership, runtime/services/protocol, signing and boot-result suites. |
| x86/AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Four-LP x86 guest | **15 passed, 0 failed, 0 pending**, including strengthened loan/source fixtures and existing EL0 IPC tests. |
| AArch64 security guest | **19 passed, 0 failed, 0 pending**. Both probes reported `0xffff`, publication generations reached 1 and 2; cancellation traffic retired after 4,720 requests. TCP/IP clock/cycle progress held while pressure clients retired after 1,667 and 1,651 requests. |

Successful guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance source-qualification-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18499 \
CATTEN_DEPLOY_HOST_PORT=17799 scripts/run-aarch64.sh --security-test \
  --instance source-qualification-20261006 --fresh-storage --timeout 180
```

Runners rebuilt both kernels and checked native assembly permissions. Existing
validated signed service bundles were reused; this batch changes kernel code
only. The initial ARM launch could not bind host-forwarding ports in the sandbox;
the approved isolated retry passed. No unrelated VM or storage was stopped/reset.

## Remaining scope

SEC-18 remains partial: allocator release latency under IPC, recoverable
shootdown failure, full CPU/DMA quiescence, physical-device reset and recovery
for abandoned roots remain open. Final root shutdown still depends on
caller-established thread quiescence and recipient progress.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
