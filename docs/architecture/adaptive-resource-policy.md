# Adaptive resource policy

This note frames how CharlotteOS should adapt resource sizes to the machine it
finds itself on and to the work it is running, and it records the first
implemented step. It is deliberately split into observation (safe, no behavior
change), boot-time derivation, creation-time feedback, and in-life adaptation,
so each stage can be evaluated on its own.

The guiding position is **mechanism in the kernel, policy in userspace**. The
kernel owns hard caps, sensors, and the operations that are only safe close to
the hardware; a resource controller decides *how much* to hand out. Adaptation
never widens a signed envelope.

## What the system does today

The distinction that matters is between memory the frame allocator can reach
and structures sized by constants.

| Resource | Today | Source |
|---|---|---|
| Physical frames | All `MEMMAP_USABLE` RAM, no RAM cap; bitmap sized to the highest usable address | `crates/catten/src/memory/physical/mod.rs` |
| Kernel heap | 8 MiB initial + pre-mapped growth reserve derived from usable RAM: `clamp(usable/64, 64 MiB, 256 MiB)`, 2 MiB-aligned (Phase 2) | `crates/catten/src/memory/allocators/global_allocator.rs` |
| Domain heap | Virtual window with a load-time capacity adapted from the principal's previous peak, backed on first touch; live allocation and peak are sensed through the standard status-page record | `crates/catten-rt/src/lib.rs`, `crates/catten/src/memory/mod.rs`, `crates/catten/src/service/loader.rs` |
| User stack | Signed per deployment, 1–64 pages (4 KiB–256 KiB), inherited by every thread; kernel-launched services without a descriptor adapt to the principal's previous high-water (Phase 3); both architectures commit one page and grow on fault up to that budget (Phase 4) | `crates/catten/src/memory/mod.rs`, `crates/charlotte-launch/src/deployment.rs`, `crates/charlotte-lifecycle/src/lib.rs`, `crates/catten/src/cpu/isa/*/lp/thread_context*` |
| User threads | Signed maximum, 1–64 per domain | `crates/catten/src/cpu/scheduler/system_scheduler/mod.rs` |
| CQ rings, endpoint queues | Mostly static entry counts; endpoint capacity is caller-chosen at create | `crates/catten/src/completion/cq.rs`, `crates/catten/src/ipc/mod.rs` |
| Service buffers | Compile-time constants (network, relmsg, Raft, storage) | `crates/catten-services`, `charlotte-protocol-*` |

A 1 TiB node would still run with an 8 MiB kernel heap and 4 MiB per-domain
heaps. Frames are reachable through services, but the metadata structures do
not scale with the machine.

Existing runtime adaptation is real but localized:

- scheduler load balancing with a sustained-imbalance window
  (`crates/catten/src/cpu/scheduler/system_scheduler/mod.rs`);
- idle-LP timer fallback and per-LP timer re-arming (`crates/catten/src/timers`);
- CQ backlog spilling and coalesced deferred interrupt wakes
  (`crates/catten/src/completion/mod.rs`, `crates/catten/src/device/mod.rs`);
- userspace endpoint backpressure retries;
- Raft election jitter/backoff and relmsg fragment-scaled retransmission;
- leader-driven replica reconciliation over committed membership, with
  low-pass-filtered per-node capacity signals, reservation-aware admission,
  and dwell-controlled pressure reassignment
  (`crates/charlotte-launch/src/placement.rs`; see
  [cluster artifacts and placement](cluster-artifacts-and-placement.md)).

## Design principles

1. **Signed maxima are security invariants.** A controller may only choose
   below a blessed envelope (`MAX_USER_STACK_PAGES`, `MAX_USER_THREADS`, signed
   deployment limits). A buggy controller then costs performance, not safety.
2. **Policy in userspace.** The kernel exposes sensors and enforces caps; a
   supervisor/controller service computes allocations. This mirrors the
   existing signed-descriptor model and keeps the trusted core small.
