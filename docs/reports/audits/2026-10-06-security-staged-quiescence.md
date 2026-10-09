# Staged-copy rollback and QEMU quiescence/recovery

Date: 2026-10-06. SEC-18 follow-up to IPC backing release at `890d19d8`.
The requested hardware target is QEMU first. SEC-18 remains partial; current
contracts live in [hardware quiescence](../../reference/hardware-quiescence.md)
and [memory-object retirement](../../reference/memory-object-retirement.md).

## Staged-copy rollback

Scalar copied calls/sends, copied calls carrying delegated connections and
vector calls/sends now capture both exact namespace roots before preparation.
They parse descriptors, allocate/copy frames and roll back unpublished backing
outside IPC serialization. Publication revalidates connection payload, endpoint
ownership/queue admission and captured namespace identities. Delegated sources
are revalidated before jointly publishing memory and connection authority.

`PreparedCall::commit_retained` returns its owner on publication/admission
rejection; destruction follows IPC guard release. Vector entry storage is
prepared fallibly. Ordinary completion releases root leases after staging ends;
panic/abandonment retains them. Scalar move/loan adapters keep their existing
serialization; ordinary metadata allocation/destruction under IPC remains open.

Boot fixtures exercise partial vector-copy failure, retirement-storage rejection,
retired publication, destination closure, 35-page multi-batch rollback, source
restoration, unchanged capability/queue counts and exact root retention. Cleanup
callbacks assert that IPC/lifecycle/table/object/allocator guards are available.
No additional permanent copy-quarantine fixture is introduced.

## CPU quiescence and retained-owner retry

The previous shared countdown and fail-stop sender are replaced by bounded,
serialized epochs with an acknowledgement per LP. Failed delivery and missing
acknowledgement return errors without inventing completion. A handler captures
its epoch before flushing, and acknowledgements cannot regress. Coordinator
admission is limited to 500 ms with cooperative yield; after admission, the
acknowledgement deadline is 100 ms. IRQ-masked callers reject. Mandatory legacy
wrappers panic; owned cleanup retains physical backing instead of reusing it.

