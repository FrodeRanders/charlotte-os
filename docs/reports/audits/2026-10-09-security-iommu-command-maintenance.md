# DMA command-engine ownership and unlocked retirement maintenance

This continues the [detached-domain correction](2026-10-09-security-iommu-detached-release.md)
within C14/C15/G3 of the [cleanup/recovery strategy](../../reference/cleanup-recovery.md).
The audit ledger remains **20 corrected, six partial and four open**. This
qualifies explicit domain destruction's maintenance boundary; G3 and SEC-18
remain partial because other backend operations and enclosing contexts remain.

## Correction and containing ownership

Physical release was already outside backend serialization, but explicit destroy
still held its masking mutex through configuration invalidation and transaction
drain. Simply copying register/ring descriptors would permit another caller to
change the shared command producer or completion cell while retirement waited.
Removing only the guard would not preserve exclusive hardware-command ownership.

Each installed unit now owns an `Option<Commands>` with its actual command state.
VT-d owns its register base, invalidation offset and required read/write draining
command. AMD owns its command buffer, producer, completion cell and checked
completion epoch. SMMUv3 owns its queue and producer. These owners are not
cloneable and are never reconstructed from snapshots. Queue/completion/table
backing remains installed and unit-owned for its kernel lifetime; extracting a
command owner does not free, replace or adopt that backing. IRQ fault/event
handling retains its existing independent immutable locations and does not
mutate this command engine.

Explicit retirement marks its exact domain retiring and publishes the rejecting
hardware descriptor under the backend registry. SMMUv3 now separates descriptor
publication from CFGI/SYNC; ordinary creation still composes both operations.
Before leaving serialization, retirement extracts the complete domain and actual
engine together into `Maintenance<D,C>`. The existing domain cell remains empty
and its requester entry remains nonzero. No new registry, node allocation,
teardown snapshot or scalar ownership reconstruction is used.

An absent engine fences every ordinary backend closure before payload/descriptor
mutation or reset callback execution. Competing calls return `OperationInFlight`,
distinct from hardware timeout or an absent domain. This gate also rejects an
absent-domain close while the unit is claimed, so it cannot publish false
completion through an active claim. The fencing is unit-wide because the queue
and completion state are shared across domains.

Hardware maintenance runs outside backend serialization using that moved engine.
Each backend keeps its own completion contract: VT-d context invalidation and
read/write drain, AMD strict exact-epoch completion stores, and SMMUv3 aborting
STE CFGI/SYNC followed by the original ASID TLBI/SYNC. Intel additionally checks
that a busy invalidation register has cleared before writing a new command;
a timeout must not authorize overwriting an older unconsumed register command.
AMD and Arm retain their previous producer/epoch/full-queue rules.

Ordinary hardware rejection restores the actual engine state and exact retiring
domain together under one original registry hold. Its admitted cell/source
identity is checked before transfer, and restoration allocates nothing. The
requester fence, table charges and all data pins remain; a later destroy may
retry hardware maintenance only against that retained state. No queue rewind,
completion-cell reset, stale acknowledgement acceptance or pin release occurs.

Abandonment retains both owners through their `ManuallyDrop` fields without
registries, allocators, pools, callbacks, logging or implicit field destructors.
The missing engine and empty domain cell permanently fence the unit and requester.
This state is terminal retention; it has no operator retry/custody interface and
cannot be cleared to manufacture progress. Other installed domains/unit backing
remain registered and owned; abandonment does not claim they were quiesced.
The enclosing device/root transaction retains its existing abandonment contract.

Confirmed maintenance returns the engine before continuing the previous unlocked
physical phase. Only the domain stays detached. `Tables` still freezes before
physical work, restores its complete frozen domain to the same cell on ordinary
physical rejection, and never retries partially freed addresses. Physical
finalization uses a registered-state hold that does not require engine admission:
an unrelated domain can own the engine while the already-quiescent domain
restores/finalizes its exact cell. Ordinary mutating entry points cannot use that
restore-only path. Successful domain removal still leaves the reset-required
requester tombstone, then consumes data pins outside backend serialization.

## Evidence

The guarded containing-owner probe now includes both a real allocated domain
metadata vector and a separate allocated command-metadata vector. Both destructor
counters stay unchanged while backend registries, lifecycle, both CPU tables,
heap/physical allocators and the original table pool are held. Empty domain and
engine cells remain claimed. Existing partial physical failure/restoration and
successor cleanup checks remain. No additional retained table charge/frame is
introduced: the IOMMU table retention fixtures remain **22 charges and 15 frames**.

Private RAM-backed command fixtures exercise production engine methods without
installing their register/ring backing in hardware:

- Both compiled x86 engines execute on Intel and AMD guests. VT-d preserves
  busy context/IOTLB register values before any replacement write. Private idle
  registers accept new commands and time out without a fabricated completion.
