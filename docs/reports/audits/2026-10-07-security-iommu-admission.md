# IOMMU table admission and failed-map rollback

Date: 2026-10-07. SEC-07 follow-up after runtime stack admission at `e38776c8`,
with a related DMA rollback lifetime correction. The renewed audit's general
allocator/metadata and recovery findings remain partial.

## Changes

Intel VT-d, AMD-Vi and Arm SMMUv3 now use one hardware-table backing owner.
Each domain admits at most 1,024 actual 4 KiB pages, including its root and
SMMUv3 context descriptor. Each unit admits at most 2,048 pages for shared
tables, command/event queues and completion storage. The independent node pool
is one thirty-second of usable RAM, rounded down to pages, minimum one; all
domains share three quarters, leaving the remaining quarter for shared units.
These are initial kernel limits, not workload-derived signed entitlements.
They do not guarantee physical availability or provide aggregate per-launch
sponsorship/fairness. There is no application-controlled platform exception.

Admission and fallible ledger preparation precede physical allocation. The owner
captures contiguous backing before zeroing and remains armed until its ledger
owns the extent. Private roots/branches roll back through checked release;
physical failure retains admission. Children are initialized before publication.
Tables are marked published before the first hardware-visible context/DTE/STE
or unit-base register. Shared unit backing remains charged for controller lifetime.
Linked sparse prefixes and empty tables remain charged and reusable until domain
retirement. Separate raw allocator/free helpers and constructor cleanup ladders
are removed, including the SMMU root leak when context-descriptor allocation fails.

Registered domains now survive physical table-release failure. Retirement must
confirm detachment and existing backend maintenance/drain before release begins;
only complete physical success permits removal of the owner/source fence and pin
cleanup. A partial release enters a terminal frozen state and retains the whole
charge; later destroy cannot retry freed table frames. Published/abandoned Drop
also retains backing and charge. Creation rollback marks its owner retiring;
SMMUv3 abort rollback additionally completes the original ASID's TLBI/SYNC.

A further inspection found that failed multi-page mapping cleared its installed
prefix but immediately released its data pin. Table admission makes this failure
path routinely reachable. All three backends now confirm IOTLB drain or ASID
maintenance after prefix clearing before unpinning. Rejected completion retains
the exact pin until acknowledged domain destruction. Duplicate maps of a retained
object reject; quarantine storage is fallibly prepared before leaf publication.
Normal teardown consumes the removed domain's existing mapping/pin collections
outside backend serialization without allocating a snapshot.

Contracts: [IOMMU admission](../../reference/iommu-table-admission.md) and
[hardware quiescence](../../reference/hardware-quiescence.md).

## Regression evidence

- Common admission fixtures exercise multi-page node/subpool reservation,
  unit headroom under domain pressure, rejection before allocator callbacks,
  physical allocation failure, aligned contiguous zeroed backing and exact
  private rollback/refund. A successful repeated release refunds only once.
- Each active architecture walker uses a private unpublished root and borrowed
  data frame. A small ceiling rejects a sparse walk after retaining one linked
  table; repeated rejection adds no charge, cached reuse succeeds at the ceiling,
  and increasing the kernel-only fixture ceiling permits retry and exact cleanup.
- Published abandonment retains one frame/charge. A four-frame owner releases
  two real frames before an injected rejection; its whole four-page charge stays,
  and a second release never invokes a deallocator. Together these fixtures retain
  five charged pages and three actual frames throughout the guest. A successful
  successor cannot refund their original charges. Rejection/abandonment are
  injected states, not actual allocator corruption or panic unwinding.
- The real QEMU NVMe fixture checks unchanged table charges after rejected
  hardware completion, refund after real retirement, and capability refund when
  domain pressure rejects creation. Shared controller/context charges remain.
- A two-page public DMA map crosses a cached/fresh leaf-table boundary at its
  ceiling. Confirmed prefix rollback permits memory close; rejected completion
  retains the pin, denies duplicate mapping and makes memory close return
  `LendingActive`. Real domain retirement subsequently permits cleanup and
  restores the original domain-pool count. No I/O command is submitted and no
  physical acknowledgement is suppressed.
- Existing reset, old-MMIO exclusion, operational NVMe/object store, persistent
  Raft, IPC/ownership, table/stack and root-retirement fixtures remain enabled.

## Validation

| Check | Result |
| --- | --- |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,792 requests. |

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance iommu-admission-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance iommu-admission-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance iommu-admission-arm-20261007 --fresh-storage --timeout 180
```

Runners rebuild kernels and verify native assembly section permissions. Existing
validated embedded services are reused for this kernel-only change. The first
Arm invocation could not bind its forwarding port in the sandbox; rerunning with
the required permission completed successfully.

## Remaining scope

SEC-07 remains partial for inherited boot/CPU/interrupt stacks and tables,
general kernel heap, metadata and callback storage. Separate pools are not a
complete physical RAM ledger, worst-case latency bound or node-exhaustion proof.
IOMMU admission follows a hardware domain/unit, not an aggregate userspace
sponsor; controller recovery and shared-unit reclamation remain absent.

SEC-18 allocation under backend serialization remains: table ledger preparation
is fallible/bounded, but other BTree registries/caches still use the general heap,
and physical allocation remains under backend guards. Broader reset support,
physical-platform quiescence and abandoned-owner recovery are unchanged. A
frozen partial-release domain remains permanently fenced; there is no force-clear
or unsafe retry. Authentication, production provisioning and security-time
findings remain separate work.
