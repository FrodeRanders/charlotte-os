# Implementation reference

These documents explain current code-facing contracts and invariants. They
should be updated with the implementation when those contracts change.

- [Scheduler state machines](scheduler-state-machines.md) — thread, timer,
  completion, CQ, interrupt, context-switch, and lock-order invariants.
- [Locking](locking.md) — the synchronization primitives (spin mutex/rwlock,
  external spin, talc, lock-free containers), interrupt-masking discipline,
  and cross-subsystem lock-ordering rules.
- [Raft conformance](raft-conformance.md) — required parity between
  `catten-graft`, the other Graft implementations, and the TLA+ projections.
- [Observability](observability.md) — capability-preserving runtime statistics
  and snapshot interfaces.
- [smoltcp adapter](smoltcp-adapter.md) — frame routing, adapter behavior, and
  the userspace TCP/IP service.
- [UTC time service](time-service.md) — default launch behavior, internal IPC
  operations, NTP synchronization, drift, uncertainty, and persisted holdover.
- [S3 client service](s3-client.md) — SigV4 object streaming, capability
  profiles, RustFS/ECS compatibility, and the TLS boundary.
- [Kafka client service](kafka-client.md) — idempotent production,
  read-committed consumption, transactional offsets, and owned backpressure.
- [Cryptographic entropy](entropy.md) — architectural randomness, the
  capability-scoped VirtIO RNG service, and QEMU provisioning.
