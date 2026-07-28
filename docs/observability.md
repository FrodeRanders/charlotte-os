# Observability

CharlotteOS now has a small, capability-preserving foundation for runtime
statistics. It is intentionally split into three layers:

1. The kernel records integer statistics at scheduler context switches.
2. An address-space-scoped syscall returns an immutable snapshot in a memory
   object owned by the caller.
3. The `observe` userspace service publishes its own snapshot through endpoint
   IPC and the node name service.

This is a foundation, not yet a complete machine-wide monitoring system.

## Running statistics

`klib::statistics::RunningStatistics` accepts `u64` samples and records:

- count;
- minimum and maximum;
- a `u128` total;
- a `u128` sum of squares; and
- whether any accumulator saturated.

Snapshots expose exact rational components for the mean and sample variance.
Floating-point conversion, square root, standard deviation, and coefficient of
variation belong in userspace. The kernel does not use floating point because
doing so would couple instrumentation to preservation of privileged FP/SIMD
state. Accumulators can be merged, which permits per-LP collection without a
global hot lock.

## Per-thread scheduler snapshot

Every thread records completed dispatch count and on-CPU slice duration. The
`THREAD_STATISTICS` syscall returns only records belonging to the caller's
address space. Each snapshot contains:

- thread ID and generation;
- owner ASID and current scheduler state;
- affinity and pinned LP, when set;
- dispatch count;
- count, minimum, maximum, total, and sum of squares for completed on-CPU
  slices;
- saturation status; and
- the start tick of a currently running slice, when applicable.

The header reports the architectural counter frequency and capture tick.
Consumers can therefore derive CPU time, mean and variance of execution slices,
and interval utilization by comparing two snapshots. The active slice is
reported separately and is not included in the completed-slice accumulator.
All wire fields are little-endian `u64` values; 128-bit values are encoded low
word first. The constants in `catten-syscall` are the authoritative ABI.

## Security and aggregation

Observability does not imply ambient inspection authority. A normal service can
inspect only its own protection domain. This is consistent with CharlotteOS's
capability model and prevents an arbitrary process from learning the activity
of unrelated services.

The boot supervisor starts exactly one `observe` service, grants it a typed
system-observer capability, and registers no other grant path. The service
registers the name `observe` and implements `OP_THREAD_SNAPSHOT`. Its reply
moves a machine-wide snapshot memory object to the caller. Fabricated,
wrong-type, or other-address-space handles fail closed.

Other observability producers can be built in two explicit ways:

- services voluntarily publish selected snapshots to an aggregator; or
- the supervisor delegates a more narrowly scoped observer capability
  authorizing a defined set of address spaces.

The implemented system observer is deliberately all-or-nothing. Production
policy may eventually add scope and metric-class rights, but must not represent
them with magic ASIDs or unrestricted syscalls.

## Internal instrumentation

The same integer accumulator can instrument internal operations by measuring
the architectural counter at entry and exit. Good initial candidates are:

- syscall and endpoint-call latency by opcode;
- ready-queue wait time and timer wake-up lateness;
- completion-queue occupancy and overflow backlog;
- allocator request size and allocation latency;
- interrupt service and deferred-work latency;
- storage and network request latency and queue depth; and
- service restart and name-lookup latency.

Hot paths should use per-LP accumulators and merge snapshots outside the path.
High-frequency functions may need sampling. Instrumentation overhead must
itself be benchmarked; collecting every possible measurement would work against
the system's predictability objective.

## External protocols

Endpoint IPC is the implemented observability protocol. An HTTP adapter in
userspace is architecturally straightforward: look up `observe`, request a
snapshot, convert it to text or JSON, and return it from a read-only endpoint.
It should remain a replaceable adapter rather than moving HTTP into the kernel.

The current TCP/IP service does not yet provide the complete server-side
`bind`/`listen`/`accept` path required for an HTTP listener, and end-to-end NIC
operation is still under development. Consequently no HTTP endpoint is claimed
yet. Once server sockets work, a minimal `GET /metrics` service can translate
the existing IPC snapshot without changing the kernel ABI.
