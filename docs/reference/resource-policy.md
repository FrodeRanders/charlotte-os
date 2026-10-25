# Kernel resource policy

Kernel capacity is selected centrally by
[resource_policy/defaults.rs](../../crates/catten/src/resource_policy/defaults.rs)
and the optional absolute-file `CATTEN_RESOURCE_POLICY` build input. See the
[Configuration guide](../guides/configuration.md) and manual chapter for the
operator procedure, authority, planning and qualification process.

[Policy](../../crates/catten/src/resource_policy.rs) groups typed, copyable
immutable settings. Family-local constant aliases forward to those fields;
existing reservation helpers still enforce admission at their owning boundary.
Six metadata families share the checked `AdmissionPool` counter mechanics:
root metadata, principal history, thread metadata, scratch nodes, snapshot
buffers and mapping nodes. Each retains its own lock, typed charge, captured
classification and allocation-lifetime release policy. The shared counter
owns no resource and grants no cleanup authority; both counters change together
on accepted admission/refund and remain unchanged on rejection.
No new account, registry, classification source or cleanup owner is introduced.
All numerical defaults below are preserved. They are defaults, not fixed
application requirements or workload sizing guarantees.

## Fixed-pool inventory

Counts and requested bytes are independent dimensions. Multi-value cells label
each quantity and separate dimensions with semicolons. Integer counts have no
thousands separators. No comma is used as a decimal separator in this inventory.
A missing domain limit means that this pool has no per-domain dimension, not
unlimited node capacity.

| Policy field | Dimensions | Domain default | Ordinary default | Total default |
| --- | --- | --- | --- | --- |
| `capabilities` | records | 4096 | 49152 | 65536 |
| `completions` | records | 1024 | 6144 | 8192 |
| `timers` | events | 1024 | 6144 | 8192 |
| `watches` | endpoint/thread/callback entries | 1024 | 6144 | 8192 |
| `waiters` | scheduler entries | 1024 | 6144 | 8192 |
| `observer_lists` | allocation lifetimes | — | 6144 | 8192 |
| `mailbox_records` | records | 512 | 6144 | 8192 |
| `mailbox_queues` | sets; ring bytes | 2 sets; 1 MiB | 768 sets; 6 MiB | 1024 sets; 8 MiB |
| `ipc_records` | connections, pending calls, replies | 512 each | 6144 each | 8192 each |
| `endpoints` | endpoints; queue slots | 64 endpoints; 8192 slots | 768 endpoints; 24576 slots | 1024 endpoints; 32768 slots |
| `completion_queues` | queues; backing bytes | 32 queues; 256 KiB | 1536 queues; 3 MiB | 2048 queues; 4 MiB |
| `root_metadata` | node-layout bytes; nodes | — | 3 MiB; 9216 nodes | 4 MiB; 12288 nodes |
| `principal_history` | node-layout bytes; nodes | — | 768 KiB; 3072 nodes | 1 MiB; 4096 nodes |
| `thread_metadata` | context/node bytes; bundles | principal 1 MiB; 128 bundles | 6 MiB; 3072 bundles | 8 MiB; 4096 bundles |
| `scratch_nodes` | node-layout bytes; nodes | — | 384 KiB; 6144 nodes | 512 KiB; 8192 nodes |
| `snapshot_buffers` | vector-buffer bytes; buffers | — | 3 MiB; 6144 buffers | 4 MiB; 8192 buffers |
| `mapping_nodes` | node-layout bytes; nodes | — | 768 KiB; 6144 nodes | 1 MiB; 8192 nodes |
| `memory.objects` | backing objects | 1024 | 6144 | 8192 |

Per-source defaults are 64 scheduler waiters, 128 endpoint/thread watches and
128 completion callbacks. Maximum endpoint capacity is 4096 entries; diagnostic
snapshot scratch is 1 MiB per request. These forward to `Policy.local`.

## Backing, physical fractions and kernel heap

`Policy.memory` groups:

- Per-object and domain object backing: 16384 pages each. One authenticated
  principal shares 16384 pages across its admitted backing and 1024 objects.