Both caller and remote flushes include global translations and paging-structure
caches through CR4.PGE maintenance as well as CR3 reload. CR3 reload alone does
not retire global translations. This follows the
[Intel system programming manual](https://cdrdv2-public.intel.com/825758/253668-sdm-vol-3a.pdf).

Retained kernel-range receipts can retry. Final-root rejection can return the
same owning hierarchy/slot/accounts before physical release starts; production
final invalidation attempts three fresh epochs before quarantine. Physical
release failure remains non-retryable, preserving the original whole charge.
Missing Arm range roots now reject rather than report false completion.

The stricter boundary exposed stack cleanup in x86 IRQ tails and stack
preparation beneath boot/test masks. Pinned scheduled reapers now perform
physical cleanup on their own LP with IRQs live; IRQ tails only stage retired
threads. Boot preparation is preemptible with individually serialized scheduler
publication. The CQ fixture prepares before its short Ready-state guard, and
user-isolation teardown allows its deferred stack-retirement lease to finish.

Host tests cover stale/duplicate/non-regressing acknowledgements, exclusive
ownership and epoch exhaustion. The four-LP guest omits one actual remote IPI
to test delivery rejection, then separately leaves one LP without delivery to
test acknowledgement timeout. Each failure retains an exact root, slot and
heap charge; a fresh real rendezvous completes teardown. No acknowledgement is
faked and no unrelated CPU/VM is stopped for injection.

## DMA completion and requester recovery

VT-d now requires advertised read and write draining and requests both during
IOTLB invalidation. Retirement now clears Context Present rather than publishing
a present physical-zero pointer. Root/context and AMD DTE secondary metadata
precede valid-link publication; invalidation leaves old secondary metadata
intact until hardware maintenance completes. An intermediate QEMU diagnostic
exposed the physical-zero context defect despite passing deferred tests.
AMD-Vi follows invalidation with a strict Completion Wait
store to Unit-owned coherent memory and checks the exact submission epoch;
command-head advancement alone is insufficient. These requirements follow the
[Intel VT-d specification](https://cdrdv2-public.intel.com/831418/vt-directed-io-spec.pdf)
and [AMD IOMMU specification](https://www.amd.com/content/dam/amd/en/documents/processor-tech-docs/specifications/48882_IOMMU.pdf).

AMD/SMMU queue admission preserves unconsumed commands after timeout. SMMU
destruction installs an aborting STE without first altering its live secondary
words, completes configuration maintenance, then performs the original ASID's
TLBI/SYNC before releasing data pins. Configuration invalidation alone is not
the data-transaction completion boundary; stage-1 invalidation completion covers
client transactions translated through its targeted entries. See
[Arm SMMUv3 architecture, sections 3.21 and 4](https://documentation-service.arm.com/static/66c5c097882fec713ef4a8ff).

All three backends fence map/unmap once retirement starts. Rejected destroy
retains table backing, data pins and source ownership for retry. Successful
destroy or confirmed creation rollback keeps a requester tombstone, preventing
new domain creation until a supported reset completes. x86 requester conversion
also rejects values outside 16 bits rather than silently truncating them.

QEMU NVMe reset checks its model, segment and bounded BAR layout, excludes old
MMIO authority under lifecycle/device serialization, disables bus mastering and
memory decoding during restored BAR-size probes, and retains the exact config
guard through new-domain creation. It clears `CC.EN` and checks `CSTS.RDY=0`
within 100 ms; failure leaves bus mastering disabled and the requester fenced.
Only successful domain creation consumes the reset owner and enables memory/
bus mastering. Supported NVMe resets at first grant as well as reassignment.
The controller reset/ready contract follows
[NVMe Base 2.0c](https://nvmexpress.org/wp-content/uploads/NVM-Express-Base-Specification-2.0c-2022.10.04-Ratified.pdf).

Public MMIO invalidation now propagates failure. Uncertain cleanup retains its
root lease, in-flight claim and mapping/scratch record. Explicit close detaches
authority before unlocking but retains a claimed registry descriptor throughout
physical cleanup; competing reset and close reject during this interval.
Failure keeps the descriptor/root without allocating a reinsertion. Metadata
is cleared only after confirmed invalidation and scratch completion, so reset
cannot mistake uncertain register
access for removed authority.

The pre-driver fixture enables a real QEMU NVMe controller with DMA admin
queues. One injected software rejection after hardware detachment verifies that
pins remain live, the owner cannot close pinned memory and retiring DMA cannot
map. Real teardown retry completes hardware invalidation; a successor's grant
rejects while old MMIO exists, including an injected attempt after explicit close
detaches authority but before its physical invalidation completes. After old
descriptor/root cleanup, actual reset precedes reassignment and `EN/RDY` both
read zero. Normal NVMe storage and
persistent-Raft tests then run after another reset on the operational grant.
The fixture submits no I/O command and does not withhold a physical IOMMU ACK.

## Validation

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including five CPU epoch and three IOMMU command/queue tests, runtime/services/protocol, ownership and signing suites. |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**; staged rollback, CPU delivery/timeout retry and NVMe recovery fixtures executed. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**; the same fixtures and operational NVMe storage passed. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; NVMe recovery executed, probes passed with `0xffff`, publication generations 1/2, concurrent cancellation retired after 4,808 requests. Pressure clients retired after 1,532/1,690 requests with time/cycle progress. |

Final commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance staged-recovery-final-intel-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance staged-recovery-final-amd-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance staged-recovery-final-arm-20261006 --fresh-storage --timeout 180
```

The runners rebuilt kernels and checked native assembly permissions. Existing
validated embedded service bundles were reused; this batch changes kernel and
host lifecycle code only. Arm forwarding used approved isolated localhost
ports. Initial sandbox forwarding rejection was rerun with approval. Earlier
x86 runs exposed the corrected masking/reaper/fixture assumptions above; the
final results are from fresh isolated instances after those corrections.

## Remaining scope

SEC-18 remains partial for general allocator/metadata work under serialization,
legacy serialized loan adapters, generic PCI function/bus reset, other device
reset adapters, permanently unresponsive physical CPUs/devices and recovery of
abandoned owners. Unsupported requester reassignment stays fenced. A timeout
never proves quiescence; all incomplete participants must actually confirm a
fresh completion before retained backing is released. These results establish
QEMU regressions, not full physical-platform safety or worst-case latency.

SEC-07 table/general metadata admission, peer/management authentication,
production provisioning and security-time work remain open as previously
documented. No quarantined root, backing, charge or abandoned claim is adopted
or force-cleared by this change.
