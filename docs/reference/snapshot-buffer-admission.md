# Mapping, copy and DMA snapshot buffers

Kernel admission capacity for this family comes from the default compile-time
[resource policy](resource-policy.md). Related values can be adjusted together
in a reviewed build profile; see [Configuration](../guides/configuration.md).
Structural/protocol bounds and original-charge/physical-completion rules remain
separate contracts.

CPU mapping, copy-pin and DMA-pin frame snapshots share fixed kernel admission:

| Dimension | Total | Ordinary callers |
| --- | --- | --- |
| Requested vector-buffer bytes | 4 MiB | 3 MiB |
| Buffer reservations | 8192 | 6144 |

A nonempty buffer reserves `pages * size_of::<PAddr>()` bytes and one count before
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

## Acceptance scope

The [audit evidence](../reports/audits/2026-10-25-security-snapshot-buffer-admission.md)
qualifies real byte/count exhaustion, trusted reserve, rejection before allocation
and pin publication, buffer-free-before-refund order, generation reuse, late
promotion and charged guarded abandonment.

| Audit gate | Added evidence | Remaining acceptance work |
| --- | --- | --- |
| R7-2 | This snapshot family rejects quota/allocation failure before publication and retains its original allocation owner. | Other metadata families and the complete application-triggerable allocation inventory. |
| R7-3 | One shared requested-byte/count ceiling covers concurrent and retained snapshot buffers, including across root retirement. | Aggregate signed-principal snapshot/general-heap admission and allocator overhead. |
| R7-4/R7-5 | Serialized actual-buffer pressure, trusted reserve and teardown at exhaustion. | Sustained concurrent EL0 pressure with management, timer/scheduler and teardown latency/progress. |
| R18/G1 | Existing typed owner/physical-proof boundaries remain; publication/fallback contexts are guarded controls. | Broader caller/destructor/IRQ contexts, concurrent hardware failures and authorized recovery custody. |

These are partial gate advances. SEC-07/18 and the finding ledger remain unchanged;
bounded pools and passing guests do not establish fairness or production readiness.