- Heap backing per root: 1264 pages, the existing launch ABI's `0x4f0000`-byte
  heap interval; the launch's ordinary initial heap is a separate value. Image/
  runtime backing permits 16384 pages per root. Private tables allow 1024 pages.
- IOMMU per-domain and per-unit backing: 1024 and 2048 pages respectively.
- Independent usable-RAM ratios: objects, heap and image each 1/4; stack 1/8;
  private tables 1/16; fresh shared tables 1/64; IOMMU tables 1/32. Each pool
  retains the previous minimum-one-page rule. Ratios apply after 4 KiB page
  conversion, preserving the original rounding.
- Ordinary share within those backing pools: 3/4. The memory-object record
  count has its separately grouped ordinary/total settings.
- Physical progress floor for ordinary object/heap/image/private-table frames:
  1/8 of usable pages, with a minimum of one page, checked under the frame lock.

`Policy.kernel_heap` selects 8 MiB initial backing and a growth reserve of usable
RAM / 64 clamped to 64–256 MiB, rounded to 2 MiB pages. Validated endpoints are
aligned; clamp precedes rounding to avoid overflow on extreme reported RAM.
The architecture's actual allocator-arena length is also checked before mapping.

These pools are independent admission ceilings, not disjoint preallocated RAM
partitions or a complete RAM ledger. They omit boot costs, some metadata and
allocator overhead. A configured reserve does not prove available physical RAM,
bounded lock hold times, fairness or service progress.

## Validation and coupled changes

Compile-time validation rejects zero/overlapping ordinary reserves, domain limits
above ordinary, dimensions beyond machine allocation range, invalid RAM ratios,
inconsistent backing ceilings, principal thread metadata above its shared pool,
maximum objects that cannot fit an ordinary snapshot, local watch/waiter limits
above their domain budgets, endpoint capacity beyond rounded domain backing,
and invalid/overflowing heap layout. The kernel separately checks heap backing
against the current launch VA contract. Unknown fields and wrong Rust types also
reject; omitted fields need an explicit `..DEFAULT` or a complete current policy.

The [larger example](../../crates/catten/src/resource_policy/larger.rs) changes
three related metadata families together. Other important relationships include
watch replacement overlap, RPC resources across several pools, thread/principal
metadata versus stack backing, and requested layouts versus heap overhead. Some
are workload-dependent and require measurement rather than a universal formula.
Changing a node total does not silently rescale other families; changes must be
explicit in the same reviewed profile.

The [host harness](../../scripts/tests/test-resource-policy.py) compiles the
actual policy and counter code for the default, larger and optional selected
file. It covers ordinary/total exhaustion and atomic rejection/refund, invalid
profiles, path selection, RAM rounding and extreme-RAM heap alignment. Native
controls read selected scratch/snapshot/mapping capacities, not duplicated default
numbers. Source/layout-specific pressure controls in other families may need
qualification changes for substantially different profiles. Passing static checks
never exempts a changed profile from boot and workload-pressure testing.

## Installation and remaining inventory

The file selector is an immutable build input. It is not a signed deployment
setting, boot-trust replacement, application capability or live-resize API.
Captured sponsor/generation/classification and original charge lifetimes remain
unchanged; configuring a higher limit cannot authorize cleanup or replay.

Boot-resource installation is not yet implemented. Static counters and early
heap preparation need one ordered installation boundary, complete authenticated
input, rejection before allocation, and validation against discovered resources.
The existing typed representation can be reused; account generations and retained
charges must not be resized or replaced after allocation begins.

This inventory covers kernel hardening admission and related backing policy.
Additional limits remain in their current contexts and need separate planning:
service-image/header/deployment capacities in `service/store.rs`, `loader.rs` and
`supervisor.rs`; per-request protocol buffers and userspace reactor concurrency;
recovery retry/deadline policy; and fixed architecture/launch/telemetry-array
layouts. Those are not silently configurable through this profile. Classify each
as workload capacity, scheduling/protocol policy or structural contract before
adding it here. New admission work should extend this central inventory and its
tests rather than introduce another unexplained family-local capacity literal.

See [qualification evidence](../reports/audits/2026-10-25-security-resource-policy.md).
SEC-07/18 remain partial; aggregate metadata/principal accounting and sustained
essential-service pressure remain separate acceptance work.
