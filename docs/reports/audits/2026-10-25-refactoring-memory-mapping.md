# Audit follow-up: memory mapping boundary

Date: 2026-10-25. Reviewed baseline: `7b66b441`.

This follows the [IPC close consolidation](2026-10-25-refactoring-ipc-close.md).
Memory-object map/map-any admission, page installation, rollback, unmap and
invalidation now live in
[`object/mapping.rs`](../../../crates/catten/src/memory/object/mapping.rs),
alongside admitted storage and existing fixtures. The object registry falls
from 1806 to 1405 lines. Shared retirement pins, mapping records, namespace/loan
retirement, backing custody and their original guards remain in place.

## Boundary and preserved policy

Installation-only helpers are private to mapping and its children. The two
existing retirement-fixture injection hooks retain object-family scope;
storage preparation/types/release retain their original object-family scope
for namespace and loan retirement. Current callers use the canonical
`mapping::storage` path; no retired module alias is retained. Root public
`map`, `map_any` and `unmap` reexport the same implementations.

Caller operation leases still retain the exact address-space generation across
scratch reservation, admission, page-table work and invalidation. Preparation
captures the same root/object, admits its node and snapshot outside memory/table
guards, then rechecks identity, retirement, rights, loans and pins before
allocation-free snapshot fill, record publication and pin acquisition. Captured
trusted classification and original layout/snapshot charges are unchanged.

Preflight rejection still releases unused scratch without maintenance when no
pin was published. Successful installation and rejected/rolled-back admitted
prefixes still invalidate through the same finish path. Failed detach,
invalidation or scratch release keeps the original mapping/backing fence and
original admission; absence of a leaf or an empty prefix grants no replay or
physical cleanup authority. Confirmed unmap retains its pin through actual node
disposal before unpin/backing release. The same foreign-leaf checks, bounded
frame batches and per-call invalidation causes remain.

The storage admission implementation, original record/layout, counter and
allocation lifetimes are unchanged. Unused preparation still finishes after
local guards; abandonment retains its original fields. Mapping pin/retirement,
namespace and revocation contexts remain separately qualified; relocation does
not grant a new root/pin, recovery custody, physical completion or arbitrary
outer IRQ/guard/destructor proof. C05/C18/G1 remains qualification work.
Configured limits, public register/wire ABI, userspace and model inputs are
unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-memory-mapping.json) records frozen
source/artifact hashes, comparisons and native executions. All map admission,
publication, rollback, finish, unmap and loan-predicate bodies match after scoped
visibility, qualified memory paths and module routing. Storage construction,
publication and explicit release match after restoring their object-family
scope at the new depth. Remaining registry, backing, transfer, close, pin and
retirement code matches after the facade/storage path changes.

The storage fixture and admission implementation move byte for byte. The map
contract fixture only restores its original family visibility; the admission
fixture only updates its ancestor-scoped visibility path. Retirement fixtures
only import the same two injection hooks and call the relocated map contract
fixture at the same position. Assertions and order remain. These comparisons
aid review, not formal refinement.

Strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`. No mirror test or
alternate implementation is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs pass 256 pin-free denials without maintenance, admitted success and
rolled-back-prefix invalidation, per-call failure-cause retention, 192 unlocked
record/snapshot prefix rejections, 64 heap-held publications/detachments and
32 rollback receipts through confirmed maintenance. Real ASID reuse rejects
stale preparation. Guarded uncertain rollback retains two data pages, its
mapping node/root and original charge; other partial/full preparation controls
retain their admitted empty nodes/snapshots without quota resets. Actual node
pressure reaches ordinary/total exhaustion, preserves trusted reserve and
reconciles baseline after public unmap and generation reuse. These scoped
controls do not establish sustained aggregate pressure or progress guarantees.
Existing retirement and DMA CPU-remap fencing also pass. Both runners finish
with `reason=complete`; signed bundles are reused with unchanged userspace
sources. Formatting, whitespace, frozen Rust hashes and local references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. x86_64 also records **one TimeoutError retry** before its
successful initial metrics request. All nine diagnostics and eventual passing
probe results are preserved separately in the evidence. They are retried
attempts within these runs, not separate clean executions. No controlled
comparison attributes them to this refactoring, host scheduling or historical
failures.

No finding status changes. SEC-07/18 remain partial; the ledger is **29 corrected
/ 6 partial / 3 open**. Historical scheduler, metrics, root-close and mapping
failures remain unattributed. Broader pressure, fairness and outer-context
qualification remain open.
