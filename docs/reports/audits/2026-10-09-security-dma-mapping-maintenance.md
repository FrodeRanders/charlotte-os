# Complete DMA map/unmap ownership through unlocked maintenance

This continues the [unit-initialization correction](2026-10-09-security-iommu-unit-initialization.md)
within C13/C14/C15 and G1/G2/G3/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger remains **20 corrected, six partial and four
open**. No new owner family, registry or custody controller is introduced.

## Defect and correction

All three backends previously kept their unit registry mutex through ordinary
map/unmap and failed-prefix IOTLB/ASID maintenance. The public device layer
resolved a scalar domain ID and released the device registry, but held no exact
user-root operation or capability claim through those waits. On Arm, rejected
unmap reinserted its removed mapping into a `BTreeMap`, allocating a new metadata
node on an exceptional cleanup path.

Public DMA map/map-exclusive/unmap now use `DmaOperation`. It admits the exact
user-root operation before claiming the live capability in the existing device
entry. The in-flight bit excludes competing operations and explicit close;
namespace device preparation also rejects that claim. The permanent kernel
root remains a separate, non-reusable case. Ordinary admission rejection releases
its unused lease explicitly. Complete operation abandonment retains the root
count and capability claim without cleanup.

The backend extracts the complete nonretiring domain and actual command engine
into the existing typed `Maintenance`, leaving its admitted domain cell empty,
requester nonzero and engine absent. `MappingMaintenance` adds `PendingPin` for
any data pin not currently in a mapping/quarantine collection. These fields use
`DetachedDomain` retention; no implicit table, vector, pin or command-metadata
destructor runs on abandonment. Ordinary mutation, reset, competing destruction
and early initialization reject the absent engine before changing hardware.

Sparse walking, mapping metadata preparation, leaf detachment and maintenance
then run outside the backend guard. The domain borrows the pending pin through
fallible walk preparation. No failed prefix may unpin without its backend's
existing completion proof: VT-d draining invalidation, AMD strict completion
epochs, or Arm ASID TLBI/SYNC. Actual queue state is moved, not reconstructed.

After an ordinary result, one hold validates the exact requester/domain cell
and restores the complete domain and actual engine without allocating an
insertion node. Only after that hold leaves can a confirmed pending pin unpin.
The public owner next clears its exact capability claim and completes its root
operation. Impossible slot/engine/source substitution asserts while the containing
owners still retain their fields; an error code cannot manufacture restoration.
This is not a controller retry interface.

## Phase classification

| Phase | Completion or retained state |
| --- | --- |
| Capability/root admission or initial backend claim rejects | No leaf mutation. Explicitly release unused root admission or newly admitted pin outside subsystem guards. |
| Private mapping work rejects without an installed data prefix | Restore the exact domain/engine, then release the pending pin and public claim/root. Sparse intermediate tables remain charged and cached. |
| Failed installed prefix | Complete backend maintenance before releasing the pending pin. Rejected completion moves it into pre-admitted domain quarantine; the complete domain/engine restore before the error returns. |
| Successful map maintenance | Restore the mapping and actual engine before returning the IOVA and finishing the public claim/root. |
| Initial map maintenance rejects | Existing x86 rollback removes the new mapping and confirms a second maintenance before unpin; rejected rollback quarantines. Arm retains the installed internal mapping/pin without returning an IOVA. |
| Confirmed unmap maintenance | Restore exact domain/engine, release its detached pin after unlock, then complete the public claim/root. |
| Rejected unmap maintenance | Keep detached leaves' pin in pre-admitted quarantine. No allocating mapping reinsertion, duplicate physical release or retry of the consumed unmap. Real domain retirement must complete before that pin releases. |
| Abandonment during a claimed operation | Preserve domain/engine/pending pin and public root/capability claim; the engine and admitted domain slot stay unavailable. No physical work, callbacks, registry/allocator entry or restoration in Drop. |

The rejected-unmap policy now applies coherently on all three backends. It
removes Arm's exceptional reinsertion path. A repeated unmap cannot treat the
absent record as proof that its pin is released; remapping the quarantined object
also rejects. Domain retirement retains its separate hardware completion proof.
Other memory operations still obey the original pin/exclusive-DMA restrictions.

## Execution evidence

The real QEMU NVMe fixture checks successful map, successful unmap and rejected
sparse-prefix cleanup immediately before their actual maintenance. Probes retain
the captured root/capability/memory and entry IRQ state and require backend,
lifecycle, device, CPU-root-table, heap and physical-allocator availability.
Global availability checks retain their one-second bound without yielding or
enabling IRQs.

