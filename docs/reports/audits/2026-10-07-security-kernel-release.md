# Terminal kernel-range physical release

Date: 2026-10-07. SEC-18 continuation after IOMMU admission at `81517176`.
This corrects a remaining physical-release retry boundary; broader allocator,
metadata and recovery findings remain partial.

## Finding and correction

`RetiredKernelRange` retained a failing physical extent for later `release`
retry. The large/huge deallocator releases constituent 4 KiB frames in order
and can reject after a partial prefix. The receipt could therefore retry an
extent whose prefix was already free. If a freed address had been allocated
again, stale cleanup could release the successor's backing. This was confirmed
by source review and a controlled physical-ownership regression; no application
exploit or real allocator corruption was demonstrated.

The receipt now arms a terminal physical phase before its first allocator call.
Physical rejection or interrupted ownership prevents further invalidation,
deallocation and receipt reinitialization. Only a completely successful physical
walk clears the phase and permits ordinary receipt reuse. Failed invalidation
before physical release still retains the same owner for real barrier retry;
repeated release after complete success remains harmless.

Standard, large and huge extents are expanded into a fixed sixteen-address
base-frame batch. The physical allocator is held for at most sixteen 4 KiB
releases, with its guard gone between batches. There is no new heap allocation,
address-space lookup, teardown snapshot or table/arena guard. Confirmed progress
counts individual returned pages rather than treating a partially released
large extent as wholly retained. An interrupted batch can conservatively retain
an uncertain diagnostic count, but can never retry physical addresses.

Drop preserves the remaining backing and updates the existing quarantine
counter. Stack cleanup still returns failure to its owning `Stacks` transaction;
that transaction retains its whole original reservation/root/slot even if some
actual data frames released. No refund, administrative recovery or owner
re-adoption is introduced. Other bulk physical allocator helpers are unchanged.

Contract: [kernel-frame retirement](../../reference/kernel-frame-retirement.md).

## Regression evidence

- Thirty-five real standard pages detach, invalidate and release in batches of
  16/16/3. Batch hooks verify physical and kernel-table guards are available.
  Successful cleanup restores the exact free-frame count.
- A real 2 MiB kernel leaf detaches and invalidates, then releases its 512 base
  frames in thirty-two batches. The same receipt remains usable after success.
- A second 2 MiB leaf rejects its final base-frame release after 511 real frees.
  One frame remains unavailable and the receipt is terminal. A new kernel
  physical owner claims its first freed address to reproduce allocator reuse.
  Rejected retry invokes no invalidation/deallocation/completion callback and
  does not free that successor. Reinitialization rejects too. Dropping the old
  receipt changes no free count; normal successor Drop succeeds.
- Simulated interruption after real completed invalidation and phase admission
  retains one additional frame and forbids physical retry. The existing Drop
  case retains its original frame. Together these kernel-range fixtures retain
  three 4 KiB frames, with exact quarantine-count deltas.
- Physical rejection/interruption are controlled fixture states, not actual
  allocator corruption, panic unwinding or manufactured hardware completion.
  Huge-page physical release is compiled but not exercised with a real 1 GiB
  allocation. Existing failed-barrier/retry, foreign-leaf protection, partial
  map/allocation rollback, bounded range admission, stack and IOMMU tests remain.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,800 requests. |

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance kernel-release-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance kernel-release-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance kernel-release-arm-20261007 --fresh-storage --timeout 180
```

Runners rebuild kernels and enforce native assembly section permissions. Existing
validated embedded services are reused for this kernel-only change. The first
Arm invocation could not bind its forwarding port inside the sandbox; the final
run uses the required permission.

## Remaining scope

SEC-18 remains partial for other allocation/destruction under masking subsystem
guards, full physical-platform quiescence, broader device reset and recovery of
abandoned owners. Batching bounds each allocator hold, not total teardown time,
fairness or worst-case latency. Partial physical release remains permanently
fenced. There is no general quarantine reclamation API.

SEC-07 still includes inherited boot/CPU/interrupt tables/stacks, kernel heap,
general metadata and callback storage. This receipt change adds no admission
pool and does not establish whole-node exhaustion isolation. Authentication,
production provisioning and security-time findings remain separate work.
