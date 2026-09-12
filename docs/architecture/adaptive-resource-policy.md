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
| Domain heap | Fixed 4 MiB per domain | `crates/charlotte-launch/src/lib.rs` |
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
- leader-driven replica reconciliation over committed membership, but no
  cross-node capacity input (`crates/charlotte-launch/src/placement.rs`; see
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
- **Wire exposure**: the `CCOSTAT` snapshot is version 3. Thread records carry
  `STACK_RESERVED_PAGES` (budget), `STACK_COMMITTED_PAGES` (mapped pages; the
  difference is the remaining growth headroom), and `STACK_USED_PAGES`; an appended per-domain
  section reports `ASID`, owned frames, reserved/high-water stack pages,
  touched high-water, and thread counts. Callers without the observer
  capability see only their own domain, preserving the existing capability
  posture (`docs/reference/observability.md`).
- **Aggregation semantics**: live per-thread high-water is reported directly in
  the thread records. The per-domain touched high-water is folded in when a
  thread is retired, so a long-lived thread's current mark is visible per
  thread while the domain aggregate reflects only completed threads. Per-domain
  reserved stack pages and thread counts are live.
- **Presentation**: the observe service forwards the page unchanged; httpd
  renders the new fields in `GET /metrics` and the dashboard.
- **In-memory history**: the observe service samples system aggregates every
  second into a bounded 256-sample ring and serves it through `OP_HISTORY`
  (`CCHIST` wire format). httpd renders the most recent samples as the
  `history` section.
- **Durable archive**: the same sampler writes to a bounded ring of
  object-store chunks (`CCARCH01`): sixteen 8 KiB chunks under reserved IDs
  `0xfffc_0000_0000_0001..16`, with the active chunk rewritten every ten
  seconds and on rotation. Sequence numbers let an offline reader detect
  overwritten history after the ring wraps. The store is resolved lazily
  through the name service (`obj`), so the archive fails soft and never
  delays sampling when storage is absent or restarting.
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

The initial 8 MiB heap claim is unchanged. Initial stack defaults, the domain
heap, and CQ ring capacities remain fixed; deriving those (from installed RAM
and expected domain/service counts) is the next piece of Phase 2 and is low
risk because it happens once, before concurrency exists.

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

The remaining Phase 3 work is applying a similar policy to the domain heap and
CQ capacities, which needs a controller surface rather than a per-launch
formula, and letting the placement layer see node pressure (Phase 4).

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
- The recoverable path charges one page from the thread's stack budget, maps
  it into the address space under the serializing address-space lifecycle
  lock, and returns to retry the faulting instruction. The existing adaptive
  size becomes the budget ceiling, not the reservation.
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
  committed frame.

Two negative models deliberately violate the budget on growth and leak on
kill, and the checker must produce the expected counterexample for each. Any
implementation that changes the accounting must keep the traces conforming.

### Resizable completion and endpoint capacities

Backlog high-water marks already exist per CQ (`completion::cq_pending`) and
per endpoint; capacities are caller-chosen at create but fixed for the
lifetime. A resize operation would act on the same high-water evidence that
drives stack sizing, bounded by the hard caps in `charlotte-launch` and IPC,
and must preserve the no-unbounded-queue invariant. This is a candidate for a
small state model once the exact resize protocol (drain, publish, swap ring)
is chosen.

### Capacity-aware placement

The placement layer is deterministic over membership and readiness and has no
capacity input. The safe shape is: nodes publish resource pressure through
discovery (a wire change), the Raft leader computes placement from
synchronized inputs, and local controllers never diverge replicated decisions.
This is policy work first; formal treatment belongs with the existing
cluster-ingress and Raft models.

## Verification

- Phase 1 is covered by the existing boot self-tests plus the versioned-wire
  self-test; accounting invariants (no negative counters, generation checks)
  are enforced in `memory::usage`.
- Controller decisions in later phases need dedicated self-tests, a fixed
  policy for CI, and a TLA+ treatment for any protocol that acquires or
  releases authority (stack growth, placement).

## Open questions

- What is an acceptable telemetry volume/retention budget per node? The chunk
  ring currently bounds the archive to sixteen 8 KiB objects and depends on
  the writer rotating; there is no store-side eviction.
- Should the resource controller be a distinct service or part of the
  supervisor?
- Which measurements are safe to expose without the system-observer
  capability, and which require a narrower telemetry capability?
