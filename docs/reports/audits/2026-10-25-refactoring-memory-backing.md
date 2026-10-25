# Audit follow-up: memory-object backing boundary

Date: 2026-10-25. Reviewed baseline: `28b84323`.

This follows the [CQ readiness consolidation](2026-10-25-refactoring-cq-readiness.md).
Charged frame allocation, physical release and complete retired backing now
live in [`object/backing.rs`](../../../crates/catten/src/memory/object/backing.rs).
The object registry falls from 2032 to 1806 lines. Mapping, pins, loans,
capability/namespace retirement and their guard boundaries remain in the
existing modules.

## Boundary and preserved policy

`ChargedFrames` keeps its frame vector, original charge, unused object storage,
complete retired node and started/failed fence private. Its `Deref`/`DerefMut`
payload exposure is removed. Existing allocation and copy callers use the
associated allocation method, a borrowed prepared-frame slice and the same
original-charge admission predicate. These methods add no allocation, lookup,
guard, root, reservation or owner instance. Source/target generation validation
and capability publication remain in their original transactions.

`RetiredMemory` now owns private backing and metadata in the same module. A
consuming constructor only assembles those existing fields at the former
literal-construction point after registry unlock. The type and its consuming
release remain reexported through `memory::object`; no compatibility allocation
helper remains. Its observer seam retains object-family scope for the existing
retirement fixtures.

Admission still precedes node/vector/frame allocation. Partial construction
retains the original owner through ordinary rollback. Physical retirement
still records its fence before the first attempt and consumes identities in
the same sixteen-frame batches before invoking the allocator. Failure or
interruption retains the complete private/published fields and original charge;
no later Drop or release may replay attempted identities. An empty frame vector
does not prove completion. Successful physical release disposes borrower and
authority metadata, object storage and frame-vector backing before refunding
original admission in the same order.

`ChargedFrames::Drop` remains active, including physical allocator entry and
failure logging. `RetiredMemory::Drop` still retains its complete backing.
Relocation does not qualify broader caller/IRQ/destructor contexts, introduce
recovery custody or authorize physical retry. C18/G1 remains qualification work.
Configured limits, register/wire ABI and userspace sources are unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-memory-backing.json) records frozen
source/artifact hashes, comparisons and native executions. Allocation,
physical-release, explicit retirement and Drop bodies match after scoped
visibility, qualified memory paths, owner-method routing and explicit private
payload access replacing dereference access. Read-only methods return the
original fields/predicate; the retired constructor matches former field assembly.
Remaining registry/mapping/pin/retirement code and transfer code match after
those constructor/accessor changes. These comparisons aid review, not formal
refinement.

The storage fixture moves beneath the owner with the same assertions and order;
only allocation-method routing and private-payload inspection change. Its
physical-release injection stays private to that module. Existing mapping and
retirement fixtures keep their code and hook order. Initial strict compilation
caught two remaining direct field accesses (`E0609`): frame insertion inside
allocation and the copied-backing admission check. They now use the private
payload and unchanged admission predicate respectively. Final strict base-kernel
Clippy passes for AArch64 and x86_64 with signed bundle inputs and
`--no-default-features --features acpi -- -D warnings`. No mirror test or
alternate implementation is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs pass 128 charged preparation rejections, 64 copied-prefix rejections,
64 heap-held object relinks/detaches, physical-before-node-before-charge disposal,
and terminal physical-release rejection across exact generation reuse. The
injection deliberately retains one data page, the complete object node and
original charge; the next release/Drop does not replay it. This is an injected
rejection, not a physical allocator exhaustion measurement. Existing retirement,
last-unpin, hidden copy/loan, capability preparation, copy/DMA snapshot and
original-sponsor accounting controls also pass. Both runners finish with
`reason=complete`. Signed bundles are reused with unchanged userspace sources.
Formatting, whitespace, frozen Rust hashes and local references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. AArch64 also records **one TimeoutError retry** before its
successful initial metrics request. All nine diagnostics and eventual passing
probe results are preserved separately in the evidence. They are retried
attempts within these runs, not separate clean executions. No controlled
comparison attributes them to this refactoring, host scheduling or historical
failures.

No model input or finding status changes. SEC-07/18 remain partial; the ledger
is **29 corrected / 6 partial / 3 open**. Historical scheduler, metrics,
root-close and mapping failures remain unattributed. Broader pressure, fairness
and outer-context qualification remain open.
