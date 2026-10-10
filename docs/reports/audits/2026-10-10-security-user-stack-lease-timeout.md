# Intermittent user-stack lease timeout investigation (2026-10-10)

This investigates the historical intermittent Intel/AMD **user-stack lease
timeout** that blocked final-root close in the user-isolation fixture, and
records the bounded correction. It is a C16/G1/G2 availability and
owner-continuity item under SEC-18; no finding state changes.

## Symptom and history

`user_isolation::verify()` spawns faulted EL0 threads, waits for them to leave
the master thread table, then closes the fixture's address space. Close
succeeds only once every stack pair's exact root lease is released. On the
historical failures, close kept returning `OperationsInFlight` until the
fixture's ten-second deadline expired and it panicked with
`user stack retirement lease`
([user_isolation.rs:171](../../../crates/catten/src/self_test/user_isolation.rs#L171)).

The failure was reproduced historically twice: Intel during the
IOMMU follow-up and AMD during the published-creation follow-up
([dma-creation-rollback report](2026-10-09-security-dma-creation-rollback.md)),
with a passing unchanged-artifact repeat. The abort-handoff and
executor-owner corrections closed the identified self-request window but were
not established as the cause. The timeout remained causally unresolved in the
[remediation ledger](../../reference/security-remediation.md).

## Investigation

The release path is
[`Stacks::release_with`](../../../crates/catten/src/memory/thread_stack/stacks.rs#L226),
reached only from the pinned, IRQ-enabled reaper
([threads/mod.rs:236](../../../crates/catten/src/cpu/scheduler/threads/mod.rs#L237)).
The reaper skips masked callers and defers in-use contexts; on x86
`is_on_cpu()` is constant `false` and `kernel_stack_contains` is exact, so a
dead context is attempted on the next reaper pass.

Failure classes after the detach step are deterministic:

- identity rejection compares the captured root generation;
- detach rejection comes from page-table unmap of an exactly tracked leaf;
- physical rejection checks the allocator bitmap (only a double free,
  misalignment or invalid address fails);
- only user-translation invalidation is a rendezvous that can fail
  transiently. On x86 it is a global epoch-fenced shootdown with a 100 ms
  acknowledgement deadline and a 500 ms admission deadline
  ([ipis/mod.rs:102](../../../crates/catten/src/cpu/isa/x86_64/interrupts/fixed/ipis/mod.rs#L102));
  it can return `Busy`, `TimedOut` or `Delivery` under load.

Both the shootdown API and the release contract are one-shot: any failure
retains the complete pair, arms the `release_started` phase fence and is never
retried by later scans. Pages that were already detached from the page table
therefore keep their root lease forever, so one transient rendezvous miss
permanently blocks address-space close. That matches the fixture symptom.

Reproduction attempts on the pre-correction kernel (revision
`f66f793c`-derived worktree, `--no-network`, fresh storage, 4-LP guests):

- 16 sequential runs alternating `--iommu intel`/`amd`;
- 3 concurrent 8-LP runs per wave (three-way stress), until the source change
  landed.

All 19 pre-correction runs passed; the historical failure was not reproduced.
The runs therefore do not identify the missed acknowledgement's cause, do not
justify a deadline increase, and do not close the item.

## Correction

User-translation invalidation is now re-confirmed with a bounded
`USER_INVALIDATION_ATTEMPTS = 3` fresh rendezvous attempts **before** the pair
is declared failed
([stacks.rs:273](../../../crates/catten/src/memory/thread_stack/stacks.rs#L278)).
The correction preserves the contract:

- no frame is released until invalidation succeeds, so retrying only
  re-confirms quiescence of already-detached translations and cannot
  double-free;
- each attempt uses a new epoch from the shared shootdown coordinator; stale
  or duplicate acknowledgements cannot satisfy a retry;
- `release_started` remains one-shot: after the bound is exhausted the pair is
  retained exactly as before, and later scans never retry it;
- deterministic identity/detach/physical rejections are not re-attempted;
- a post-success physical interruption remains terminal.

## Evidence

Kernel stack-admission fixtures now cover both outcomes:

- `transient_invalidation_retries_before_failure` fails the first two
  invalidations and succeeds on the third. The pair releases completely: 17
  frames returned, reservation budget restored to baseline, slot at zero, and
  root close succeeds.
- `retained_failures`'s invalidation case asserts exactly
  `USER_INVALIDATION_ATTEMPTS` attempts, no physical release and one retained
  owner; the aggregate `INVALIDATION_REJECTED` counter still increments once.
- The boot banner now reports transient-invalidation re-confirmation in the
  passing suite.

Validation on the corrected tree:

| Check | Result |
| --- | --- |
| x86_64 Intel VT-d guest | 15 passed, 0 failed, 0 pending (`0x1bbff`) |
| x86_64 AMD-Vi guest | 15 passed, 0 failed, 0 pending (`0x1bbff`) |
| AArch64 security guest | 19 passed, 0 failed, 0 pending (`0x2003ffff`) |
| Workspace host suites | 342 tests, zero failures |
| Clippy `-D warnings` | AArch64/x86_64, default and `live_upgrade_test` |

Kernel SHA-256:

- x86_64 (both IOMMU backends):
  `2bbdcc30527673174ac6833c897b6674cfe0963c3456eef089e89c21f9320af8`
- AArch64 security guest:
  `527275eadea9810375b0c94bab681400028e8996bb0259892e6071b5aea36983`

Guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu intel \
  --instance stack-fix-intel --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance stack-fix-amd --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18851 CATTEN_DEPLOY_HOST_PORT=18151 \
  scripts/run-aarch64.sh --security-test \
  --instance stack-fix-arm --fresh-storage --timeout 180
```

## Limits

The historical failure was not reproduced, so this does not prove the missed
invalidation was its cause. A future retained snapshot showing `error=User`
with `INVALIDATION_REJECTED` after the bounded attempts, or a perpetual
deferral (`started=false`, no error), would indicate a different residual
cause. The change does not add an operator-visible retry owner, does not
recover already-retained pairs, and does not address the enclosing
reaper/caller IRQ context or general thread metadata fallback tracked in
[stack admission](../../reference/stack-admission.md) and
[C16/G1/G2](../../reference/cleanup-recovery.md).
