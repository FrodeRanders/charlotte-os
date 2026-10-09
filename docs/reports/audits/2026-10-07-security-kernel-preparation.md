# Provisional kernel-frame abandonment

Date: 2026-10-07. SEC-18 continuation after observer-list admission at
`3f9e58b`. This corrects a provisional data-frame destructor contract; broader
physical recovery and metadata/heap findings remain partial.

## Finding and correction

`PreparingKernelFrame::drop` acquired the physical allocator and immediately
freed its standard/large/huge extent. The owner could not prove that leaf
publication had not happened, nor that its caller held no allocator/table guard.
Explicit abandonment after publication could return reachable backing; destruction
under an already-held allocator could self-deadlock. Its large/huge deallocator
also held one allocator guard for the full extent, bypassing the explicit
receipt's sixteen-frame batching policy.

This is a latent unsafe destructor contract. Current production range helpers
already transfer ordinary mapping rejection into a retirement receipt; both
kernel profiles and custom targets use panic abort. No live syscall trigger or
panic-unwinding path was demonstrated. Controlled kernel fixtures exercise the
unsafe ownership states directly.

The destructor now retains the complete physical extent and updates only the
atomic kernel quarantine counter. It allocates nothing, takes no allocator/table
guard, frees nothing, performs no invalidation and enters no logger. Apparently
unpublished backing also remains unavailable on abandonment: an unknown caller
context is insufficient permission for implicit physical cleanup.

Ordinary success still consumes the owner into kernel mapping ownership.
Ordinary rejection consumes it into `RetiredKernelRange`, whose explicit release
runs after guards are gone, with real invalidation and bounded physical batches.
The old scalar extent-deallocation helper is removed from this adapter. Detached
foreign-leaf and successor fixtures now use fresh owning receipts for confirmed
release rather than relying on provisional Drop.

No frame is re-adopted after abandonment. There is no quarantine recovery,
allocator retry, new admission pool, userspace ABI or protocol change. Runtime
stack owners retain their existing whole reservation/root/slot on incomplete
cleanup; this does not change their refund boundary or provide general kernel
heap accounting.

Contract: [kernel-frame retirement](../../reference/kernel-frame-retirement.md).

## Regression evidence

- A real unpublished standard frame is abandoned while its physical allocator
  guard is held. Drop returns without acquiring that guard or changing free-frame
  count. Its unavailable page increments quarantine exactly once.
- A second real frame is mapped into an already-warmed kernel table. Its
  provisional owner is then abandoned while both kernel-table and physical
  allocator guards are held. Free-frame count does not increase and the installed
  leaf still resolves to that exact physical frame.
- After releasing both guards, the fixture completes real invalidation following
  leaf detachment. It never re-adopts or releases that abandoned backing. Both
  pages remain unavailable; allocator/table guards are available afterward.
- Existing foreign-leaf and successor cleanup explicitly consume preparation
  owners into receipts and restore their expected free-frame counts. Existing
  allocation/map rejection, prefix rollback, barrier failure/retry, standard and
  2 MiB release batching, terminal partial release and interrupted-receipt cases
  remain enabled.
- The two new pages plus the existing three kernel-range quarantine pages leave
  **five 4 KiB data pages** reserved for each test guest's lifetime. Shared table
  lifetime and other subsystem quarantine fixtures are counted independently.
- Abandonment is controlled, not actual panic unwinding, allocator corruption,
  a physical hardware fault or an application exploit. Large/huge abandonment
  uses the same compiled destructor, but only standard frames are abandoned by
  these new fixtures. Existing real 2 MiB physical-release tests still run.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,668 requests. |

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance kernel-preparation-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance kernel-preparation-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance kernel-preparation-arm-20261007 --fresh-storage --timeout 180
```

Runners rebuild kernels and enforce assembly section permissions. Existing
validated embedded services are reused for this kernel-only change. Arm uses
required local forwarding-port permission.

## Remaining scope

Abandoned data remains permanently unavailable; this change supplies no
administrative recovery or owner reconstruction. SEC-18 still includes broader
allocation/destruction under masking guards, physical-platform quiescence,
broader device reset and abandoned-owner recovery. Frame-release helpers outside
this adapter are unchanged.

SEC-07 remains partial for general kernel heap, registry/sponsor metadata,
arbitrary callback captures, other weak-only allocations and principal aggregates.
Authentication, production provisioning and security-time findings are unchanged.
The passing scoped tests do not certify hostile-workload containment.