3. **Coarse timescales and dwell times.** Controllers run at tens to hundreds
   of milliseconds, never in the syscall or interrupt hot path. The scheduler's
   `REBALANCE_WINDOW_MILLIS` is the precedent: deadband, hysteresis, and
   minimum dwell to avoid oscillation.
4. **Bounded memory, fail closed.** Every history ring, cache, and archive has
   a hard cap. Sizing changes never introduce unbounded queues.
5. **Generation-stable accounting.** Per-domain counters are keyed by the
   generation-bearing `AddressSpaceHandle`, so ASID reuse cannot transfer one
   domain's accounting to its successor.
6. **Adaptation must be pin-able.** CI asserts exact boot/self-test behavior;
   every controller needs a fixed-policy mode and observable decisions.

## Control model

[[sensors]] -> [[controllers]] -> [[actuators]] with an explicit stability
budget.

| Layer | Existing | Phase 1 additions |
|---|---|---|
| Sensors | LP load summaries, CQ pending/backlog, interrupt counts, timer queues | Per-domain owned frames, stack reservations, thread counts, stack high-water marks |
| Controllers | Scheduler rebalance; Raft backoff | (none yet; Phase 3) |
| Actuators | Migration, timer intervals, endpoint capacity, launch limits | (none yet; Phase 2/4) |

## Phase 1: observation and accounting (implemented on this branch)

Phase 1 changes no allocation behavior. It adds:

- **Per-domain accounting** in `crates/catten/src/memory/usage.rs`: frames
  registered by the loader, user-stack pages reserved by live threads,
  high-water marks for reserved stack pages, touched stack pages, and thread
  counts. Entries live and die with the address-space lifetime.
- **Stack high-water sampling**: each `ThreadContext` keeps a relaxed atomic
  low-water mark, updated from the context-switch path (AArch64 reads banked
  `SP_EL0`; x86-64 uses the per-LP user-RSP scratch slot saved on SYSCALL
  entry). Sampling is lock-free and never allocates.
- **Wire exposure**: the `CCOSTAT` snapshot is version 7. Thread records carry
  `STACK_RESERVED_PAGES` (budget), `STACK_COMMITTED_PAGES` (mapped pages; the
  difference is the remaining growth headroom), and `STACK_USED_PAGES`; an
  appended per-domain section reports `ASID`, owned frames, reserved/high-water
  stack pages, touched high-water, thread counts, and the heap record
  (validity, capacity, allocated, peak) when the domain publishes one. Callers
  without the observer capability see only their own domain, preserving the
  existing capability posture (`docs/reference/observability.md`).
- **Aggregation semantics**: live per-thread high-water is reported directly in
  the thread records. The per-domain touched high-water retains retired-thread
  contributions; consumers take the maximum of that aggregate and the live
  thread records. Per-domain reserved stack pages and thread counts are live.
- **Presentation**: the observe service forwards the page unchanged; httpd
  renders the new fields in `GET /metrics` and the dashboard.
- **In-memory history**: the observe service samples system aggregates every
  second into a bounded 256-sample ring and serves it through `OP_HISTORY`
  (`CCHIST` version 2). Besides domain, thread, stack, and owned-frame totals,
  each record carries free/usable frames, logical processors, the monotonic
  CPU-busy counter, and aggregate live/peak heap bytes. httpd renders the most
  recent samples as the `history` section.
- **Durable archive**: the same sampler writes to a bounded ring of
  object-store chunks (`CCARCH01`, version 2): sixteen 8 KiB chunks under reserved IDs
  `0xfffc_0000_0000_0001..16`, with the active chunk rewritten every ten
  seconds and on rotation. Sequence numbers let an offline reader detect
  overwritten history after the ring wraps. The store is resolved lazily
  through the name service (`obj`). Flushes are an asynchronous four-stage
  create/size/write/flush transaction with a two-second deadline; all memory,
  connection, and pending-call resources remain in one owning state machine.
  A missing, slow, or restarting store therefore delays durability without
  blocking the one-second sampler; reconnection backfills the most recent
  chunk-sized window from in-memory history.
  `scripts/telemetry-archive.py` reassembles the chunks from a captured NVMe
  image for offline analysis.

