# Bounded kernel telemetry snapshots

`THREAD_STATISTICS` uses the current version-11 payload and existing `(0, 0)`
unavailable result. Ordinary callers see their own domain; a validated
`SystemObserver` capability widens the thread and domain sections and adds the
[metadata pool inventory](metadata-admission.md#read-only-runtime-inventory).
Its fixed 24 records are included in checked output sizing; sampling takes no
allocation and reports independently busy pools without waiting.

The kernel counts selected records under a short source hold, releases it,
then prepares storage with `Vec::try_reserve_exact` through its captured
`ByteAllocator<SnapshotBuffers, Sponsor>`. Capture takes the source
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

All three vectors also share the existing `snapshot_buffers` requested-byte
pool (16 MiB total, 12 MiB ordinary). They consume bytes and zero frame-buffer
counts. Each allocation first admits its whole requested layout against the
captured signed principal's **16 MiB kernel-buffer ceiling**, shared with
[root frame catalogs](page-table-lifetime.md#root-data-frame-catalogs) and
[primitive input/plaintext and CPU/copy/DMA frame buffers](snapshot-buffer-admission.md), then against the
family. The counter lives in the existing boot-lifetime principal-history record;
there is no second registry. Allocation/family rejection refunds only the unused
prefix. Growth admits the full candidate while the old allocation stays charged;
actual vector deallocation precedes both family and principal refunds.

The syscall captures principal and trusted class under one root-table hold,
before counting sources or allocating. The principal comes from the verified
root's prebound translation account. A `SystemObserver` capability widens what
can be observed; it supplies neither trusted storage classification nor a new
sponsor. Moves, allocator clones, root retirement, promotion and ASID reuse do
not replace existing buffer sponsorship. Unidentified kernel/raw boot-fixture
roots retain node admission only; signed application construction prebinds its
principal before any application runs. No caller-supplied name, role or identity
selects this exception. Capture itself grants no root lease or cleanup authority.

All scratch elements are plain `Copy` observations. Preparation/allocation and
all vector disposal leave thread/usage/physical/metadata observation guards;
callbacks from the read-only pool visitor run after its leaf hold. Principal,
family and allocator locks are taken separately, with each released before the
next. Ordinary failure drops the entire local prefix; there is no active
physical/capability element requiring retained rollback in these buffers.
The output object retains its existing independent physical/capability owner.

The 1 MiB per-request bound is unchanged. The 16 MiB principal ceiling permits
sixteen such maximum working sets in isolation from other charged kernel buffers and allocates no backing in
advance; the shared family may bind earlier, including competition with frame
snapshots and primitive syscall inputs. Family ceilings and kernel heap reserve
are unchanged. This corrects diagnostic scratch admission, not all downstream
memory-object metadata, allocator overhead, general kernel-heap principal
coverage or essential-service progress. Outer syscall IRQ/guard contexts remain
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
records the original deterministic controls and execution limits. The
[Q4 diagnostic qualification](../reports/audits/2026-11-05-q4-diagnostic-buffers.md)
adds principal/family admission, allocator-traced growth and three-target plus
minimal qualification, retaining the failed host control separately. See also
[observability](observability.md) and [cleanup/recovery](cleanup-recovery.md).
