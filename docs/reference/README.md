# Implementation reference

These documents explain current code-facing contracts and invariants. They
should be updated with the implementation when those contracts change.

- [Scheduler state machines](scheduler-state-machines.md) — thread, timer,
  completion, CQ, interrupt, context-switch, and lock-order invariants.
- [Locking](locking.md) — the synchronization primitives (spin mutex/rwlock,
  external spin, talc, lock-free containers), interrupt-masking discipline,
  and cross-subsystem lock-ordering rules.
- [Memory-object budgets](memory-object-budgets.md) — hard admission,
  generation-scoped sponsorship, transfer and late-release accounting, and
  node/platform progress reserves.
- [Completion-timer budgets](completion-timer-budgets.md) — domain/node event
  admission, reserved platform progress, cancellation ownership and deferred
  queue reclamation.
- [Completion-queue budgets](completion-queue-budgets.md) — queue counts,
  kernel backing bytes, fallible replacement and loader rollback.
- [Endpoint-close watch budgets](close-watch-budgets.md) — bounded one-shot
  registrations, local cancellation and transactional submission.
- [Completion-record budgets](completion-record-budgets.md) — retained-object
  and detached-result admission, node/platform pools, generation fencing and
  CQ replacement cleanup.
- [Endpoint budgets](endpoint-budgets.md) — endpoint and queue backing
  admission, transactional growth, retained delegation and generation-safe
  cleanup.
- [Raft conformance](raft-conformance.md) — required parity between
  `catten-graft`, the other Graft implementations, and the TLA+ projections.
- [Observability](observability.md) — capability-preserving runtime statistics,
  snapshot interfaces, and the node/cluster keyholes.
- [smoltcp adapter](smoltcp-adapter.md) — frame routing, adapter behavior, and
  the userspace TCP/IP service.
- [UTC time service](time-service.md) — default launch behavior, internal IPC
  operations, NTP synchronization, drift, uncertainty, and persisted holdover.
- [S3 client service](s3-client.md) — SigV4 object streaming, capability
  profiles, RustFS/ECS compatibility, and the TLS boundary.
- [Capability-grant controller](capability-grant-controller.md) — signed
  deployment grants, scoped application bootstrap, and private service
  discovery mediation.
- [Signed deployment notification](deployment-ingress.md) — off-cluster
  descriptor notification, Raft admission, and central-S3 node pickup.
- [Encrypted operational deployment bindings](../architecture/deployment-secrets-and-operations.md)
  — HPKE profiles, signed admission bundles, role-aware trust, leader-verified
  ingress, replay fencing, privileged S3 pickup, kernel-only HPKE open, and
  read-only connector profile delivery.
- [Kafka client service](kafka-client.md) — idempotent production,
  read-committed consumption, transactional offsets, and owned backpressure.
- [Cryptographic entropy](entropy.md) — architectural randomness, the
  capability-scoped VirtIO RNG service, and QEMU provisioning.
