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

Threads do not currently carry human-readable names. The snapshot identifies
them by thread ID, generation, and owning address-space ID; application and
service names belong to the name-service registry and are not thread labels.

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

Endpoint IPC is the implemented observability protocol. The `httpd` keyhole
service in userspace is that adapter in practice: it looks up `observe`,
requests a machine-wide snapshot, and merges it with per-service status into
a JSON report served over TCP port 80. It remains a replaceable userspace
adapter rather than moving HTTP into the kernel.

The TCP/IP service provides the complete server-side
`bind`/`listen`/`accept` path (smoltcp over the frouter's IP/ARP routing),
and end-to-end NIC operation is validated. `httpd` aggregates the `observe`
snapshot plus the `net`/`tcpip`/`frouter`/`ns`/`dns`/`disco`/`relmsg` status
ops into one JSON page, reachable from the host via SLIRP `hostfwd`
(`scripts/run-aarch64.sh --http-test` or
`scripts/run-x86_64.sh --http-test`). The name-service section preserves
printable registry names as JSON strings and represents internal binary keys as
`hex:<bytes>`; those hexadecimal entries are intentional opaque identifiers,
not unnamed threads or malformed text.

The report is time-aware, not just a point-in-time dump. It reuses the
observe snapshot's monotonic counter and frequency (`mono_ticks` and
`counter_hz`) as a wall-clock source, so it reports uptime, the interval
between consecutive requests, and per-counter `*_delta`/`*_rate` fields for
`tcpip` and `frouter` — all with integer arithmetic, no floating point and no
dependency on a real-time clock. Services publish richer diagnostics through
the protocol as well: `socket::OP_STATUS` carries send errors, DHCP mode,
gateway, and MTU; `relmsg::OP_DIAG` and `disco::OP_DIAG` move a page of live
transport/probe counters (peers, retransmits, send failures, received,
in-flight, decoded frames); `disco::OP_LIST_PEERS` supplies the peer table;
and `dns` also serves `raft::OP_CLUSTER_STATUS` for commit index, member
count, and leader identity.

The aggregation model follows the two explicit producer paths above: each
service *voluntarily publishes* a status op, and the `observe` service is the
sole holder of the system-observer capability; the httpd holds neither and
queries both over IPC.
