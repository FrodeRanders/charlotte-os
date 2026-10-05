# Demand-backed heap admission

The runtime heap's virtual capacity is a launch claim, not a physical allocation.
First touch commits a zeroed 4 KiB page. The kernel now admits that backing
before allocating a frame, on both AArch64 and x86-64.

| Scope | Physical heap backing ceiling |
| --- | --- |
| One address-space lifetime | Fixed heap-window maximum: `HEAP_VA_LIMIT / 4096` (1,264 pages); accessible pages are also limited by its configured virtual capacity. |
| Node total | One quarter of usable RAM, rounded down to pages (minimum one). |
| Ordinary domains | Three quarters of the node heap pool. |

The remaining quarter of the heap pool is shared platform headroom, not a
per-service guarantee. Only trusted kernel launch policy promotes an account;
application names, roles and manifests cannot request the reserve. Promotion
reclassifies the account's existing heap pages as well as future pages. There
is no userspace setter or signed deployment override for these physical limits.
The memory-object pool is separate; heap and memory-object backing together
are capped at half of usable RAM on normal machines. Neither pool accounts for
all physical consumers.

## Ownership and failure

Each owning `AddressSpace` embeds one heap `Account`. It has no per-page
ledger allocations or reference-counted account blocks. A captured generation
is validated before mapping, and the address-space table guard covers
reservation, fallible frame-tracking preparation, frame allocation, mapping and
charge commit. Two first touches cannot install replacement frames or double
charge an already mapped page. A stale handle cannot charge a reused ASID.

A provisional `PageCharge` owns node admission. `PreparingHeapFrame` owns the
unpublished physical frame. Early return frees the frame before refunding
admission. The owned-frame vector reserves its tracking slot before frame
allocation; successful insertion therefore allocates nothing. The mapping's
page-table allocations remain a separate, uncharged concern.

Under the frame-allocator guard, heap backing also preserves the existing
one-eighth physical free-frame floor. That check concerns the requested heap
frame, not every page-table allocation needed to map it. Loader, stack,
page-table and kernel allocations can still consume that floor.

Retirement fences new heap commitment under the mapping guard, without
refunding live pages. `AddressSpace::drop` returns its owned frames before its
embedded account releases aggregate charges. Charges are not returned when a
Rust allocation is freed inside the userspace arena: committed heap pages stay
backed until domain teardown. Borrowed page-table snapshots own no heap charge.
This adds no unmap/decommit API.

Heap commitment still returns failure to the architecture's fatal user-fault
path when quota, frame tracking, frame allocation or mapping fails. It does not
provide a recoverable userspace allocation-error ABI. Fallible tracking and
owned rollback do not make every page-table allocator path OOM-safe.

## Verification scope

Synchronous kernel fixtures check isolated ordinary/node pool saturation and
platform headroom, provisional charge cancellation, account promotion and
retirement, zeroed real backing, a one-page domain quota, repeated touch,
exact ASID reuse and charge release after teardown. A kernel-only mapper
adapter rejects before leaf publication to verify real-frame and charge
rollback without exhausting the physical allocator. Production always uses
the real architecture mapper.

These are deterministic kernel fixtures plus ordinary service heap faults in
the AArch64 security guest. They are not a new real-EL0 quota probe, a node-wide
pressure soak, forced allocator exhaustion or exhaustive teardown-race proof.
Loader/runtime pages, user/kernel stacks, page-table backing, kernel heap and
general metadata remain separate SEC-07 work.