- AMD timeout preserves producer 32/epoch 1 through move/restoration. A private
  late store of epoch 1 cannot complete a new epoch 2; producer advances to 64
  while earlier command words remain. Full-ring rejection leaves producer/slot
  unchanged while its admitted epoch advances. A one-slot queue accepts its
  invalidation but rejects the completion command, preserving producer 80/epoch
  4 and the submitted prefix. Epoch exhaustion rejects before queue mutation. The completion command retains strict coherent store encoding.
- Arm timeout preserves producer 2 through restoration, and further CFGI/SYNC
  advances to 4 without overwriting prior ASID/SYNC commands. Full or malformed
  consumer values reject without producer/slot mutation. A one-slot queue accepts
  TLBI but rejects SYNC admission, preserving producer 5 and its installed prefix.

Private fixtures end their command scope and explicitly cancel never-published
backing. Their original charge/free-frame baselines restore. These checks are
RAM simulations of timeout/stale state, not actual hardware stalls or withheld
acknowledgements, and no actual panic unwinding/allocator corruption is induced.

The real pre-driver QEMU NVMe recovery fixture now checks two per-call boundaries.
Before maintenance it verifies caller IRQ state and backend/lifecycle/device/
physical/heap guard availability while the actual engine is absent. Initialize,
create/reset, map, unmap and destroy—including another absent domain—reject with
`OperationInFlight`; reset's callback cannot run. Memory close still rejects on
its live data pin and charges stay unchanged. After real maintenance it repeats
guard/IRQ/pin/charge checks with the restored engine and still-owned domain cell.

The existing rejected-drain case restores live backend ownership and permits
real subsequent retirement. Old-MMIO exclusion, supported NVMe reset/reassignment,
node pressure, sparse-prefix rollback pins, root/stack ownership, user isolation
and scheduler fixtures remain enabled. The NVMe controller has saved admin-queue
state but no submitted I/O command. These are reentrant boundary probes, not
concurrent cross-LP command contention or outstanding-I/O tests. No acknowledgement
is fabricated for production hardware.

## Validation and remaining work

The complete `scripts/run-host-tests.sh` harness passed, including 29 slot/lease
owner tests, four retirement-list tests and thirteen signer CLI tests. Clippy
passed on both custom targets with `--locked -- -D warnings`, using staged
service bundles. Formatting, diff whitespace and all 88 local documentation
links/anchors passed; eighteen owner families and seven gates remain unchanged.

| Final QEMU target, four LPs and isolated fresh storage | Result | Evidence |
| --- | --- | --- |
| Intel VT-d | 15 passed, zero failed/pending | `/private/tmp/charlotte-iommu-maintenance-intel-final.log` |
| AMD-Vi | 15 passed, zero failed/pending | `/private/tmp/charlotte-iommu-maintenance-amd-final.log` |
| Arm SMMUv3 security suite | 19 passed, zero failed/pending; probe `0xffff`, retired-policy/publication generation 2, 4,844 cancellation requests | `/private/tmp/charlotte-iommu-maintenance-arm-final.log` |

Both final x86 executions used kernel SHA-256
`42f81c39d581b69db48dfb1fd5aacda9b8ce8f747b5f98ae5a29bb590db5bee4`.
Arm used
`344d888d1d356d340a73ab3b5315e51b083aa8adde972acac9448a3b9cde261c`.
Boot runners verified executable/read-only assembly ownership for 249 x86 native
entries and one Arm entry. QEMU executions ran sequentially after host/Clippy;
userspace bundles were unchanged and reused. Arm used forwarded ports
18599/17899. All instances used distinct names and fresh storage.

Host evidence is `/private/tmp/charlotte-iommu-maintenance-host.log`.
Final Clippy evidence is
`/private/tmp/charlotte-iommu-maintenance-clippy-{x86,arm}-tested.log`.
Preliminary Intel/AMD runs also passed and remain in
`/private/tmp/charlotte-iommu-maintenance-{intel,amd}.log`. Final runs include the
additional completion/SYNC admission failure after invalidation submission.
The earlier stack-retirement timeout did not recur; fresh success does not close
its unresolved diagnosis.

Creation, map/unmap and initialization maintenance still hold backend guards.
Private allocation/rollback, registry metadata admission/destruction and complete
outer-mask/caller qualification remain G1/G3/G4 work. Extending engine claims to
those operations also needs their exact payload/source/publication owners; an
engine claim alone does not make failed publication rollback safe. Unit-wide
contention/fairness and actual concurrent domain cleanup remain G7 evidence gaps.
No common recovery registry, external operator authorization, physical qualification
or SEC-18 closure is claimed.

The earlier Intel user-stack retirement timeout remains unresolved. Its failed
executions and exact diagnostic limitations remain in the
[preparation report](2026-10-09-security-iommu-preparation-abandonment.md).
Fresh-run success does not resolve its cause or close C16/G1/G2/G7.
