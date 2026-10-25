# Audit follow-up: mapping retirement custody

Date: 2026-10-25. Reviewed baseline: `83d6d82b`.

This follows the [mapping preparation consolidation](2026-10-25-refactoring-memory-mapping.md).
The shared mapping pin and complete mapping-retirement receipt now live in
[`object/retirement.rs`](../../../crates/catten/src/memory/object/retirement.rs).
The object registry falls from 1405 to 1158 lines. Namespace, loan and ordinary
unmap transactions still retain these same owners and original guards.

## Boundary and preserved policy

`MappingRetirementPin` keeps its captured object identity and detached mapping
node private. Object-family callers read identity/record metadata through
borrowed accessors and consume the same optional detached node into its owner
at the former assignment point. The setter performs that same assignment; it
adds no lookup, allocation, assertion, admission, guard or retry policy. The
original registry guard remains through custody publication. That pin-owned
node still disposes before pin decrement and any backing release.

`RetiredObjectMappings` keeps its pin, page count, complete records/peer leases
and detach state private. Namespace publication consumes the same existing
fields into its constructor, which sets the original `detached=false` state.
It cannot set the detached flag directly. Identity inspection borrows scalar
metadata; it supplies no new root lease, registry lookup or cleanup authority.
The shared `RetiredMappings` variants retain object-family scope for existing
guarded namespace assembly; they remain part of this complete receipt.

Pin acquisition still increments the exact object's fence under the memory
registry. Bounded frame batches borrow physical identities while that pin
retains immutable backing. Leaf walks leave the memory guard; detach records
the same aggregate outcome. Finish invalidates the same mappings, requires both
detach and invalidation success, then releases applicable scratch. Only that
confirmed path consumes the pin, completing its existing backing release before
final mapping-map node and peer-lease disposal. Earlier detach/invalidation/
scratch failure and abandonment retain their complete receipts; a later backing
release error retains its existing terminal backing policy. Original admission
follows its respective physical backing or metadata allocation. Neither metadata,
a missing leaf, an empty prefix nor an error diagnostic authorizes replay or
physical release.

The existing pin Drop remains non-releasing; receipt field destruction retains
its original node/peer custody. Containing namespace/revocation/backing paths
keep their existing active disposal and context requirements. Relocation does
not qualify arbitrary IRQ/guard/destructor contexts, add recovery custody or
change C05/C06/C11/C18/G1 status. Limits, register/wire ABI, userspace and model
inputs are unchanged.

## Verification

The [evidence](evidence/2026-10-25-refactoring-mapping-retirement.json) records
frozen source/artifact hashes, comparisons and native executions. All moved
acquire/unmap/detach/invalidation/scratch/release/Drop bodies match after family
visibility and module facades. New accessors read the original fields, node
custody performs the same assignment, and receipt construction matches former
field assembly. Namespace, revocation, ordinary map/unmap and storage-fixture
callers match after those changes, with original guards and statement positions.
Remaining registry/copy/DMA/close/backing code matches after extraction/plumbing.
These comparisons aid review, not formal refinement.

The retirement fixture moves byte for byte; its root kernel fixture facade and
hook order remain. Storage fixture assertions/order remain after read-only and
node-custody routing. Initial strict compilation caught two missed direct field
accesses (`E0616`): a mapping rollback assignment and a storage-fixture identity
inspection. They now use the same node setter and metadata accessor. Final
strict base-kernel Clippy passes for AArch64 and x86_64 with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`. No mirror test or
alternate implementation is introduced.

Four-LP native results on the frozen source, run sequentially:

| Architecture / IOMMU | Deferred passed / failed / pending | HTTP checks | Security publication generations |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 1 and 2 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 1 and 2 |

Both runs pass bounded lock-separated retirement, last-unpin fencing, borrower
authority, scratch rejection, partial detach, failed invalidation, Drop
quarantine and foreign-leaf preservation. The fixture deliberately retains
twelve data pages for the guest lifetime, without re-adopting them. Mapping
storage and preparation controls pass 32 rollback receipts through confirmed
maintenance and guarded uncertain rollback with original node/root/charge
retention. Namespace controls pass stale-peer rejection, mapped-peer retention
outside lifecycle, preparation rollback, closing peers and failure/Drop
quarantine. Runtime mapped-peer retirement with secondary LPs online, DMA-loan
preservation and CPU-remap fencing also pass. These scoped controls do not
establish aggregate pressure or progress guarantees. Both runners finish with
`reason=complete`; signed bundles are reused with unchanged userspace sources.
Formatting, whitespace, frozen Rust hashes and local references pass.

Each architecture records **four ConnectionResetError retries**: one before
successful EOF recovery, one before abortive-peer recovery and two before
idle-client recovery. All eight diagnostics and eventual passing probe results
are preserved separately in the evidence. They are retried attempts within
these runs, not separate clean executions. No controlled comparison attributes
them to this refactoring, host scheduling or historical failures.

No finding status changes. SEC-07/18 remain partial; the ledger is **29 corrected
/ 6 partial / 3 open**. Historical scheduler, metrics, root-close and mapping
failures remain unattributed. Broader pressure, fairness and outer-context
qualification remain open.