It is careful about the hot paths: context-switch sampling uses one atomic
operation; scheduler, loader, and teardown hooks are off the interrupt path.

### Telemetry export

Point-in-time snapshots serve live dashboards, not offline analysis. The
measured counters are intended to feed developers and operations, so export is
part of the phase rather than an afterthought:

- **Schema stability.** Treat `CCOSTAT` like an on-disk format: magic,
  version, and per-record byte sizes are part of the contract, and an analyzer
  must reject what it does not understand. Identifiers carry generations.
- **Volume and cardinality.** Per-thread records every second are expensive;
  archives should carry per-domain aggregates plus bounded per-thread
  summaries. Export deltas with the counter frequency so consumers can compute
  rates.
- **Timestamps.** Every sample carries monotonic ticks and frequency; wall
  clock is correlated through the time service, not assumed.
- **Sinks, in order of increasing trust cost:**
  1. an in-memory history ring in the observe service (implemented: bounded,
     lost on reboot, immediately useful in CI and on a dev machine);
  2. a chunked archive on the local object store (implemented: analyzable from
     a captured NVMe image, no new egress policy);
  3. remote/cluster sinks through the existing S3 client or a dedicated
     telemetry endpoint (needs an explicit egress decision; observation does
     not imply ambient authority).
- **Offline tooling.** `scripts/telemetry-archive.py` reassembles the chunk
  ring from a captured NVMe image and prints ordered samples (or NDJSON), so a
  developer or operator can inspect trends without booting the node. CI should
  archive the telemetry alongside the serial log when a run exercises the
  archive.

## Phase 2: boot-time derivation

The kernel-heap growth reserve is now derived from the discovered memory map
instead of a constant: one sixty-fourth of usable RAM, clamped between the
historical 64 MiB floor and a 256 MiB ceiling, and rounded up to the 2 MiB
large-page granularity of the allocator arena. A 512 MiB QEMU node keeps the
old 72 MiB total; an 8 GiB node maps a 136 MiB total heap; a large server caps
at 264 MiB. The frame allocator now exposes `usable_bytes()` from the boot
memory map, and the allocator self-test asserts the derived value stays inside
its bounds and page-aligned.

The initial 8 MiB kernel-heap claim is unchanged. Domain heap pages and user
stack pages are committed on demand within their launch budgets. CQ ring
capacities remain fixed; deriving those from expected domain/service counts is
still a possible boot-time policy because it happens before concurrency exists.

## Phase 3: creation-time feedback (implemented baseline)

The supervisor now chooses the stack allocation for services launched without
a signed deployment descriptor from the previous generation of the same
principal:

- when an address space is torn down, `memory::usage` retains its touched
  stack high-water mark in a per-principal table (in memory, reset at boot);
- the next launch calls the pure `charlotte_lifecycle::adaptive_stack_pages`
  policy: the recorded mark plus one page of headroom, clamped to the default
  and signed maxima;
- a cold boot or an unknown principal selects the default, so the first
  generation of every service is deterministic and CI behavior is unchanged;
- signed deployment descriptors still win — they never pass through the
  adaptive path.

Growth is damped by memory pressure. The frame allocator maintains an exact
free-frame count on every bitmap transition, and the supervisor withholds
history-based growth whenever free frames fall below one sixteenth of usable
RAM, falling back to the default; pressure never shrinks a stack below what a
first generation would receive. Every decision with recorded history is logged
as
`[supervisor] adaptive stack: principal=... high_water_pages=... free_frames=...
reserve_frames=... stack_pages=...`.
On a default boot only restarting services (for example the UART driver after
its uncooperative-exit test) exercise the path, and the clamp keeps them at the
default until their observed usage actually reaches it.

