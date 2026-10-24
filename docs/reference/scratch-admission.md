# Scratch reservation admission and completion

Memory objects and MMIO share a 512 MiB virtual scratch window in each user
address-space lifetime. This is virtual space, not a physical allocation.
Reservations become reusable only after their owning mapping/close operation
confirms leaf removal and invalidation. Public callers retain an exact root
operation or borrowed closing ownership; raw boot fixtures retain their existing
serialization. The scratch allocator's captured handle is identity, not a new
root lease or physical completion proof.

## Prepared namespace and extent nodes

The existing global scratch registry and each window's sorted live-extent map
use shared admitted nodes. Preflight validates nonzero page-aligned size, window
bounds, checked arithmetic and current first-fit availability. One preparation
then owns a fallible extent node and namespace candidate outside the scratch
registry. Final publication checks the exact captured root generation again,
recomputes first fit under the same hold and only relinks prepared storage.
A competing namespace keeps the losing candidate in that original owner until
explicit post-guard disposal. A different generation rejects without reset or
replacement. Ordinary rejected preparation disposes its complete prefix after
local guards; abandonment retains all partial fields without cleanup.

There is no allocating namespace `BTreeMap` insertion, extent-vector growth or
vector shifting. Each live reservation owns one extent node; gaps combine
implicitly. Exact release only detaches the original node under the scratch
registry and explicitly frees it after that guard. Neither publication nor
detachment allocates or deallocates. First-fit lookup and ordered insertion are
linear; the 131,072-page window and existing resource limits do not qualify
worst-case hold time, concurrent preparation pressure or comprehensive metadata
byte/principal admission. Nodes add allocator overhead rather than a vector
high-water capacity. There is no compatibility-only old allocation path.

MMIO scratch preparation also leaves `DEVICES`, with its existing exact-root
lease and claimed descriptor retained. Claim admission rechecks mapped/unmapped
expectation atomically with the in-flight bit; an earlier preliminary probe
cannot authorize a claim after another completed operation changed that state.
The same physical descriptor, pages and claim revalidate after preparation.
Ordinary map/unmap/close hardware and uncertainty policies are unchanged.

## Exact completion and whole-root retirement

Release must name the offset and size of one exact live reservation. Partial,
combined, unknown and already released extents reject without mutation. The
namespace records the full address-space handle; stale publication, captured
release and detachment cannot alter a successor. Range matching remains **not**
an allocation nonce: an invalid stale scalar pair within the same generation
could match a later reservation at the same address. Current callers must keep
the original mapping/close owner and proof; this check grants no replay authority.

Ordinary unmap and rejected-prefix rollback retain their original backing pin
on failed scratch completion. Bulk retirement waits for all invalidations,
attempts surviving domains' completions and releases its pin/loan restrictions
only after all required completions succeed. Individually completed ranges may
be reused; rejected ranges remain reserved. MMIO uncertainty retains its existing
mapping/range/root/claim owner and never restores or retries started cleanup.

Sealed whole-root cleanup detaches the complete scratch namespace and its extent
nodes into the existing `RetiredMetadata`, retained by `ClosingAddressSpace` and
`RetiredAddressSpace`. Final invalidation rejection/abandonment keeps those nodes
with the original complete root, slot, backing and charges. Only confirmed final
quiescence permits explicit extent/node disposal after lifecycle/scratch/local
registry guards, before root backing/slot completion. This extends the existing
root receipt, without a new recovery family, registry, metadata ticket or retry
policy. Uncertain mapping/device work still prevents sealing; scratch absence
or generation checks cannot substitute for its physical completion proof.

## Verification and limits

The standalone host runner reuses the exact production admitted collection:
**six scratch cases plus sixteen shared collection cases** pass. Scratch checks
cover first fit, implicit gaps, exact release rejection, missing preparation,
invalid/exhausted/overflow requests, independent windows and a 6,000-operation
bitmap oracle. Shared collection controls trace allocation-free relinking,
inert abandonment and actual node-disposal ordering. They do not force global
allocator OOM or prove cross-LP fairness.

Native controls cover 192 preparation-prefix rejections, 64 heap-held
publications/detachments, competing preparation, stale-generation publication/
release/detachment, full/invalid rejection and guarded partial/retired retention.
Whole-root controls retain a real namespace after rejected final invalidation,
release it through the existing bounded custody path after confirmation, preserve
a reused successor and retain another namespace during guarded root-attempt
abandonment. Existing MMIO controls check factory placement and both claim-state
rejections; ordinary native MMIO/memory mappings remain enabled.

See [scratch storage evidence](../reports/audits/2026-10-25-security-scratch-storage.md) and
[memory-object retirement](memory-object-retirement.md). General metadata-byte/
principal admission, scalar reservation identity, serialized fixture disposal,
other factory/destructor/outer IRQ contexts, sustained pressure and historical
failure causes remain separate. No physical/replay proof is added.
