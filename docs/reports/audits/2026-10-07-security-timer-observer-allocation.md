# Completion timer observer allocation

Date: 2026-10-07. SEC-07/SEC-18 continuation after provisional kernel-frame
abandonment at `3b878bb4`. This fixes two producer-observer allocation boundaries;
general heap admission and registry allocation remain partial.

## Finding and correction

`completion::submit_timer` and `submit_detached_timer` prepared their charged
timer event, cancellation state and queue node fallibly, then used `Arc::new`
for the producer observer. Capability timers also prepared a completion record
and hidden capability reservation before that allocation. If observer allocation
failed, the kernel would enter its allocation-failure path rather than return
the submission backpressure error. Count admission does not guarantee available
global heap backing.

Both production paths now use `Arc::try_new`, returning
`SubmitError::WouldBlock` on observer allocation failure. The observer is still
prepared before callback registration, completion/authority publication and
enqueue. Ordinary rejection drops all unpublished owners: event/node,
cancellation state, record and any capability reservation. The rejected
observer payload's weak completion reference is released as well. No CQ result
is fabricated and no live submission slot is installed. Detached operation IDs
consumed during preparation remain unreused.

The private preparation helpers permit deterministic substitution of only the
observer allocator in kernel fixtures. Public signatures, the syscall's existing
submission-failure representation, timer admission limits and publication
ordering are unchanged. The observers continue to use the global allocator;
this adds no new observer-backing budget or physical recovery mechanism.

Contract: [scheduler timer budgets](../../reference/scheduler-timer-budgets.md).

## Regression evidence

- With an existing pending operation and one remaining slot, each timer family
  rejects its observer allocation 64 times after real event/node and record
  preparation. Every rejection restores node and ordinary capability,
  completion-record, event and observer-list admission counts, plus namespace
  live/table/detached/backlog counts. No result appears in the CQ; the existing
  operation remains in flight.
- One rejected capability observer deliberately leaves a weak record reference
  in the fixture. Its payload is dead and its event is gone, but record admission
  remains charged and rejects further submission until that final weak reference
  is released. All captured counters then return to baseline.
- Real zero-duration capability and detached timers subsequently enqueue,
  expire and deliver successful CQ results with the correct operation/cookie
  identities. The same remaining slot is reused; unrelated pending work remains
  live. Normal timer cancellation and submission abort also release their owners.
- These are controlled allocation errors, not actual heap exhaustion, physical
  corruption or an application exploit. No end-to-end workload that exhausts
  the global heap was demonstrated.

The first QEMU runs stopped at a fixture assertion that incorrectly equated a
capability timer's CQ operation ID with its capability. The test now captures
the separate operation ID and checks the capability cookie. Failure-path
checks had already completed. Final validation uses the corrected fixture.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,784 requests. |

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance timer-observer-v2-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance timer-observer-v2-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance timer-observer-v2-arm-20261007 --fresh-storage --timeout 180
```

All runners exited successfully, rebuilt kernels and enforced assembly section
permissions. Existing validated embedded services were reused for this
kernel-only change. Arm required local forwarding-port permission.

## Remaining scope

Completion and detached registries still insert into infallibly allocating
`BTreeMap`s, and hidden capability admission has its own registry metadata.
Observer allocation rejection now rolls back correctly, but the entire
submission path is not safe under heap exhaustion. General kernel heap,
registry/sponsor metadata, arbitrary callback captures, other weak-only backing
and per-principal aggregates remain SEC-07 work.

Allocation/destruction under completion and other masking guards remains SEC-18
work. Physical-platform quiescence, broader device reset and abandoned-owner
recovery are unchanged. Authentication, production provisioning and security
time findings are unchanged. Scoped guest tests do not certify hostile-workload
containment or close either broad finding.
