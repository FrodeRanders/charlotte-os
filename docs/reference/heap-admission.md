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
reclassifies existing heap pages as well as future pages, but cannot reclassify
quarantined backing. There is no userspace setter or signed deployment override
for these physical limits.
The memory-object and [image/runtime](loader-admission.md) pools are separate.
All three pools together are capped at three quarters of usable RAM on normal
machines. They do not account for all physical consumers.

## Ownership and failure

Each owning `AddressSpace` embeds one heap `Account`. It has no per-page
ledger allocations or reference-counted account blocks. A captured generation
is validated before mapping, and the address-space table guard covers
reservation, fallible frame-tracking preparation, frame allocation, mapping and
charge commit. Two first touches cannot install replacement frames or double
charge an already mapped page. A stale handle cannot charge a reused ASID.

`PreparingUserBacking` jointly owns the node reservation, unpublished physical
frame and exclusive address-space borrow. Early return refunds admission only
after confirmed frame release. Rejected release retains a page against the
original domain ceiling and node pool; root destruction and ASID reuse cannot
refund it. An unconfirmed publication retains reachable backing without
deallocation. Frame tracking is preflighted before allocation; successful
insertion therefore allocates nothing. See
[joint preparation](kernel-backing-preparation.md). Page-table allocations
remain a separate, uncharged concern.

Under the frame-allocator guard, heap backing also preserves the existing
one-eighth physical free-frame floor. That check concerns the requested heap
frame. [Private-table preparation](page-table-preparation.md) now also checks
the floor for each root/intermediate frame needed by the mapping. A rejected
mapping can retain a partial owned tree. Image/runtime backing uses the same
data-frame check; shared kernel tables, stacks and other kernel allocations can
still consume the reserve. None of these floor checks charges tables to the
heap pool.

Retirement fences new heap commitment under the mapping guard, without
refunding live pages. `AddressSpace::drop` returns its owned frames before its
embedded account releases aggregate charges. Charges are not returned when a
previous provisional page remains quarantined or any owning-root release fails.
Freeing a Rust allocation inside the userspace arena does not return charges:
committed heap pages stay
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
rollback without exhausting the physical allocator. Joint-preparation fixtures
also check failed release, domain ceilings, mixed teardown, platform pool
identity, uncertain publication and retained charges across reuse. Production
always uses the real architecture mapper.

These are deterministic kernel fixtures plus ordinary service heap faults in
the AArch64 security guest. They are not a new real-EL0 quota probe, a node-wide
pressure soak, forced allocator exhaustion or exhaustive teardown-race proof.
Image/runtime backing has its own admission. User/kernel stacks, page-table
backing, kernel heap and general metadata remain separate SEC-07 work.