The same creation-time feedback now applies to the domain heap, and committed
node pressure feeds placement. CQ capacity feedback remains future work because
the current one-page CQ ABI does not support resizing its physical ring.

## Heap sensing and physical sizing

Heap sensing is implemented. `catten-rt` wraps the domain's talc arena with an
accounting layer that publishes `charlotte_launch::heap_status` (magic/version,
capacity, currently allocated bytes, peak bytes) into the reserved region of
the domain's own status page. The kernel reads that page when it builds the
domain records and exposes it as `CCOSTAT` v7, so `httpd`/`/metrics` shows live
and peak heap per domain. Observed peaks on the default boot are tens of
kilobytes against the 4 MiB capacity. The same record carries cumulative
allocations and allocated bytes plus the arena-lock spin count, and httpd
derives an allocation rate and reports the spin total — the measurement base
for deciding whether the heap needs sharding.

Physical sizing is implemented by demand commitment rather than by shrinking
the mapped reservation. The loader reserves the heap's virtual window but maps
no frames; the first touch of each heap page faults, commits one zeroed frame
owned by the address space, and retries the access. Teardown releases those
frames with the domain's other owned frames. On the default boot this drops
per-domain owned frames from 1024 reserved heap pages plus metadata to tens of
pages, while the full 4 MiB capacity stays available — so a service can never
fail an allocation because a capacity policy guessed too small.

Capacity sizing is implemented alongside it. The loader chooses the capacity
with `charlotte_lifecycle::adaptive_heap_bytes` — twice the principal's
previous heap peak plus 256 KiB, clamped to `[HEAP_SIZE, HEAP_VA_LIMIT]` —
writes it into the launch header, registers it so faults beyond the claim stay
domain errors, and `catten-rt` claims exactly that size through a lazy arena
initialization. A cold boot and a lightly loaded restart keep the historical
4 MiB capacity; evidence may grow a later generation but never shrink it below
that compatibility floor. Demand commitment, rather than a guessed smaller
virtual limit, supplies the physical-memory saving. A signed per-deployment
heap limit would still need a descriptor field.

The VA layout caps any single heap at roughly 4.9 MiB. Growing beyond that, or
giving each shard its own arena, is a layout decision; the shard-local study
below frames it.

### Shard-local heap (study)

The heap is the one serialization point inside a domain: `catten-rt` exposes a
single talc arena behind a spin mutex, so any two shards that allocate contend
even though their CQs and stacks are shard-local. Three shapes are worth
comparing:

- **Per-shard arenas with a routing header.** Allocate from the calling shard's
  arena; store the arena index in a small per-allocation header so `dealloc`
  returns the block to its owner. Sharing pointers across shards then works by
  default, at the cost of a header word and a cross-shard free path.
- **Per-shard arenas plus an explicit shared arena.** Keep a domain-level
  shared arena for data that intentionally crosses shards and a per-shard arena
  for everything else. The type system can express the distinction (shared
  versus shard-local owners), giving zero-overhead frees on the local path
  while making cross-shard sharing an explicit decision — the same
  qualified-sharing discipline the capability model applies across domains.
- **Keep one arena, reduce hold time.** The talc critical section is short and
  the contention may not be measurable. The heap record now publishes
  cumulative allocations, allocated bytes, and arena-lock spin iterations, and
  `httpd` derives an allocation rate, so this option is decided by evidence:
  zero spins means no restructuring is justified yet.

Does shard locality constrain LP placement? Not directly. Shards are logical
work partitions; services already pin shard workers to LPs (`SHARD_CQ_COUNT`
rings, `pinned_lp`/`affinity_lp`), but shards and LPs are not one-to-one. A
per-shard arena follows the shard, which is the stable identity, so migration
does not move an arena and a shard that outlives an LP keeps its locality. The
property to preserve is "the allocating thread usually frees in the same
shard", which scheduler affinity already encourages; shard-local heaps
reinforce the recommended model rather than impose new placement constraints.

