# Cleanup qualification details

Current C16/C18 contracts and review boundaries, consolidated 2026-10-30. Use
the [owner-family map](cleanup-recovery.md#receipt-coverage-map) for continuation
and [security engineering](../guides/security-engineering.md) for the reusable
lessons. This page describes current knowledge and limits; it records no new
execution and does not close G1–G7 or SEC-07/18.

## C16 thread retirement and stack pair

Owners: [`ReapBatch` / threads](../../crates/catten/src/cpu/scheduler/threads/mod.rs),
[`RetirementList`](../../crates/catten/src/klib/collections/retirement_list.rs),
[`Stacks`](../../crates/catten/src/memory/thread_stack/stacks.rs), `StackSlot`
and the complete thread/context payload.

### Ownership and release order

1. Prepare context and retirement-node storage before generation claim, stack
   allocation or thread publication. Reserve full configured user-stack capacity
   plus the kernel half; retain root/slot/classification with that reservation.
2. Keep run-queue membership in the owning thread and unlink it before retirement.
   Self-exit retains its context through outgoing save and architecture ownership
   handoff. A returned worker marker cannot establish that handoff completed.
3. Stage exact captured generations in their admitted nodes. Owner-LP reapers run
   pinned with IRQs enabled and reject masked production entry. Boot-fixture
   adapters cover only never-admitted contexts.
4. Detach ranges and leave local table/staging guards before invalidation and
   one-shot physical pair release. Preserve unrelated caller masks/guards.
5. Confirm both stack halves, then explicitly dispose complete thread metadata
   outside local guards, before releasing its retirement node and original charge.
   Failed/started pairs remain terminal in their existing nodes.

The thread fallback retains context, wait state, subscriptions, sponsors, stack,
root and retirement storage. It performs no callbacks, waiter cancellation,
usage/frame refund or metadata destruction. Provisional stack/growth fallback
also retains reservation-only prefixes; ordinary unused cancellation is explicit
and consuming. The exact kernel-range owner replaces scalar range reconstruction
and allocating guard-address bookkeeping.

Master-table growth has its own [byte/slot admission](thread-table-admission.md).
A complete candidate is prepared before publication guards, exact entries and
slot metadata move into it without allocator work, and its receipt retains the
empty old buffers through explicit post-guard deallocation/refund. Abandonment
retains those buffers and their original charge. Thread exit does not refund
retained capacity. This metadata operation supplies no physical completion or
thread-retry authority; the IRQ-masked copy and wider callers still need latency
and outer-context qualification.

Contracts: [thread retirement](thread-retirement.md), [stack admission](stack-admission.md),
[thread metadata](thread-metadata-admission.md) and
[kernel frame retirement](kernel-frame-retirement.md).

### Readiness and switch boundaries

Per-LP reapers register against their retirement head and recheck productive
readiness. Started/failed pairs do not signal permission to retry. Deferred
notifications remain on their producer LP until local guards leave. A wait must
flush that delivery before publishing Blocked; every actual switch also completes
owner-LP staging/deferred delivery on its incoming stack, including fresh entry.
Restore incoming FP/SIMD state after callbacks. Readiness never permits physical
release without exact-generation quiescence.

Scheduler queue selection/migration uses inline owning links without temporary
load vectors. Wait preparation captures the generation and sponsor outside local
LP/table guards; early wakes remain hints. Kernel timer sponsors are shared by
address-space generation. Isolated pressure fixtures prepare a private trusted
sponsor before thread publication rather than resizing or saturating a live one.

Contracts: [run queues](scheduler-run-queues.md),
[scheduler waiter budgets](scheduler-waiter-budgets.md) and
[scheduler timer budgets](scheduler-timer-budgets.md).

### Qualification limits

Guarded abandonment, exact-node retirement, readiness and switch controls exercise
their specified local boundaries. They do not qualify every constructor/syscall
rollback, callback, active parent destructor or enclosing IRQ/allocator context.
Shared principal backing and thread metadata admission are implemented; other
metadata/general-heap accounting and sustained pressure remain R7 work.

Historical exact-exit and stack-lease timeouts retain their attribution limits.
See the [current incident list](security-remediation.md#unresolved-incidents).
Terminal retention preserves safety and consumes capacity; it supplies no
supervisor reconciliation, physical retry or restart authority.

## C18 publication rollback and detached memory

Owners compose [`PreparedTransfer`](../../crates/catten/src/memory/object/transfer.rs),
[backing](../../crates/catten/src/memory/object/backing.rs),
[copy pins](../../crates/catten/src/memory/object/copy.rs),
[IPC transactions](../../crates/catten/src/ipc/transaction.rs),
[message delivery](../../crates/catten/src/ipc/message.rs),
[completion records](../../crates/catten/src/completion/record.rs),
[namespace lifecycle](../../crates/catten/src/completion/namespace.rs) and
[CQ preparation](../../crates/catten/src/completion/delivery/preparation.rs).
Service-local operations use the same containing-transaction principle while
retaining their protocol-specific cleanup policy.

### Publication and disposal contracts

| Boundary | Required ownership and ordering | Qualification limit |
| --- | --- | --- |
| Capability/payload publication | Prepare admitted authority and payload nodes before mutation. Retain exact namespace identity and original accounts. Publish by relinking under subsystem serialization; dispose losing storage after local guards. | Active reservation/escrow parents, other consumers and enclosing lifecycle/IPC/IRQ contexts need their own checks. |
| Memory transfer and hidden delivery | Keep source escrow, source pin, destination preparation and private copy backing in the same transfer. Restore escrow/release pin/clear the exact source fence under one memory hold; expose attachments only under receive/first-result observation. [Scalar attachment submission](ipc-receive.md#scalar-attachment-submission) prepares outside IPC under both exact roots; a complete single-send owner retains all fields on abandonment and finishes explicitly after local guards. | `PreparedTransfer::drop` remains active, including private-copy release. Vector, reply, supervisor and other enclosing destructors still need qualification; the single-send parent does not qualify its child globally. |
| Memory backing and pin/snapshot preparation | Reserve metadata/buffer bytes and prepare snapshots fallibly before pin publication. Keep original backing and charges in `ChargedFrames`; ordinary rejection and last-pin/cancelled-copy release consume it explicitly. | Backing abandonment retains charge-only/private/published fields. Physical failure is terminal; retained frames cannot reconstruct a retry transaction. |
| Mapping and scratch retirement | Prepare nodes before leaf publication, retain installed prefixes and exact peer leases, and complete invalidation/scratch before pin release. Charge allocated/reserved metadata until actual deallocation. | A scratch/range check is not reservation identity. Uncertain cleanup retains the whole dependency set, not merely frame addresses. |
| IPC calls, replies and cancellation | Prepare records, endpoint growth, authority and loan receipts outside IPC. Joint commit keeps moved/returned outputs hidden; exclusive claims retain both roots through post-guard revocation and publication. | Claimed fronts, failed revocation, source close and pending-call borrow cleanup retain their existing typed fences and fail-closed policy. |
| Completion/CQ construction and retirement | Capture original buffer, target and admission before fallible record/list/node/producer preparation. Prepare namespace accounts before publication. Remove nodes/authority/backlog under serialization; dispose after unlock, payload/node before original charge. | Active observer/token/source/timer/final-handle destruction, whole-parent contexts, aggregate account bounds and callback execution remain separate. |
| Timer/watch/waiter preparation | Keep sponsor, weak target, candidate/selected source, entry and original charge in one preparation before allocation. Initialize once; explicitly finish losing/unused preparation after guards. Retain partial fields on abandonment. | A subscription cancellation affects the subscription, not the operation. Weak allocation lifetime and all enclosing contexts remain relevant. |
| Entropy and descriptor ownership | Move caller-funded backing through the read; keep pending reply/mapping and output borrows through checked completion/cancellation. Arm the complete device attempt before descriptor publication and retain it after uncertainty. | A timeout or halfword-access correction gives no reset/drain authority. Partial setup, shutdown, device faults and forced termination need their own qualification. |
| TLS/status/reactor operations | Retain pending RPC before session/socket/buffer destruction, use original deadlines and confirmed output prefixes, and observe CQ hints through exact owners. Status/deployment rotate four positions and drain all phases before acknowledgement. Deployment cancels incomplete input while continuing accepted controller/output work under its original budgets; terminal owners enter consuming logical close. | Credential expiry, UTC loss or a status deadline cannot prove accepted work cancelled. Idle-client ARM shutdown has a focused control; synchronous rendering, full-pool flooding, stalled controller/output work and receiver-held cleanup remain separate. Local erasure or logical close is no physical-release proof. |
| Deployment launch records | [One reserved fixed slot](scoped-deployment-launch.md#deployment-record-admission) fences capacity and principal before construction, then consumes its exact serial for cancellation or allocation-free publication. Abandoned preparation retains the fence; preparing retirement returns Pending. Completed retirement removes its complete record before guard exit and post-guard disposal. | Releasing an unused bookkeeping slot refunds no root/backing charge and supplies no uncertain-launch retry authority. Fixed registry admission does not qualify all constructor rollback/destructor contexts, concurrent EL0 pressure or general heap accounting. |

Current contracts: [capability admission](capability-admission.md),
[memory retirement](memory-object-retirement.md),
[memory-capability nodes](memory-capability-node-admission.md),
[mapping nodes](mapping-node-admission.md), [scratch](scratch-admission.md),
[snapshot buffers](snapshot-buffer-admission.md),
[IPC records](ipc-record-budgets.md),
[completion records](completion-record-budgets.md),
[CQ budgets](completion-queue-budgets.md),
[observer lists](observer-list-admission.md),
[entropy](entropy.md), [TLS](management-tls.md) and
[status observation](status-observation.md).

### Review and acceptance

- Follow the complete parent operation across preparation, commit, result
  observation and disposal. Include implicit field destructors and temporary
  guards on every rejection path.
- Exercise submission/allocation/mapping rejection, cancellation/drop and
  returned-capability cleanup where practical. Distinguish unstarted rejection,
  accepted uncertainty and terminal physical failure.
- Check original charges through weak/detached lifetime, transfer, sponsor
  retirement, platform promotion and successor generation reuse. Refunding one
  component cannot establish complete cleanup.
- Keep notification and metadata disposal outside producer guards, preserving
  outer IRQ state. Record exact physical completion before any reuse/refund.
- Close remaining G1/G4 caller/allocation inventories and G7 integrated pressure
  separately. Local guarded controls, finite models and passing repeat boots
  cannot establish universal destructor safety or explain unresolved failures.

Update these contracts when behavior changes. Store a significant incident or
experiment in a focused dated report; routine validation belongs in its change
summary. Keep this page free of sequential remediation entries.
