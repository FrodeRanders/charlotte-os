# Detached DMA-domain physical release

This continues the [IOMMU preparation correction](2026-10-09-security-iommu-preparation-abandonment.md)
within existing C14/C15/G3 of the [cleanup/recovery strategy](../../reference/cleanup-recovery.md).
The audit ledger remains **20 corrected, six partial and four open**. This is
scoped SEC-18 progress; full backend phase separation and SEC-18 remain open.

## Correction and containing ownership

Explicit VT-d, AMD-Vi and SMMUv3 destruction previously held their masking backend
mutex through configuration/TLB maintenance and physical table release. Moving
only a table owner would have left domain pins and metadata outside its containing
transaction. Removing the registry node would also require allocating reinsertion
on rejection and could make a competing close report false completion.

All three registries now admit an `Option<Domain>` cell during creation. Their
existing typed hardware maintenance remains serialized and unchanged: VT-d
context invalidation plus read/write drain, AMD strict completion-store epochs,
and SMMUv3 aborting STE configuration followed by the original ASID TLBI/SYNC.
Only confirmed maintenance allows extraction of the complete domain into the
shared retention-only `DetachedDomain<T>` owner. This generic retention wrapper
is not a completion proof, a queue-state snapshot or a recovery registry.

The original admitted cell remains empty and its requester entry retains the
nonzero original domain ID throughout physical work. IDs are monotonic and never
reused. Competing map/unmap/destroy reject that empty cell; create/reset rejects
the still-owned requester before invoking its reset callback. An absent domain
retains the existing idempotent close behavior, distinct from a claimed cell.
Other domains may operate after maintenance; no shared command queue/tail/epoch
is moved, copied, cleared or reset during physical teardown.

`Tables::release` and successful table-ledger disposal now execute outside the
backend guard. Its existing freeze-before-release rule remains: partial physical
rejection retains the entire original charge, including capacity for returned
frames, and never revisits freed addresses. Ordinary rejection restores the exact
complete frozen domain into its existing cell without node allocation. Mappings,
quarantine pins and original table admission remain owned; map/unmap still reject
the retiring domain. A physical failure is not a retryable hardware receipt.

Abandonment retains every domain field through `ManuallyDrop`, including table
ledger, mapping/quarantine storage, pins and charges. It does not run implicit
field destructors or enter registries, heap/physical allocators, pools, callbacks
or logging. The empty admitted cell and requester fence remain permanently
claimed. This inaccessible retained payload is terminal quarantine, not a common
custody owner or an operator-authorized retry. The enclosing device cleanup/root
owner keeps its existing abandonment contract.

Confirmed physical completion removes the empty cell and installs the existing
reset-required requester tombstone. Mapping/pin collections and remaining domain
metadata are consumed outside the backend guard. Supported device reset still
requires old MMIO authority exclusion and confirmed controller reset; completion
does not override those boundaries or refund any abandoned predecessor.

## Evidence

Private boot probes use actual table backing and a containing heap-allocated
metadata vector with a destructor counter. They exercise complete-owner
abandonment while backend registries, lifecycle, both CPU tables, heap/physical
allocators and the original table pool are held. No metadata destructor, frame
release or charge change occurs. An empty cell remains claimed.

An injected two-frame physical walk returns the first frame and rejects the
second. The exact metadata allocation identity survives restoration into the
existing cell while the same registry/allocator guards are held. A subsequent release rejects before its physical callback; guarded
abandonment retains the complete frozen payload. A successful successor releases
its own tables/metadata and cannot refund earlier retained admission. These
probes retain **three original domain charges and two frames**, bringing the
IOMMU table retention fixtures to **22 original charges and 15 frames**, separate
from live hardware backing. No actual panic unwinding or allocator corruption is
performed; these table roots were never published to hardware.

The real pre-driver NVMe fixture retains its existing rejected-drain case, then
uses a per-call boundary after real maintenance and before any table release or
data unpin. It checks both compiled backend guards on x86, the SMMU guard on Arm,
and lifecycle/device/physical/heap guard availability. Caller IRQ state is
preserved. Charges remain unchanged and memory close still rejects on the live
pin. Reentrant create/reset, map, unmap and destroy calls reject without consuming
the claim; reset's callback cannot run. Normal completion then permits capability
cleanup and the existing old-MMIO exclusion/reset/reassignment sequence.

These are reentrant boot probes, not cross-LP stress or an outstanding-I/O test.
The enabled controller has saved admin-queue state but no submitted I/O command.
No hardware acknowledgement is fabricated or withheld. The existing sparse map
prefix failure/pin retention, admission pressure, stack, root, user isolation and
scheduler fixtures remain enabled.

## Validation and remaining work

The complete `scripts/run-host-tests.sh` harness passed, including 29 slot/lease
owner tests, four retirement-list tests and the thirteen signer CLI tests.
Clippy passed with `--locked -- -D warnings` for both custom targets, using the
existing staged service bundles. `cargo fmt --all -- --check` and `git diff
--check` passed. Local documentation links/anchors validated and the coverage
map remains eighteen owner families and seven gates.

| Final QEMU execution | Result | Evidence |
| --- | --- | --- |
| Intel VT-d, four LPs, isolated fresh storage | 15 passed, zero failed/pending | `/private/tmp/charlotte-iommu-detached-intel-final.log` |
| AMD-Vi, four LPs, isolated fresh storage | 15 passed, zero failed/pending | `/private/tmp/charlotte-iommu-detached-amd-final.log` |
| Arm SMMUv3 security suite, four LPs, isolated fresh storage | 19 passed, zero failed/pending; probe `0xffff`, retired-policy/publication generation 2, 4,836 cancellation requests | `/private/tmp/charlotte-iommu-detached-arm-final.log` |

Both x86 executions used kernel SHA-256
`5c963c021a7964bac330e315b7040a2e3ac16d932f6e727ca5369efe4cfd3b93`.
Arm used
`b6384f8d8d148bc5a85f895a0ef194249fee4ad4886cc0891733fde1a7c801cc`.
Boot runners checked executable/read-only assembly ownership for 249 x86 native
entries and one Arm entry. QEMU executions ran sequentially after host/Clippy
completion; bundled userspace code was unchanged. Arm used forwarded ports
18589/17889 and each runner used a unique instance name with fresh storage.

Final Clippy evidence is in
`/private/tmp/charlotte-iommu-detached-clippy-{x86,arm}-final-owner.log`;
host evidence is `/private/tmp/charlotte-iommu-detached-host.log`.
Preliminary runs also passed, but review found that their new metadata probe
used a zero-sized element. Those runs do not prove containing heap-allocation
retention. The corrected probe uses a real allocated vector and additionally
restores the exact slot under held guards; all three final executions above
reran that corrected code. Preliminary logs remain in
`/private/tmp/charlotte-iommu-detached-{intel,amd,arm}.log`.
The stack timeout did not recur in these executions; no causal conclusion or
closure follows from that success.

Hardware command submission/waits and creation rollback still hold backend
serialization. A complete shared command-engine claim, preserving each backend's
queue/epoch completion contract through timeout and abandonment, is required
before moving those waits outside it. Private constructor allocation/rollback,
registry-node allocation/destruction, unrelated enclosing masks and general
metadata admission remain G1/G3/G4 work. No new owner-family row, unbounded
registry, force-clear, physical partial-release retry or production recovery
claim is introduced.

The earlier Intel user-stack lease timeout remains unresolved. Fresh-run success
cannot close C16/G1/G2/G7 or SEC-18; its original failed executions and diagnostics
remain in the [preparation report](2026-10-09-security-iommu-preparation-abandonment.md).
