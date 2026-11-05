# Q4 shared heap capacity baseline

Baseline `d1cf898b`, after Q3 acceptance. **Q4 is in progress, not accepted.**
This checkpoint establishes a reproducible sizing baseline and the next design
decisions. It changes no policy, resource limit or runtime admission contract.
The [evidence](evidence/2026-11-05-q4-capacity-baseline.json) contains actual
compiled policy values, canonical catalog units, source hashes and boot excerpts.

## What the current capacities mean

The default catalog has 24 metadata pools. Nineteen have a requested-byte
dimension; their independent ceilings sum to **205.5 MiB total and
154.125 MiB ordinary**. Five account/list pools count bundles or allocations,
not bytes. Their control backing is not converted into a byte estimate here.
Mailbox ring and CQ backing ceilings remain separate (8/6 MiB and 4/3 MiB
total/ordinary). Do not add these as disjoint usage without reviewing overlap.

The same final Q3 native campaigns, with 512 MiB configured guest RAM, logged:

| Executed profile | Discovered usable RAM, rounded log | Mapped kernel heap |
| --- | --- | --- |
| ARM SMMUv3 security | 378 MiB | 8 MiB initial + 64 MiB reserve = 72 MiB |
| Intel VT-d security | 367 MiB | 8 MiB initial + 64 MiB reserve = 72 MiB |
| AMD-Vi security | 367 MiB | 8 MiB initial + 64 MiB reserve = 72 MiB |

The [allocator](../../../crates/catten/src/memory/allocators/global_allocator.rs)
maps the whole reserve at boot. Later OOM extension claims within that mapping,
avoiding allocator-to-page-table/frame-lock recursion. It cannot acquire another
heap mapping beyond the reserve at runtime. These boot excerpts establish the
configured envelope, not peak heap use, fragmentation or saturation behavior.

The actual default formula, evaluated on a 64-bit host, gives:

| Usable RAM input, not configured guest RAM | Growth reserve | Total mapped heap |
| --- | --- | --- |
| 512 MiB | 64 MiB | 72 MiB |
| 1 GiB | 64 MiB | 72 MiB |
| 4 GiB | 64 MiB | 72 MiB |
| 8 GiB | 128 MiB | 136 MiB |
| 16 GiB | 256 MiB | 264 MiB |

Increasing guest RAM does not necessarily increase this heap. Conversely, the
metadata sums are ceilings, not measured usage, disjoint preallocations or proof
that all maxima are jointly reachable: count constraints, physical admission,
shared owners and other pools can bind first. The discrepancy establishes that
local byte ceilings alone do not form a sufficient shared-heap capacity proof.
It does not establish that the passing workloads exhausted the allocator.

## Capacity choices and required evidence

| Choice | Benefit | Cost or remaining obligation |
| --- | --- | --- |
| Keep the current 64 MiB growth floor and add coherent shared admission | Predictable aggregate rejection within a smaller physical footprint | Needs measured boot/control/fragmentation margins and legitimate concurrent working-set qualification; accepted cleanup still needs reserved progress capacity. |
| Raise the growth floor to 96 MiB | Adds 32 MiB of kernel working room; total heap 104 MiB | Maps another 32 MiB permanently, reducing physical headroom for applications and other backing. Still needs original-principal and aggregate admission. |
| Raise the growth floor to 128 MiB | Adds 64 MiB of kernel working room; total heap 136 MiB | Maps another 64 MiB permanently; same accounting/progress obligations. Test small-RAM boot and real concurrency before choosing it. |
| Premap the sum of every independent byte maximum | Simple conservative treatment of those maxima | Not a complete bound: count-only controls, allocator overhead, boot and overlap remain. Potentially wastes scarce guest RAM for maxima that cannot coexist. No recommendation to do this blindly. |

No smaller input/resource limit is selected to make this arithmetic fit. A modest
heap increase remains a valid option, alongside a documented supported guest-RAM
profile. Choose it using peak allocator backing and fragmentation measurements
for a defined concurrent workload, not the sum alone. Any shared gate must
account for old/candidate overlap, weak/private holders, boot peaks, restart and
quarantine, and preserve original sponsorship until actual deallocation.

Q4's first concrete coverage corrections remain diagnostic working buffers and
heap/image frame-tracking vector capacity. Reuse typed allocation/counter
mechanics and the existing boot-lifetime principal record; do not invent a
second principal registry or infer sponsorship from the current worker/ASID.
The existing per-family accounting also needs its wider principal/overhead
review. Q5's caller/IRQ/Drop review and Q7's cleanup-progress gate remain separate.

## Reproduction and controls

Run `python3 scripts/audit-resource-capacity.py` for the default policy, or pass
`--policy crates/catten/src/resource_policy/larger.rs`. The helper compiles the
actual policy and full syscall catalog with the repository-pinned Rust compiler
and `-D warnings`; it copies no numerical defaults into its harness. Repeated
`--usable-mib` inputs evaluate the actual heap formula; `--output` saves JSON.

Default and larger profiles compiled successfully. Only the larger profile's
three intended pools changed, with all dimensions scaled exactly 5/4. A 4097 MiB
usable input rounded to a 66 MiB reserve; the largest integral-MiB u64 byte input
clamped to 256 MiB. Overflow input rejected before compilation, and an invalid
ordinary/total policy failed the real compile-time validator without JSON output.
No new native run was needed for this read-only census; reused Q3 boot excerpts
are explicitly identified. No Q4 pressure acceptance is claimed.
