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
| Kernel heap | 8 MiB initial + 64 MiB pre-mapped growth reserve; hard 72 MiB cap independent of installed RAM | `crates/catten/src/memory/allocators/global_allocator.rs` |
| Domain heap | Fixed 4 MiB per domain | `crates/charlotte-launch/src/lib.rs` |
| User stack | Signed per deployment, 1–64 pages (4 KiB–256 KiB), inherited by every thread | `crates/catten/src/memory/mod.rs`, `crates/charlotte-launch/src/deployment.rs` |
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
- **Wire exposure**: the `CCOSTAT` snapshot is version 2. Thread records add
  `STACK_RESERVED_PAGES`/`STACK_USED_PAGES`; an appended per-domain section
  reports `ASID`, owned frames, reserved/high-water stack pages, touched
  high-water, and thread counts. Callers without the observer capability see
  only their own domain, preserving the existing capability posture
  (`docs/reference/observability.md`).
- **Aggregation semantics**: live per-thread high-water is reported directly in
  the thread records. The per-domain touched high-water is folded in when a
  thread is retired, so a long-lived thread's current mark is visible per
  thread while the domain aggregate reflects only completed threads. Per-domain
  reserved stack pages and thread counts are live.
- **Presentation**: the observe service forwards the page unchanged; httpd
  renders the new fields in `GET /metrics` and the dashboard.

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
  1. an in-memory history ring in the observe service (bounded, lost on
     reboot, immediately useful in CI and on a dev machine);
  2. an append-only archive on the local object store (analyzable from a
     captured NVMe image, no new egress policy);
  3. remote/cluster sinks through the existing S3 client or a dedicated
     telemetry endpoint (needs an explicit egress decision; observation does
     not imply ambient authority).
- **Offline tooling.** A host-side analyzer, analogous to
  `scripts/symbolize-kernel-panic.py`, should consume an archive and summarize
  per-domain peaks, stack headroom, and growth trends. CI should archive the
  telemetry file alongside the serial log.

## Phase 2: boot-time derivation

Replace constants with formulas over the discovered memory map, with today's
values as floors: kernel-heap reserve as a fraction of usable frames, initial
stack defaults from installed RAM and expected domain counts, and CQ ring
capacities from expected service counts. This is low risk because it happens
once, before concurrency exists.

## Phase 3: creation-time feedback

A supervisor-side controller chooses `ServiceLimits` per launch from:

- the previous generation's stack high-water mark for the same service
  identity, plus a safety margin;
- current free frames, live domain count, and thread pressure;
- the signed maximum as a hard clamp.

This is the first genuine control loop. It must be observable (every decision
logged or published) and pin-able for tests.

## Phase 4: in-life adaptation

- **Demand-grown user stacks.** A guard page plus a recoverable EL0 fault that
  extends the stack within a per-domain page budget; over-budget overflows kill
  the domain cleanly. Today a stack fault aborts the whole address space
  (`crates/catten/src/cpu/isa/aarch64/interrupts/mod.rs`), so this needs a
  recoverable fault path and a budget protocol that should be modeled in TLA+.
- **Resizable completion/endpoint capacities** driven by backlog high-water
  marks, always within hard caps.
- **Capacity-aware placement.** Publish per-node resource pressure through
  discovery and let the Raft leader place from synchronized inputs; local
  controllers must not diverge replicated decisions.

## Verification

- Phase 1 is covered by the existing boot self-tests plus the versioned-wire
  self-test; accounting invariants (no negative counters, generation checks)
  are enforced in `memory::usage`.
- Controller decisions in later phases need dedicated self-tests, a fixed
  policy for CI, and a TLA+ treatment for any protocol that acquires or
  releases authority (stack growth, placement).

## Open questions

- Which durable sink comes first: local object-store archive or in-memory
  history ring?
- What is an acceptable telemetry volume/retention budget per node?
- Should the resource controller be a distinct service or part of the
  supervisor?
- Which measurements are safe to expose without the system-observer
  capability, and which require a narrower telemetry capability?