At each boundary, nested backend create/reset/map/unmap/destroy/early-init calls
reject the claimed engine, including invalid target IDs. Public close and
competing unmap return busy without consuming the capability; exact root close
rejects its active lease. Memory close rejects while the data pin is held.
The ordinary result restores state and completes admission, and full operational
storage tests subsequently run against the same actual unit.

An injected unmap completion rejection leaves charges unchanged and the memory
pin live. Repeated unmap/remap cannot release it; actual acknowledged domain
retirement then allows memory close and restores domain-table accounting. This
injection skips maintenance rather than suppressing a hardware acknowledgement;
it is not an actual hardware-timeout or outstanding-I/O test. Existing prefix
completion-rejection checks remain active.

A separate synthetic containing owner checks ordinary success/error completion
and busy close, then abandons an actual data pin, one published-state table,
heap-backed domain/command metadata and its exact public root/capability claim
under backend/lifecycle/device/CPU-table/heap/physical/pool guards. Its domain ID
and command payload are fixtures, never a real installed hardware domain or
acknowledgement. It retains **one additional original domain-table charge/frame,
two memory data frames and their pin, the exact root operation and original
DMA/memory authority**. User-root backing and memory-object charges are separate
from the table pool. A fresh root cannot reuse that retained root's identity.

With earlier intentional table fixtures, table-only retention totals are
**36 charges/27 frames on VT-d**, **32/23 on AMD-Vi**, and **38/29 on SMMUv3**.
These terminal fixtures cannot be adopted by numeric IDs or addresses.

Selected boundary markers omit unrelated boot data:

```text
[DMA mapping ownership] exact-root/capability busy close and ordinary completion; complete domain/engine/pending-pin Drop under guards retains one domain table, two data frames, metadata, original authority and root
[DMA mapping maintenance] real map/unmap/prefix completion outside backend/lifecycle/device/table guards; exact-root/capability busy close, unit-wide mutation/reset exclusion and rejected-unmap pin retention until real retirement passed
```

## Validation

| QEMU target, four LPs and fresh isolated storage | Authoritative result |
| --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| AMD-Vi, default suite | 15 passed, zero failed/pending; bitmap `0x1bbff` |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; bitmap `0x2003ffff`; security probe `0xffff`, publication generations 1/2, 4,908 cancellation requests retired |

Both x86 runs used kernel SHA-256
`a4e363c420b29ef17430324a8917b897f1472a13e1d1d469702c916f35881d8a`.
Arm used
`eee52f42de8525d6bd12f235160e417f2e5d7e7e06b6ebacb65e2def91f183f0`.
All three emitted both selected mapping markers. No guest validation failed.
An initial Arm launch could not bind QEMU's local forwarding port under the
sandbox; port-authorized execution passed. That was a host launch restriction,
not a guest result. Historical intermittent failures remain in their original
reports; this passing matrix does not establish their cause.

Strict default-feature kernel Clippy passed on both custom targets with
`--locked -- -D warnings`. Rustfmt, whitespace, local documentation links/anchors/
tables and the eighteen-family/seven-gate map passed. Runners reused validated
service bundles, rebuilt kernels and enforced assembly permissions (249 x86
entries and one Arm). Final guest runs were sequential after the last Rust
change and strict Clippy checks. The host harness was not rerun for kernel-only
changes.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance dma-map-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance dma-map-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18669 CATTEN_DEPLOY_HOST_PORT=17969 scripts/run-aarch64.sh --security-test --instance dma-map-arm-20261009 --fresh-storage --timeout 180
```

## Remaining scope

Initial domain creation/configuration and PCI reset still hold their existing
serialization. Its reset owner retains config serialization and disabled bus
mastering through new-domain configuration; releasing those guards needs an
owning reset claim and MMIO/config fencing, not early activation. That broader
boundary is separate from these map/unmap owners.

No actual maintenance timeout, panic unwinding, allocator corruption, concurrent
multi-LP stress, outstanding I/O or physical-device recovery is injected. Existing
private RAM command tests separately qualify timeout queue/epoch preservation.
All production outer IRQ/guard contexts, raw boot adapters, general allocation/
metadata admission and implicit mapping-node construction/destruction still need
G1/G4 review. In particular, this does not claim a general fallible `BTreeMap`
allocator or qualify the internals of memory-object pin admission. Registry-node
destruction, unit shutdown, custody and authorized restart remain open.
No historical intermittent failure's cause is established by this change.
