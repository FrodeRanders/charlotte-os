# Bounded kernel telemetry snapshots

`THREAD_STATISTICS` keeps its version-8 wire format and existing `(0, 0)`
unavailable result. Ordinary callers see their own domain; a validated
`SystemObserver` capability widens the thread and domain sections.

The kernel counts selected records under a short source hold, releases it,
then prepares storage with `Vec::try_reserve_exact`. Capture takes the source
hold again and copies plain records into that fixed buffer. It cannot grow the
buffer. A source that has grown beyond the prepared count rejects the whole
request; a smaller source produces a complete, smaller section. There is no
truncated prefix, internal retry loop or partially encoded response.

One request shares a **1 MiB scratch limit** across the thread vector, domain
vector and encoded byte vector, charging actual vector capacity. Arithmetic
overflow, capacity exhaustion, allocation rejection and source growth return
unavailable. The output memory object is allocated only after preparation and
encoding finish. Its physical backing and capability use their existing
admission; they are separate from this scratch limit. Failure to write the
response closes that unreturned output through the existing memory boundary.

This bounds one request's vector storage, not aggregate concurrent heap use,
all downstream memory-object metadata or essential-service progress. Outer
syscall IRQ/guard contexts and general allocator admission remain separate
SEC-07/18 work. The two sections are independent observations, not an atomic
machine-wide snapshot.

## Status-page lifetime

Each internal domain row retains its captured `AddressSpaceHandle`. Heap sampling
acquires an exact-generation `AddressSpaceOperation` outside usage/registry
guards and checks that the registered frame still matches that root's fixed
status mapping. The bounded volatile read remains behind the address-space
table hold; ordinary completion explicitly releases the root operation.
Closing, stale, absent and mismatched backing produce unavailable heap fields,
never a lookup of a successor by numeric ASID.

Final retirement cannot admit a new ordinary operation. Its peak sampler
instead borrows the existing `ClosingAddressSpace` while lifecycle is held,
before metadata detachment and physical root release. It uses the same exact
generation/mapping checks. No scalar physical address escapes either reader.
Abandonment of an admitted live read retains its root through the existing
operation policy; no new recovery or physical-release authority is introduced.
Values remain workload-published diagnostics rather than trusted resource
entitlements, and the counter fields need not be mutually atomic.

Sources: [snapshot scratch](../../crates/catten/src/klib/collections/snapshot.rs),
[syscall encoder](../../crates/catten/src/syscall/statistics.rs),
[thread capture](../../crates/catten/src/cpu/scheduler/threads/mod.rs),
[domain capture and heap reader](../../crates/catten/src/memory/usage.rs).
The [audit follow-up](../reports/audits/2026-10-18-telemetry-snapshot-preparation.md)
records deterministic controls and execution limits. See also
[observability](observability.md) and [cleanup/recovery](cleanup-recovery.md).
