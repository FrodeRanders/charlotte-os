# Timer backing lifetime admission

Date: 2026-10-07. SEC-07 continuation after completion backing admission at
`97d63fa`. Timer node and cancellation backing now retain their original
reservation until actual release. General metadata and heap admission remain
partial.

## Finding and correction

Timer admission lived in the event payload. Event destruction could refund it
while a retained cancellation handle still owned its separate allocation.
Additionally, an event's field charge dropped before the Box containing its
queue node was deallocated. Review confirmed these accounting lifetime gaps;
controlled fixtures reproduce retained cancellation storage. No hostile
application exploit or whole-node exhaustion was demonstrated.

One event reservation now has shared lifetime owners across its prepared/queued
node, event and cancellation state. The cancellation allocation uses the existing
charged allocator, retaining original admission through all strong and weak
references. Other allocator clones retain lifetime only; they do not perform
additional allocations. The private charge holder is deallocated before the
original reservation is refunded.

`queue::OwnedNode` carries an owner outside its Box. Ordered destruction frees
the complete node before dropping that owner. Pop moves the event out and frees
the node while its outside owner remains live; the returned event keeps its own
owner through destruction. Rejection retains a preparation owner through
failed node allocation and payload destruction. A still-live cancellation handle
therefore remains charged even if node preparation rejects or the node is
discarded. The uncharged cancellable constructor is removed, including from
kernel fixtures.

Sleep prepares the same owner fallibly before node allocation/parking. Quantum
events remain in their one inline LP slot without anonymous admission. Counts,
platform classification, captured generations, cancellation flags and syscall
failure representations are unchanged. Cancellation backing can occupy admission
after queue removal; queue diagnostics continue to count actual queue membership.
These are fixed allocation quantities, not a general byte budget.

Contracts: [completion timer admission](../../reference/completion-timer-budgets.md)
and [scheduler timer admission](../../reference/scheduler-timer-budgets.md).

## Regression evidence

- A real prepared node enters the owning sorted queue and is popped/discarded.
  Its retained cancellation handle keeps a one-record account occupied.
  Destroying that handle leaves 128 weak aliases and one original weak owner;
  upgrade rejects, but new admission remains rejected until final weak release.
- Queue-node allocation rejection with a retained handle keeps its original
  charge. Dropping the handle first keeps the event charged until rejected
  preparation destroys it. Normal prepared-node discard also retains handle
  admission. Entry IRQ state is preserved across failure.
- Old cancellation weak backing survives root retirement and exact numeric
  ASID reuse. The old and successor accounts are both charged; final old weak
  destruction releases only the old account and original node/ordinary share.
  Cleanup restores exact baseline counters.
- Existing 1,024 real sorted nodes, pop/reuse, iterative filter/destruction,
  inline quantum, counter-only shared pool saturation, platform promotion and
  retirement tests remain. Cancelled-event waiter destruction now uses charged
  cancellation state and checks its independent event account.
- Scheduled tests retain pre-park rejection, syscall ownership, runnable sleep,
  deferred busy-local cancellation, owner-LP publication and 64 sleep plus 64
  watchdog cycles. Scoped security probes retain timer/watch cancellation churn.
- Six host tests rerun the actual charged allocator for allocation-failure
  destruction order, strong/weak lifetime and concurrent allocation/release.
  Node rejection and queue removal are controlled fixtures, not physical OOM,
  panic unwinding or an actual forced remote-LP reclamation race.

## Validation

| Check | Result |
| --- | --- |
| Host allocation-owner regressions | **6 passed, 0 failed**. |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,796 requests. |

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance timer-backing-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance timer-backing-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance timer-backing-arm-20261007 --fresh-storage --timeout 180
```

Run `cargo +nightly-2026-07-27 test --locked --manifest-path
/path/to/charlotte-os/crates/charlotte-lifecycle/Cargo.toml --test
charged_allocator` from outside the repository's kernel build-std configuration,
or use the host-test runner. Guests rebuild kernels/check assembly sections and
reuse validated embedded services for this kernel-only change. Arm uses the
required local port-binding permission.

## Remaining scope

Independent waiter/observer-list backing, arbitrary callback captures, sponsor
allocations, other weak-only Arc storage and general registry/heap metadata remain
separate work. This change provides no principal aggregates, fairness guarantee
or hostile-workload containment certification; SEC-07 remains partial.

Queue processing still destroys allocations under its LP-local borrow; broader
allocation/destruction under subsystem guards remains part of SEC-18, alongside
physical-platform quiescence, broader reset support and abandoned-owner recovery.
Authentication, production provisioning and security-time findings are unchanged.
