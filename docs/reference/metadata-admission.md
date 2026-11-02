# Shared metadata admission

[Resource policy](resource-policy.md) selects numerical limits. The kernel's
[`klib::admission`](../../crates/catten/src/klib/admission.rs) supplies four
family-typed owners: byte/count node charges, fixed-bundle charges, multidimensional
reservations, and observer actual-layout charges. A single
[catalog](../../crates/catten-syscall/src/metadata_pools.rs) names all 22 pools,
their dimensions and units. Every pool keeps its own static counter lock; there
is no allocated registry, common hot lock or centralized resource owner.

Family code still captures trusted sponsorship, chooses the actual allocation
layout, translates errors and retains the charge inside its complete operation.
It decides when completion permits deallocation. Node deallocation precedes
refund, detached or quarantined owners retain their original charges, and root
retirement or generation reuse cannot reclassify them. The shared layer cannot
close resources or prove CPU, thread or device completion. Principal-sponsored
thread/context/table owners release their node reservation before their original
principal charge; each counter guard leaves before the next field drops.
Observer allocation rollback refunds layout bytes while retaining its holder
bundle until the holder is actually freed. Simple node, fixed-bundle,
two-dimensional reservation and actual-layout charge sizes remain respectively
16, 1, 24 and 16 bytes on supported 64-bit targets. No numerical limits change.

## Read-only runtime inventory

`THREAD_STATISTICS` payload **version 9** has a 22-word little-endian header.
Thread and domain records retain their layouts. Header words 20 and 21 hold the
metadata record size in bytes (88) and count. After both existing sections,
validated `SystemObserver` snapshots include the complete catalog, in catalog
order. Self-only snapshots include zero pool records. An invalid nonzero
observer capability rejects the request. The syscall register ABI remains
unchanged; services and both architecture bundles must use the current payload.

Each metadata record contains eleven `u64` words:

| Words | Meaning |
| --- | --- |
| 0–2 | Pool identity, dimension count, state (`0` sampled, `1` busy) |
| 3–4 | Total limits |
| 5–6 | Ordinary limits |
| 7–8 | Total usage |
| 9–10 | Ordinary usage |

Unused dimensions are zero. Busy records retain limits but have no usage sample;
their zero usage words are padding, not a claim that the pool is empty. The
borrowed SDK parser rejects other versions, wrong identities/order/dimensions,
invalid states/limits/usage, partial catalogs, overflow and inexact lengths.
Pass the declared payload length, excluding memory-object page padding. Catalog
identity/order or record changes must update the current payload version and all
writers, readers and fixtures together.

Observation prepares its output before sampling and uses `try_lock` once per
pool. A sample is coherent within that pool; pools are **not** sampled atomically
together. Every guard leaves before encoding or callbacks. No allocation, log,
waiting, retry loop, IRQ enable or cleanup operation is introduced beneath a
pool guard. Contention reports busy rather than stalling the observer.

The existing observe service forwards this owned-memory snapshot. HTTP status
JSON exposes `threads.metadata_pools`, with each pool's name, sampled/busy state
and dimensions: unit, total/ordinary limit and total/ordinary usage. Busy usage
is JSON `null`. Authorization remains the existing observation/status policy;
there is no mutable budget or teardown endpoint. History and cluster formats
retain their own existing fields.

Byte counts are requested allocation layouts, not physical RAM or allocator
capacity. Bundle-only pools do not report bytes. Pools may describe nested
allocations and must not be summed into a machine memory total. Limits reserve
headroom in admission, not preallocated memory. This inventory supports sizing;
it does not close R7's remaining allocation, sponsor, physical heap or concurrent
pressure gaps.

## Qualification

The host suite compiles the production common owners, policy, SDK parser and
observer handles. It exercises ordinary/total rejection without mutation,
original-class refunds, allocator rejection, layout rollback, bundle headroom,
concurrent release, busy observation, callback after guard release, and malformed
wire records. The authenticated HTTP liveness probe validates the complete
catalog, units, limits and sampled/busy usage in real status responses. Existing family fixtures retain their real allocation, detachment,
weak retention, cancellation and deallocation-before-refund checks. Native
telemetry fixtures validate self-only, authorized and forged observer requests,
exact output sizing and heap-held nonblocking inventory capture. Owner-size
checks run natively too. Host mutexes do not qualify IRQ behavior.

The [2026-11-02 evidence](../reports/audits/evidence/2026-11-02-metadata-admission.json)
records source/kernel/service identities and relevant diagnostics. Fresh four-CPU
240-second security campaigns passed ARM 19/19, Intel VT-d 17/17 and AMD-Vi
18/18, with zero failures or pending tests. AMD also passed authenticated HTTP
inventory and management TLS probes. The full host suite, strict kernel checks and native test
builds passed; both architectures' test/minimal service bundles were
rebuilt and signed. The earlier ARM preflight rejection is recorded separately.
These runs do not close broader allocation, concurrency or latency gaps.