Qualifying memory for sharing within the protected domain is a userspace
allocator concern, not a kernel one: all shards share one address space, so the
kernel cannot distinguish pointers and should not try. The practical mechanism
is a typed allocator API in `catten-rt` (shard-local versus shared owners with
`dealloc` routing), plus the rule that a shard-local allocation must not escape
its shard. That keeps the fast path uncontended per shard while making
deliberate sharing cost one indirection.

## Phase 4: in-life adaptation

Phase 4 changes behavior while a domain runs, so each mechanism needs an
explicit failure story before implementation.

### Demand-grown user stacks (implemented)

Threads now start with one committed stack page and grow downward a page at a
time on translation faults, bounded by the signed or adaptive budget. Both
architectures classify data-access faults (AArch64 data/instruction aborts,
x86-64 not-present user-mode data page faults) against the growable guard
region; only faults inside a thread's own region that stay within its budget
and the free-frame reserve are handled. Everything else remains fatal and
aborts the address space, so an over-budget fault is a clean domain kill and
`ThreadContext::drop` releases exactly the committed pages.

The design this implements is:

- Each stack reserves a guard page below the lowest committed page. A fault in
  the domain's own stack VA range is identified by the kernel rather than
  treated as an arbitrary translation fault.
- The recoverable path preflights every page between the current stack bottom
  and the faulting page against both the budget and the node reserve, then maps
  those pages under the serializing address-space lifecycle lock and returns to
  retry the instruction. This covers instructions that move the stack pointer
  by several pages while preserving the reserve after the complete growth.
  The adaptive size is the budget ceiling, not a physical reservation.
- A fault with the budget exhausted kills the domain through the ordinary
  teardown path: all stack frames return to the frame allocator exactly once,
  threads are retired, and the supervisor observes a clean domain-exit rather
  than a wedged LP.
- The free-frame reserve that damps Phase 3 growth also gates in-life growth,
  so a node under pressure fails closed instead of exhausting the pool.
- Kernel stacks are out of scope; they remain fixed-size and are never grown
  from a fault.

Because this protocol acquires and releases a finite resource across a fault
edge, it is modeled in TLA+ before implementation: see
`docs/tla/CharlotteStackGrowth.tla`, registered in `docs/tla/check.sh`. The
model's safety invariants are:

- `CommittedWithinBudget`: a domain never commits more stack pages than its
  budget allows;
- `FrameConservation`: committed pages plus free frames equal the pool, so a
  growth or teardown neither leaks nor double-counts;
- `DeadDomainsReleaseFrames`: a killed or exited domain has returned every
  committed frame;
- `FreeReservePreserved`: successful growth leaves the configured free-frame
  reserve intact.

Two negative models deliberately violate the budget on growth and leak on
kill, and the checker must produce the expected counterexample for each. Any
implementation that changes the accounting must keep the traces conforming.

### Resizable endpoint capacities (implemented) and CQ rings

Endpoints now expose capacity adaptation. `IPC_ENDPOINT_RESIZE` (80) changes
the admission bound, clamped to `MAX_ENDPOINT_CAPACITY`; queued messages are
preserved, and a bound below the current depth only rejects new sends until
the queue drains. `IPC_ENDPOINT_STATUS` (81) returns capacity, queued depth,
and the depth high-water, and `catten-rt`'s owned `Endpoint` wraps both. A
service reactor can raise its bound as the high-water approaches capacity and
lower it when idle, always inside the hard cap.

CQ rings are different. The ring is a single EL0-mapped page whose modulus is
cached from the launch layout, and its physical capacity is at most 127
entries. The bounded, non-lossy backlog already absorbs bursts beyond the
ring's free space, so an in-life CQ resize needs a multi-page ring ABI (or
consumers re-reading a published capacity) before an actuator is meaningful;
backlog high-water remains the sizing evidence at deployment time.

### Capacity-aware placement and controlled movement

The placement layer is deterministic over membership and readiness. Five
parts are implemented:

