# Snapshot and primitive syscall buffers

Kernel admission capacity for this family comes from the default compile-time
[resource policy](resource-policy.md). Related values can be adjusted together
in a reviewed build profile; see [Configuration](../guides/configuration.md).
Structural/protocol bounds and original-charge/physical-completion rules remain
separate contracts.

CPU mapping, copy-pin and DMA-pin frame snapshots share fixed kernel admission
with bounded upgrade/deployment/shutdown syscall input and plaintext buffers:

| Dimension | Total | Ordinary callers |
| --- | --- | --- |
| Requested vector-buffer bytes | 16 MiB | 12 MiB |
| Buffer reservations | 8192 | 6144 |

A nonempty frame buffer reserves `pages * size_of::<PAddr>()` bytes and one count before
the fallible vector allocator. The per-object frame bound is unchanged. On the
supported targets one maximum-sized snapshot requests 128 KiB. These are
requested buffer layouts, excluding allocator overhead/excess storage, inline
owner fields, mapping nodes and other metadata allocations. Mapping nodes have
separate [requested-layout admission](mapping-node-admission.md).

## Captured policy and lifetime

Every factory receives the exact caller namespace handle captured by its existing
map/copy/DMA operation. Classification is captured once from that handle's trusted
memory-sponsor policy, before counter or allocator entry. A reused ASID cannot
select a successor's reserve classification. Names, manifest roles and callers
cannot choose reserve; later promotion cannot reclassify an existing reservation
or its refund. Borrowed backing does not substitute its owner's policy for the
calling namespace's policy. This adds no per-principal ceiling or guaranteed share
for one platform service.

The typed `Snapshot` retains vector storage and original admission together in
inert fields. Preparation cancellation and ordinary disposal free the buffer
before refunding the charge. Allocation rejection returns its original unused
reservation; quota rejection returns `ResourceLimit` before the allocator. An
allocator rejection returns `FrameAllocFailed`. Existing backend conversions and
public ABI remain unchanged.

Publication moves that same buffer owner into `CopyPin`/`DmaPin`, or leaves it in
the existing mapping preparation. It does not reserve again or enter the counter.
Abandonment retains allocation and original charge; root retirement and ASID reuse
do not refund or redirect it. An interrupted reservation-only or post-free
bookkeeping phase retains its reservation rather than inventing completion. No
recovery custody or replay owner is added.

Explicit copy/DMA unpin frees the snapshot and refunds its charge before releasing
the independent backing pin. Ordinary CPU mapping disposal ends its snapshot
allocation before the original mapping pin completes invalidation/scratch work.
Snapshot refund therefore supplies no backing or hardware completion proof. Failed
or uncertain physical work still retains its original complete owner and charges.

Empty source-escrow pins have no vector allocation and consume no buffer quota.
Their existing source pin/fence remains required. This is a distinct current
purpose, not an uncharged allocator or compatibility path. Zero-size snapshot
preparation rejects; it never supplies an empty fallback for an ordinary snapshot.

The counter is an independent leaf mutex released before allocation. Explicit
refund follows buffer deallocation after allocator return; abandonment enters no
allocator, registry or counter. Ordinary containing cleanup still requires its
qualified post-guard context.

## Primitive syscall inputs

`syscall::snapshot_buffers` captures trusted storage class from the exact
already-authorized manager/agent generation, before buffer allocation. The
executing caller's current thread retains its root; the class token borrows that
identity and grants no root or teardown authority. The user-thread `StackSlot`
retains an `AddressSpaceOperation` until qualified off-CPU stack retirement;
no additional root lease is needed just to own these private bytes. Copying
rechecks the captured
generation and uses `memory::object::snapshot_bytes_in`, retaining its shared
read-rights, object-state and length checks under the memory-object registry.
Only completed private copies reach signature/protocol validation. Promotion,
root retirement and ASID reuse cannot redirect their original byte charges.

`ByteAllocator<SnapshotBuffers>` admits requested bytes and **zero buffer counts**
before allocation. The frame-catalog count contract is unchanged. Scoped image
and descriptor copies share one `ScopedInputs`; pickup and private plaintext
share one `OperationalInputs`. Failure disposes the complete private prefix
after the copying helper's registry guard has left. Public handlers retain their
existing status and source-cap consumption rules. Their signature, freshness,
launch reservation and publication checks still run unchanged. These primitive
byte owners free on ordinary local Drop; they contain no active physical
cleanup, callback, capability element or authority. This does not qualify the
surrounding supervisor launch/upgrade transactions or arbitrary enclosing callers.

The plaintext owner allocates once before decryption and cannot reallocate after
initialization. It exposes borrowed slices and erases initialized and spare
capacity before vector deallocation, which precedes original-class refund.
Successful output, authentication failure and later profile/launch rejection
retain that same owner. Forced termination/abort and other secret copies remain
separate [SEC-15 limitations](secret-state-lifetimes.md).

The default bytes increased from 3/4 to 12/16 MiB ordinary/total so existing
protocol maxima remain usable. A maximum image plus descriptor needs
4194304 + 3584 bytes; a maximum pickup plus plaintext needs 4269180 + 65536 bytes.
Two pickup/plaintext pairs plus the former 3 MiB ordinary frame allowance need
11815160 bytes, leaving 767752 bytes within the new ordinary ceiling before
other retained usage. This permits an extra 9/12 MiB of ordinary/total heap
retention; it allocates no backing in advance. The minimum configured kernel
heap is 8 MiB initially plus 64 MiB growth reserve, shared with other families.
This pool can therefore consume 16 MiB of that 72 MiB envelope; allocator
overhead and concurrent family usage need the aggregate Q4 capacity plan.
No new pool, SDK format or service bundle is needed. Keeping the old limit
would deny legal inputs; splitting parsers would add lifetime/validation
complexity without solving aggregate admission. Shared competition and saturated
cleanup/management progress remain Q7/Q8 acceptance work, even with more bytes.
Reviewed profiles still require their own sizing and workload qualification.

## Acceptance scope

The [audit evidence](../reports/audits/2026-10-25-security-snapshot-buffer-admission.md)
qualifies real byte/count exhaustion, trusted reserve, rejection before allocation
and pin publication, buffer-free-before-refund order, generation reuse, late
promotion and charged guarded abandonment.

The [six-site Q1 record](../reports/audits/2026-11-04-syscall-snapshot-admission.md)
separately records primitive byte controls and the combined final campaign.
The remaining default-allocator image snapshot in `service::store` belongs to Q2
because it can enter the persistent cache; it is not covered by these temporary
owners. Primitive admission failures preserve the handlers' original generic
failure statuses; they do not add a new public quota status.

| Audit gate | Added evidence | Remaining acceptance work |
| --- | --- | --- |
| R7-2 | This snapshot family rejects quota/allocation failure before publication and retains its original allocation owner. | Other metadata families and the complete application-triggerable allocation inventory. |
| R7-3 | One shared requested-byte/count ceiling covers concurrent and retained snapshot buffers, including across root retirement. | Aggregate signed-principal snapshot/general-heap admission and allocator overhead. |
| R7-4/R7-5 | Serialized actual-buffer pressure, trusted reserve and teardown at exhaustion. | Sustained concurrent EL0 pressure with management, timer/scheduler and teardown latency/progress. |
| R18/G1 | Existing typed owner/physical-proof boundaries remain; publication/fallback contexts are guarded controls. | Broader caller/destructor/IRQ contexts, concurrent hardware failures and authorized recovery custody. |

These are partial gate advances. SEC-07/18 and the finding ledger remain unchanged;
bounded pools and passing guests do not establish fairness or production readiness.