- **Raw sensor data**: the `CCOSTAT` header carries machine-wide `free_frames`
  and `usable_frames` from the frame allocator, exposed through `observe` and
  rendered by httpd. It also carries online logical processors and a monotonic
  on-CPU counter that retains retired-thread runtime. The one-second history
  and object-store archive keep these unfiltered measurements for diagnosis.
- **Local control sample**: `NODE_PRESSURE` derives interval CPU occupancy for
  each calling protection-domain generation. Every `dns` instance samples it
  every five seconds and reports free frames, usable frames, and CPU occupancy
  in permille. A follower relays a 42-byte `rcapacity` frame to the leader;
  the report includes a nonzero entropy-derived boot nonce and monotonic epoch.
  Reporting is withheld until an entropy source is ready.
- **Low-pass controller**: the leader identity-checks the reporting peer,
  rejects duplicate or decreasing epochs, and applies an integer EWMA with
  alpha 1/4 to free frames and CPU occupancy. A new boot incarnation or changed
  physical-memory size resets the filter. Raft receives a `CMD_NODE_CAPACITY`
  command only when this filtered value crosses a memory or CPU bucket, usable
  memory changes, or free frames move by more than one sixteenth of usable
  memory. A three-report freshness lease prevents an old committed value from
  becoming authority after communication loss or leader failover.
- **Reservation-aware admission**: `NodeCapacityView` combines the filtered
  control signal with `committed_frames`, reconstructed from all other catalog
  deployments. New placement preserves a one-sixteenth dynamic reserve and a
  static total-capacity reserve. Every selected instance, including a pinned
  one, debits both projected values. Artifacts replaced by the same release are
  removed from the prior-reservation projection, avoiding double reservation.
  If a dynamic report expires while the node still holds commitments, the
  projection retains those promises with zero additional headroom; a later
  release cannot reinterpret the node as empty. CPU remains a soft ranking
  signal: heavy occupancy ranks a node last but does not make an otherwise
  feasible cluster unavailable.
- **Movement actuator**: reconciliation can derive a different replica set
  from the filtered signal, but a pressure-only candidate must remain unchanged
  for 30 seconds, and another pressure relocation cannot follow for 60 seconds.
  Membership loss or committed drain bypasses dwell because the old assignment
  is no longer eligible. The reassignment remains a generation-fenced Raft
  command, and exact-generation readiness keeps DSR from using a replacement
  before it publishes.

Raw telemetry and placement input are deliberately separate data products.
Keeping spikes in the archive preserves evidence needed to tune policy;
filtering only the control path prevents that fidelity from causing replica
oscillation. DNS status counters expose accepted observations, proposed
filtered updates, total reassignments, and topology-forced reassignments.

## Verification

- Phase 1 is covered by the existing boot self-tests plus the versioned-wire
  self-test; accounting invariants (no negative counters, generation checks)
  are enforced in `memory::usage`.
- Capacity control is covered by strict `rcapacity` validation, EWMA/replay
  tests, bucket hysteresis tests, pressure dwell/cooldown tests, cumulative and
  cross-release reservation tests, and a catalog replay/snapshot test. The
  three-member distributed-ingress fixture requires one leader to seed fresh
  control state for all reporters, then requires the successor leader to
  rebuild state from the surviving reporters after failover. This also guards
  the transport's tag-included receive convention for relayed `rcapacity`
  frames.
- `CharlottePlacementControl.tla` checks filtered candidate selection,
  dwell/cooldown enforcement, topology-forced movement, and preservation of
  static reservation headroom. The cluster-ingress model separately covers
  generation-safe replacement and readiness. Direct pressure injection and
  observation of a dwell-controlled QEMU relocation remain to be added.

## Open questions

- What is an acceptable telemetry volume/retention budget per node? The chunk
  ring currently bounds the archive to sixteen 8 KiB objects and depends on
  the writer rotating; there is no store-side eviction.
- Should the resource controller be a distinct service or part of the
  supervisor?
- Which measurements are safe to expose without the system-observer
  capability, and which require a narrower telemetry capability?
