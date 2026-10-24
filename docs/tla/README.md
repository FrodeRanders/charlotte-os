# Executable TLA+ Models of CharlotteOS

This directory contains forty-four finite, executable specification modules:

| Subsystem | Module | Fast configuration |
|---|---|---|
| Principal-history candidates and permanent published storage | `CharlottePrincipalHistory.tla` | `CharlottePrincipalHistory_small.cfg` |
| Thread context/node admission and original-charge disposal | `CharlotteThreadMetadata.tla` | `CharlotteThreadMetadata_small.cfg` |
| Aggregate principal backing across instances and retained generations | `CharlottePrincipalBacking.tla` | `CharlottePrincipalBacking_small.cfg` |
| Controller server preflight evidence, original budget and independent replies | `CharlotteControllerRelay.tla` | `CharlotteControllerRelay_small.cfg` |
| Deployment listener creation, setup and idle accept | `CharlotteDeploymentListener.tla` | `CharlotteDeploymentListener_small.cfg` |
| Namespace token selection, peer close and Pending revalidation | `CharlotteNamespaceDrain.tla` | `CharlotteNamespaceDrain_small.cfg` |
| Logical socket-close ownership, acknowledgement and admission | `CharlotteDeploymentClose.tla` | `CharlotteDeploymentClose_small.cfg`, `CharlotteDeploymentClose_blocking.cfg` |
| Deployment response prefix receipts and original output budgets | `CharlotteDeploymentOutput.tla` | `CharlotteDeploymentOutput_small.cfg` |
| Deployment controller ownership and independent reply observation | `CharlotteDeploymentControl.tla` | `CharlotteDeploymentControl_small.cfg` |
| Deployment input isolation and original deadlines | `CharlotteDeploymentInput.tla` | `CharlotteDeploymentInput_small.cfg` |
| Endpoint IPC and memory transfer | `CharlotteIPC.tla` | `CharlotteIPC_small.cfg` |
| IPC attachment rollback and reply waits | `CharlotteIpcTransaction.tla` | `CharlotteIpcTransaction_small.cfg` |
| Captured capability admission and hidden IPC delivery | `CharlotteIpcPublication.tla` | `CharlotteIpcPublication_small.cfg` |
| Endpoint readiness and close observers | `CharlotteEndpointObservers.tla` | `CharlotteEndpointObservers_small.cfg` |
| Local NS publication classes and exact death watches | `CharlotteNamePublication.tla` | `CharlotteNamePublication_small.cfg`, `CharlotteNamePublication_exhaustion.cfg` |
| Boot publication and conditional protected installation | `CharlotteBootTrust.tla` | `CharlotteBootTrust_fixture.cfg`, `CharlotteBootTrust_contract.cfg` |
| Completion queues and waits | `CharlotteCQ.tla` | `CharlotteCQ_mini.cfg` |
| Timed-wait wake races | `CharlotteTimedWait.tla` | `CharlotteTimedWait_small.cfg` |
| Fallible checked waits and retained operations | `CharlotteCheckedWait.tla` | `CharlotteCheckedWait_small.cfg` |
| Scheduler thread lifecycle | `CharlotteScheduler.tla` | `CharlotteScheduler_small.cfg` |
| Generation-bound thread joins | `CharlotteThreadJoin.tla` | `CharlotteThreadJoin_small.cfg` |
| Demand-grown stack budgets | `CharlotteStackGrowth.tla` | `CharlotteStackGrowth_small.cfg` |
| Reusable ASID generation identity | `CharlotteAddressSpace.tla` | `CharlotteAddressSpace_small.cfg` |
| Root leases, retirement and failed cleanup | `CharlotteRootRetirement.tla` | `CharlotteRootRetirement_small.cfg` |
| AArch64 hardware-ASID recycling | `CharlotteHardwareAsid.tla` | `CharlotteHardwareAsid_small.cfg` |
| Deferred interrupt-route reuse | `CharlotteInterruptRoute.tla` | `CharlotteInterruptRoute_small.cfg` |
| Service publication and teardown | `CharlotteServiceLifecycle.tla` | `CharlotteServiceLifecycle_small.cfg` |
| Unified tagged capability namespace | `CharlotteCapability.tla` | `CharlotteCapability_small.cfg` |
| Authorization policy and capability issuance | `CharlotteAuthorization.tla` | `CharlotteAuthorization_small.cfg` |
| DMA pinning and SMMUv3 teardown | `CharlotteDMA.tla` | `CharlotteDMA_small.cfg` |
| DMA unit admission and uncertain map/unmap ownership | `CharlotteDmaOperation.tla` | `CharlotteDmaOperation_small.cfg` |
| Raft election and durable voting | `CharlotteRaft.tla` | `CharlotteRaft_small.cfg` |
| Raft log replication and commit safety | `CharlotteRaftLog.tla` | `CharlotteRaftLog_small.cfg` |
| Raft delayed responses and confirmed progress | `CharlotteRaftResponse.tla` | `CharlotteRaftResponse_small.cfg` |
| Raft read commit/application prerequisites | `CharlotteRaftRead.tla` | `CharlotteRaftRead_small.cfg` |
| Raft joint membership and decommissioning | `CharlotteRaftMembership.tla` | `CharlotteRaftMembership_small.cfg` |
| Raft pre-membership join admission | `CharlotteRaftJoin.tla` | `CharlotteRaftJoin_small.cfg` |
| Raft snapshot installation and recovery | `CharlotteRaftSnapshot.tla` | `CharlotteRaftSnapshot_small.cfg` |
| Raft snapshot sender episodes and chunk order | `CharlotteRaftStream.tla` | `CharlotteRaftStream_small.cfg` |
| Application snapshot rejection and recovery | `CharlotteSnapshotRestore.tla` | `CharlotteSnapshotRestore_small.cfg` |
| Joint cluster placement, readiness, drain, and DSR ingress | `CharlotteClusterIngress.tla` | `CharlotteClusterIngress_small.cfg` |
| Filtered placement control, dwell, and reservations | `CharlottePlacementControl.tla` | `CharlottePlacementControl_small.cfg` |
| Remote-call identity, uncertainty, and bounded deduplication | `CharlotteRemoteCall.tla` | `CharlotteRemoteCall_small.cfg` |
| Reliable-message restart/retry sessions | `CharlotteReliableMessage.tla` | `CharlotteReliableMessage_small.cfg` |

The [23 October table/stack follow-up](../reports/audits/2026-10-23-security-principal-structure.md)
extends aggregate backing to unequal charged footprints: one-page object/table
units and a two-page reserved stack capacity. Growth uses that prepaid capacity.
One root slot can restart under another principal. Six negative controls include
double-charged growth and abandonment refund, alongside generation-only admission,
early retirement refund, move-based sponsorship changes and late release against
a replacement. Object counts, node pools, metadata, hardware and pressure fairness
remain outside this projection.

The [22 October controller-server review](../reports/audits/2026-10-22-security-controller-relay.md)
checks preflight/mutation beside a query, original budgets, independent readiness
and receiver-retained copies. Only checked preflight evidence permits mutation.
Negative controls treat unknown as missing, restart time, park beside a ready
reply or replay mutation. The model starts after admission; byte quotas, actual
record/signature validation, shutdown, legacy handlers and wall-clock fairness
remain outside its scope.

The [22 October listener review](../reports/audits/2026-10-22-security-deployment-listener.md)
checks two fixed setup positions through creation, listen and idle accept. Each
phase accepts once; setup retains one budget and parking covers every owner.
An admitted idle accept has no deadline. Binding/producer delivery and atomic
turns are assumptions; byte/attachment validation, shutdown, dynamic cross-phase
capacity, TCP correctness and wall-clock fairness are outside this projection.
Negative controls replay accepted work, restart the setup budget or park on one
pending call while another reply is ready.

The [22 October namespace-selection review](../reports/audits/2026-10-22-ipc-namespace-selection.md)
checks one fenced namespace drain with two nonreused tokens. Peer close can
consume a selection before preparation. Missing selections rescan; a still-live
claim remains Pending. Negative controls return stale Pending or skip live work.
Enclosing exact-root, loan/backing and physical cleanup proofs remain inputs.

The [22 October synchronous-close review](../reports/audits/2026-10-22-security-synchronous-close.md)
extends the same close model with one blocking owner and terminal unstarted
admission retention. A negative control restores runnable work after retention,
which must violate `NoReentry`. Accepted replay remains covered by the existing
negative control. Actual deadline durations, runtime field disposal and
containing Drop latency remain outside the finite abstraction.

The [22 October logical-close review](../reports/audits/2026-10-22-security-deployment-close.md)
adds two already owned sockets, rejected admission backoff, accepted-once close,
checked Started/Unavailable receipts, original budget and owner retention through abort.
Unsafe accepted replay and position release before acknowledgement must fail.
Bound reply sources and atomic turns are assumptions; actual socket setup,
wire decoding, nonreused ID allocation/reaping, dynamic admission, shutdown/destructor timing and physical release
remain outside this projection.

The [22 October output review](../reports/audits/2026-10-22-security-deployment-output.md)
adds two admitted outputs with two abstract prefixes each. Positive producer
receipts alone advance an offset and admit its next prefix. Wakes do not grant
progress, uncertain termination permits no replay, and one original deadline
survives all chunks. Replay and budget-reset negative controls must fail.
It abstracts producer delivery and atomic turns; actual byte framing, capacity,
kernel scheduling, saturated transport and socket close remain outside it.

The [22 October controller reactor review](../reports/audits/2026-10-22-security-deployment-control.md)
adds a separate two-call projection of bound readiness, retained owners,
per-operation expiry and receiver-owned backing after cancellation. Unsafe
single-reply parking and accepted-request replay must fail their named checks.
This covers controller observation; server execution and response/transport
latency remain outside the abstraction.

The [22 October deployment input review](../reports/audits/2026-10-22-security-deployment-input.md)
adds a four-slot finite projection: aggregate buffered-byte bounds, original
budgets and a ready healthy input's maximum intervening turns. Cursor and
deadline negative controls must violate their named invariants. It assumes
atomic input turns and producer/CQ delivery, and omits controller/send/close
latency, new admissions, TCP handshakes and saturated-client fairness.

These are abstract safety models checked with TLC. They are useful for finding
protocol and state-machine errors, but they are not a proof of the Rust
implementation. In particular, an action is atomic in TLA+ by definition.
Establishing that a Rust operation implements the same atomic transition will
require identified linearization points and a refinement argument.

Earlier scoped implementation-to-model reviews include the
[18 October boot-trust synchronization](../reports/audits/2026-10-18-boot-trust-model.md).
The preceding [NS publication synchronization](../reports/audits/2026-10-18-name-publication-model.md).
The preceding [DMA operation synchronization](../reports/audits/2026-10-18-dma-operation-model.md).
Earlier October reviews cover
[IPC publication](../reports/audits/2026-10-18-ipc-publication-model.md) and
[root retirement and checked waits](../reports/audits/2026-10-18-security-model-sync.md).
The earlier [6 September review](WEEKLY_SYNC_2026-09-06.md) covers cluster
ingress. The action-level map is [`CONFORMANCE.md`](CONFORMANCE.md).

## Running the models

Download a TLA+ tools release and run:

```sh
docs/tla/check.sh /path/to/tla2tools.jar
```

Select one module for a focused run with a second argument, for example
`docs/tla/check.sh /path/to/tla2tools.jar CharlottePrincipalHistory`. An unknown
module rejects without running configurations. CI uses the unfiltered suite.

Alternatively, set `TLA2TOOLS_JAR`. The script:

- runs all forty-seven complete fast configurations and 143 expected-failure
  principal-history, thread-metadata, principal-backing, controller-relay, deployment-listener, namespace-drain, deployment-close, deployment-output, deployment-input, deployment-control, IPC-transaction, IPC-publication, endpoint-observer, name-publication, boot-trust, CQ-buffer, timed-wait, scheduler, thread-join, stack-growth,
  checked-wait, domain-abort, address-space, root-retirement, hardware-ASID, interrupt-route, service-lifecycle, DMA, DMA-operation, authorization,
  Raft-join, Raft-response, Raft-stream, snapshot-restore, cluster-ingress, and reliable-message regression configurations;
- enables TLC action coverage;
- places checkpoints and traces in a temporary directory;
- rejects structural TLC warnings in addition to invariant failures.

The `TLA+ safety models` CI job runs this same harness with TLA+ Tools v1.7.4.
It pins and verifies the tools JAR by SHA-256 before checking both the safe
models and every expected-counterexample regression.

The individual commands are:

```sh
java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteIPC -config CharlotteIPC_small.cfg -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteIpcTransaction -config CharlotteIpcTransaction_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteIpcPublication -config CharlotteIpcPublication_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteEndpointObservers -config CharlotteEndpointObservers_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteCQ -config CharlotteCQ_mini.cfg -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteTimedWait -config CharlotteTimedWait_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteCheckedWait -config CharlotteCheckedWait_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteScheduler -config CharlotteScheduler_small.cfg -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteThreadJoin -config CharlotteThreadJoin_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteStackGrowth -config CharlotteStackGrowth_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteAddressSpace -config CharlotteAddressSpace_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRootRetirement -config CharlotteRootRetirement_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteHardwareAsid -config CharlotteHardwareAsid_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteInterruptRoute -config CharlotteInterruptRoute_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteServiceLifecycle -config CharlotteServiceLifecycle_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteCapability -config CharlotteCapability_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteAuthorization -config CharlotteAuthorization_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteDMA -config CharlotteDMA_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteDmaOperation -config CharlotteDmaOperation_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRaft -config CharlotteRaft_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRaftLog -config CharlotteRaftLog_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRaftMembership -config CharlotteRaftMembership_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRaftJoin -config CharlotteRaftJoin_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRaftSnapshot -config CharlotteRaftSnapshot_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteClusterIngress -config CharlotteClusterIngress_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlottePlacementControl -config CharlottePlacementControl_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRemoteCall -config CharlotteRemoteCall_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteReliableMessage -config CharlotteReliableMessage_small.cfg \
  -workers auto -coverage 1
```

Run them from `docs/tla`.

Generated `states/`, `*_TTrace_*.tla`, and `*_TTrace_*.bin` files are ignored.
If a counterexample is worth retaining, reduce it to a documented regression
scenario rather than committing TLC working data.

## What “checked” means

For a particular configuration, TLC enumerates every reachable abstract state
within the configured finite bounds and evaluates the listed invariants. It
does not establish:

- correctness for arbitrary bounds;
- liveness or fairness;
- equivalence between the model and the kernel implementation;
- absence of failures omitted from the abstraction;
- machine-level atomicity of the Rust code.

The `TypeOK` invariants catch malformed abstract states. Action coverage is
enabled so that a passing invariant cannot conceal an important transition
that was never enabled.

## IPC model

`CharlotteIPC.tla` models:

- a system-wide monotonic capability-handle namespace;
- per-address-space capability possession;
- endpoints and bounded queues;
- attenuated connection capabilities;
- scalar send, call, receive, reply, cancellation, and result observation;
- reply-token creation and one-shot consumption;
- memory-object creation;
- copy, move, read-borrow, and write-borrow calls;
- endpoint closure and domain teardown.

### Memory ownership

A memory object records an owner and one of these abstract states:

| State | Meaning |
|---|---|
| `Owned` | The ordinary owner has access. |
| `Moved` | Ownership has moved to the receiving address space. |
| `BorrowedR` | The lender retains ownership and one or more readers are recorded. |
| `BorrowedW` | Writable access is temporarily assigned to one borrower and removed from the lender. |

`MemoryCreate` makes the transfer transitions reachable from `Init`. The fast
configuration's TLC coverage must show non-zero counts for:

- `ScalarCallMove`;
- `ScalarCallCopy`;
- `ScalarCallBorrowRead`;
- `ScalarCallBorrowWrite`;
- `ReplyReturnMemory`;

A call queues an internal reply-token identity, but that identity is not
server authority. The receive transition installs the reply capability in the
server's capability table. Every reply action automatically revokes an
attached borrow, matching `complete_reply`.
Cancellation, endpoint close, and domain teardown also revoke outstanding
borrows as part of the same abstract transition that completes the call.

## IPC attachment transaction model

`CharlotteIpcTransaction.tla` refines the attachment work deliberately
collapsed in the base IPC model. Concrete vector calls carry multiple memory
entries, while scalar calls can carry a connection and copied memory; the
model composes those paths into a stronger cross-registry rollback obligation
without claiming that one current syscall carries every attachment form. It
requires a failed preparation to roll back every moved entry, loan, and
attached connection. A committed call retains its loan when a reply wait times
out because timeout is only a non-terminal observation; reply completion
releases the loan.

The rollback negative configuration leaves one moved entry and the attached
connection in their target registries and must violate `RollbackRestoresAll`.
The timeout negative configuration releases the loan while the call remains
pending and must violate `TimeoutRetainsLoan`.

### IPC invariants

The aggregate `Invariants` predicate includes:

| Invariant | Property checked |
|---|---|
| `TypeOK` | Every variable remains inside its declared abstract type and bound. |
| `TokenImpliesCall` | A consumed live token refers to a completed call. |
| `ActiveTokenHasCall` | An unconsumed token refers to a live incomplete call. |
| `QueueBounded` | Endpoint queues do not exceed capacity. |
| `ClosedEmpty` | Closed endpoint queues are empty. |
| `ExclusiveBorrowWrite` | A writable borrow has distinct lender and borrower and no readers. |
| `NoMixedBorrows` | Read and write borrowing cannot coexist. |
| `TokenServer` | Reply-token capabilities reside in the designated server AS. |
| `ReplyCapabilityAfterDelivery` | A unique reply capability exists exactly for a live token whose request has been received. |
| `NoDanglingMemCaps` | A moved object is not named by a different AS's memory cap. |
| `TokenCallValid` | Active tokens refer to valid pending calls. |
| `BorrowBackedByActiveToken` | Every remaining read or write borrow is justified by a live reply token. |

The fast configuration uses two ASIDs, six system-wide capability handles,
one endpoint, two tokens, two calls, two memory objects, and queue capacity
one. This is deliberately small enough for complete breadth-first exploration
while still enabling every memory-transfer action.

### Findings produced while developing the model

The model exposed these protocol requirements:

1. Caller teardown must invalidate a reply token even after the server has
   dequeued the request. The current Rust `cancel_queued_call` implementation
   scans reply tokens by pending-call ID and handles this case.
2. A sentinel representing “no result” must not collide with a valid result.
   Rust uses `Option<ReplyValue>`; the model uses an out-of-range sentinel.
3. Every ordinary reply path must restore an attached borrow before publishing
   the result; the Rust `complete_reply` helper performs this operation.
4. Endpoint close and domain teardown must revoke borrows attached to calls
   that they terminate.

These are specification findings. They are not, by themselves, proof that all
corresponding implementation paths are correct.

## Capability and IPC publication model

`CharlotteIpcPublication.tla` separates shared capability authority from payload
delivery. One transaction prepares owning memory and a delegated connection,
validates the whole batch, then publishes both as hidden attachments in a
request queue or an unobserved reply. Application lookup remains unavailable
until receive commit or first reply observation. Readiness does not deliver
either resource. Failed receive admission and result writing preserve the
queued attachments; repeated reply observation cannot revive a resource the
caller has already closed. Hidden cancellation consumes the container's grants
and never restores a committed move to the sender.

Prepared storage, staged admission, move escrow, a backing pin and private copy
backing remain distinct. Unstarted rollback restores the exact source entry
without fresh admission, including at the source namespace's record ceiling.
Authority removal detaches charged records; explicit post-guard disposal
refunds their captured accounts. A retiring source can be restored for teardown,
while a removed/replaced namespace cannot be recreated by a late token.

The finite instance has two numeric namespaces, two generations each, four
serial values, a three-record namespace limit and a six-record node limit.
Successors install unrelated sentinel payloads at reused handles 1 and 2.
Ten separate unsafe configurations exercise guessed memory/connection use,
partial batch publication, serial reuse, successor corruption by late
cancellation/restoration, premature retained-node refund, delivery from
readiness, repeated publication, and rollback that needs fresh admission.

The model uses move/copy owning memory; loan revocation stays in the earlier
transaction model and concrete tests. It omits physical backing/byte budgets,
platform headroom, rights attenuation, actual allocator/lock/IRQ contexts,
endpoint reference counts, and multiple concurrent transactions. Receive's
speculative authority is opaque while IPC is exclusively borrowed. Namespace
detachment assumes confirmed payload retirement; it does not prove root
quiescence or cancellation of hardware access. No liveness or complete Rust
refinement proof follows from this bounded projection.

## Completion-queue model

`CharlotteCQ.tla` models:

- operation submission, completion, failure, deferred cancellation, and observation;
- CQ ring capacity and a non-lossy kernel backlog;
- draining one or all available entries;
- a monotonic work-generation counter;
- waiter registration and integrated wakeup;
- timer submission and timer completion;
- result observation and reclamation.

Each action assigns a given primed variable at most once. `PostCompletion`
atomically posts to the ring or backlog, increments the generation, and wakes
a registered waiter. `DrainOne` computes the drained ring and optional backlog
refill as one `cqRings'` value.

Submission starts directly in `InFlight`, as `Completion::new` does.
Cancellation changes it to `CancelPending`; the later completion is forced to
the cancelled status. Draining a CQ entry is modeled separately from polling a
completion capability, because the Rust paths are independent.

Buffered submission additionally marks the buffer as kernel-owned. Neither a
cancellation request nor CQ draining releases that loan; only terminal
completion does. The negative buffer configuration deliberately releases it at
`CancelPending` and must violate `NonTerminalBufferRemainsLoaned`.

### CQ invariants

| Invariant | Property checked |
|---|---|
| `TypeOK` | Every variable remains inside its declared abstract type and bound. |
| `CompletionIsTracked` | Until its CQ entry is drained, every terminal completion remains in the ring or backlog. |
| `RingBounded` | Ring entries do not exceed ring capacity. |
| `WaiterValid` | A registered waiter has a valid owner. |
| `NoLostWakeups` | A waiter either has the current generation or work is visible. |
| `CqOwnerValid` | An open CQ has a valid owner. |
| `OpCqValid` | Every active operation refers to a CQ. |
| `TimerFiredImpliesCompleted` | A fired timer's operation is completed or reclaimed. |
| `NonTerminalBufferRemainsLoaned` | An in-flight or cancel-pending buffered operation retains kernel ownership of its destination. |
| `KernelBufferHasOneLiveOperation` | Every kernel-owned buffer is justified by a live operation. |

`StateConstraint` bounds the otherwise unbounded generation counter for TLC.
This is a model-checking bound, not a statement that the implementation's
counter cannot wrap.

## Endpoint-observer model

`CharlotteEndpointObservers.tla` separates endpoint readability waiters from
endpoint-close completion watchers. Message arrival signals only readability;
endpoint closure signals both classes. Its principal invariant,
`CloseSignalImpliesClosed`, makes a close completion authoritative evidence of
the endpoint lifecycle rather than a generic readiness notification.

`CharlotteEndpointObservers_unsafe.cfg` enables `UnsafeMessageWake`, which
models the former shared observer queue. The model runner requires this
configuration to violate `CloseSignalImpliesClosed`: an ordinary message wakes
the close watcher while the endpoint remains open. The repaired configuration
is checked positively, and the IPC kernel self-test follows the same concrete
trace by sending a message while a close watch is armed.

## Checked-wait model

`CharlotteCheckedWait.tla` begins with one accepted operation and its live
memory borrow. It splits watchdog/source preparation, admission rejection,
registration, the final park recheck, wake delivery, and result observation.
Time can advance during preparation. An unrelated wake or a changed source
episode returns to the same operation with its original absolute deadline.
Rejected admission and timeout retain the pending owner and loan. A completed
result's returned resource remains hidden until explicit observation.

The small instance uses a two-tick budget and two source epochs. Six negative
configurations check premature release on admission rejection, replay after
timeout, deadline extension, adoption from a wake, parking without the final
recheck, and classifying admission rejection as timeout. Abandonment retains
the complete operation and any subscription storage. It supplies no recovery
or physical cleanup permission.

The model assumes a qualified scheduled caller and producer notification when
the captured source episode ends. It omits IRQ/lock mechanics, sponsor quotas,
subscription allocator internals, and the outer domain-abort policy. It checks
safety, without assuming eventual completion or eventual admission.

## Root-retirement model

`CharlotteRootRetirement.tla` follows one reusable ASID through two generations,
with two possible operation leases and a bounded invalidation retry count.
Staged close fences new ordinary leases, preserves existing and abandoned
leases, and detaches only after device, IPC and memory cleanup have completed.
Failed invalidation retains root, slot, metadata and original charges together.
Custody transfers that complete owner before an explicit retry; abandonment
has no continuation that reconstructs authority from diagnostic identity.

After acknowledged invalidation, metadata is released before one physical
walk. Successful release refunds the backing charge. A rejected physical
release retains the original charge and cannot be retried, while slot
completion allows the next generation to be allocated. Five negative
configurations weaken lease retention, invalidation ordering, physical
once-only release, terminal abandonment and retained-charge accounting.

Thread quiescence, hardware acknowledgements and subsystem completion are
explicit boundary assumptions. The model does not prove their producers,
allocation under guards, intermediate frame-walk interruption, registry ticket
reuse, aggregate budgets or restart authorization. The older
`CharlotteAddressSpace` model continues to check stale-handle rejection.

## Scheduler lifecycle model

`CharlotteScheduler.tla` separates scheduler-table phase from physical
execution (`onCpu`). This distinction represents the interval after a thread
blocks or is remotely aborted but before its owning LP switches away from the
thread's stack. Remote abort is split into `RequestRemoteAbort` and
`RetireRemoteAbort`: the requester records the owner and signals it, while only
the owner can switch off and retire the context. Wake and dispatch reject an
abort-requested generation. `Dead` is the LP-local deferred-reaping interval;
only `Reap`, while off-CPU, makes the reusable slot absent.

The model also represents domain abort and address-space destruction.
`BeginDomainAbort` moves the address space to `Aborting`, marks every extant
thread for owner-LP retirement or immediate death, and prevents further
`Spawn`. Destruction requires all owned threads to be absent, and an invariant
rejects a destroyed address space containing a physically executing thread.

The checked invariants require one physically executing thread per LP, valid
placement and pinning, live address spaces for on-CPU threads, off-CPU reaping,
owner-CPU abort retirement, no redispatch after an abort request, a doomed
thread population after domain abort, matching blocked-waker generations, and
migration authority for movable threads.

`CharlotteScheduler_unsafe.cfg` deliberately enables `UnsafeRemoteAbort`, an
abstraction of the former immediate cross-LP removal. The model runner requires
this configuration to violate `ReapOnlyOffCpu`; if the old transition stops
producing its counterexample, the suite fails. Thus the repaired model is
checked positively and the original bug trace is retained as an executable
negative regression rather than generated TLC trace files.

`CharlotteScheduler_domain_abort_unsafe.cfg` deliberately publishes a new
sibling after the abort snapshot and must violate `AbortingThreadsDoomed`. The
implementation's generation-aware publication gate makes the snapshot and
thread-table publication mutually exclusive and remains held through the
numeric-TID sweep.

### Cross-LP abort findings

Modeling the physical CPU separately from `ThreadState` exposed two necessary
rules reflected in the Rust scheduler:

1. A remote caller cannot remove or reap a running target. It sets
   `abort_requested` and `abort_owner_lp`, sends a scheduler IPI, and the owner
   LP retires the target after switching away.
2. A target may block, or a waker may race, between request and switch. Run
   queue selection and admission must reject the abort-requested generation;
   retirement is keyed by the recorded owner rather than the transient
   `ThreadState` snapshot.

The authoritative `scheduler-lifecycle` kernel self-test exercises the same
trace deterministically. A spinning target is pinned to LP1 with interrupts
enabled while LP0 requests its abort; LP0 then verifies the pending request
and requires `submit_woken_thread` to reject the abort-requested generation.
The test waits for owner-LP retirement and deferred reaping before it can
report success.

The master table and per-LP deferred-dead lists use separate locks. Their Rust
transition is therefore marked by `RETIREMENTS_IN_FLIGHT`; domain-exit
observers reject that intermediate state. Without the marker, teardown could
observe the thread after master-table removal but before dead-list insertion,
which violated the model's atomic retirement projection and caused a
scheduling-sensitive supervisor panic.

With two threads, two LPs, two address spaces, and two generations, TLC 2.19
completely explored 819,972 distinct repaired-model states (4,308,577 states
generated, depth 22) without an invariant violation.

## Service lifecycle model

`CharlotteServiceLifecycle.tla` now models the current signed-artifact and
two-phase catalog protocol. An untrusted image is rejected before `Load`;
replicated `Prepare` allocates a fresh generation but makes the name
unresolvable; `PublishLocal` installs the node-local connection; and only an
exact generation-fenced `Activate` makes the catalog entry visible. Replacing
an active generation therefore has a bounded unavailable interval, matching
`NameCatalog::CMD_REGISTER` rather than the former atomic-swap abstraction.

The model also covers lookup, generation-fenced unregister, stale activation
and unregister rejection, asynchronous local cleanup, cooperative shutdown,
panic/fault-driven `DomainAbort`, scheduler reaping, and address-space
teardown. Its unsafe configuration removes the
owner/generation fence and must violate
`ReplacementSurvivesStaleUnregister`. Generation allocation is guarded at the
finite bound; the Rust catalog and local name service now likewise fail closed
on generation exhaustion instead of saturating or wrapping.

## Unified capability model

`CharlotteCapability.tla` models the authoritative per-address-space tag table
in `capability.rs`. Allocation and public delegation use fresh monotonically
increasing handles. Removal validates both owner and expected object kind.
A move is split into begin/commit/rollback transitions so TLC explores the
window in which source authority has been removed: commit allocates a fresh
target handle, while rollback alone may restore the exact original handle.

The invariants reject future/unallocated handles, require tags to remain below
the namespace's next serial, and require an active move transaction's source
authority to remain revoked.

## Authorization policy model

`CharlotteAuthorization.tla` is a target contract for policy-controlled
capability issuance. It keeps catalog publication and policy evaluation as
separate logical roles even if the first implementation co-locates them in the
name-service process. Policy administrators set versioned subject/service
rules; service managers publish generation-numbered bindings and a maximum
delegable rights set. A decision is bound to the authenticated subject, exact
rights, policy version, and service generation, then may be redeemed once for
an attenuated capability.

The model deliberately gives policy updates prospective semantics. Updating a
rule fences outstanding unredeemed decisions, but does not close capabilities
already issued. The current kernel has no general derivation tree or
policy-on-use mechanism, so claiming selective retroactive revocation would be
false. Endpoint close, a revocable proxy, or a future kernel lease object is
required for hard revocation.

The safe configuration checks authenticated policy/publication roles,
subject-bound redemption, policy-version and service-generation fencing,
rights attenuation, ticket backing, and replay resistance. Five unsafe
configurations each remove one of those controls and must violate the matching
invariant. The complete fast configuration explores 144,598 generated states,
54,705 distinct states, and depth 12 with TLC 2.19.

The design and implementation sequence are described in
[`../architecture/authorization-policy.md`](../architecture/authorization-policy.md). The local
name service now implements the co-located decision-and-delegation path using
kernel-authenticated domain generations and principals. Bearer-key name
lookup opcodes are retired. Public bootstrap naming cannot resolve or replace
policy-published services. Publication classes and endpoint-death watch
admission are still outside this authorization model.

## Boot trust and installation ordering

`CharlotteBootTrust.tla` distinguishes authenticating a policy from authorizing
its use at boot. The shared verifier accepts either the exact installed record
or a signed direct successor naming its predecessor. Shared `verify_for_boot`
accepts only the exact installed revision/digest, producing `VerifiedBootPolicy`
for kernel preparation. The current boot fixture uses that shared check with
compiled public state. Preparing the recipient
owner checks its public binding. Authority selection precedes bootstrap loading;
platform images and final one-shot publication use that same selected policy.
Manifests borrow the kernel gate's published view. Development trust is an
explicit configuration, never a verification fallback.

The fixture/development instance checks those implemented handoff boundaries.
A separate **conditional contract** assumes a protected pinned context, atomic
rollback-resistant state commit and serialized publication. It checks competing
installers, rejected/interrupted commits, fresh reads after uncertainty, no
enrollment fallback and reboot before/after commit. Those storage, platform and
custody assumptions have no production implementation. A qualified post-interruption
read assumes recovery has settled or fenced the original writer; observing old
bytes alone is no completion proof. `ProtectedBoundary = TRUE`
is a model input, not a supported production selector. Production remains disabled.

Both instances use two boot epochs and eight finite record classes, including
signed siblings, a bad predecessor/gap and invalid authentication. Initial
enrollment has a fixed floor of one. Twelve negative controls cover uninstalled
publication, stale commit, enrollment/development fallback, slot overwrite,
independent manifests, wrong recipient, unauthenticated receipts, publication
after failed commit, an unavailable protected boundary, development-signed
platform fallback and selection overwrite. The conditional contract fences
installation once artifact authority is selected, before bootstrap loading.
Real cryptography, firmware, atomic storage, erasure/custody and physical
lifetime are omitted.
See the [review and evidence](../reports/audits/2026-10-18-boot-trust-model.md).
The [exact boot-receipt follow-up](../reports/audits/2026-10-19-security-boot-receipts.md)
rechecked the then-unchanged models after moving the fixture-local comparison into
the shared API and narrowing the kernel constructor's input.

The [early artifact-authority follow-up](../reports/audits/2026-10-19-security-platform-artifact-authority.md)
adds selection/bootstrap/platform transitions and two required controls.

## Local name publication model

`CharlotteNamePublication.tla` follows the local registry's connection and
persistent endpoint-death watch through preparation, policy publication,
registry replacement, deferred lookup and logical retirement. Public lookup
cannot issue a policy-published connection, including when lookup was queued
before publication. Public replacement requires the same nonzero registrar and
an existing public entry. Authorized lookup consumes an abstract successful
policy decision; both publication classes have policy-store bindings, so an
authorized lookup is not categorically forbidden for public entries.

Watch admission precedes policy mutation. The current entry remains authoritative
on preparation rejection. Requests cannot interleave between policy publication
and registry installation. Live capacity includes one spare preparation watch
for a replacement; watch disposal precedes source close. CQ hints deliberately
may alias any current watch. Only its own confirmed close result can retire its
captured entry, while unexpected results or rejected waits retain owners through
terminal abort. Explicit retirement checks the exact generation and authenticated
registrar or manager; tombstones keep their generation.

Two bounded instances keep checks practical: two names, two principals and two
candidates exercise publication/lookup/watch interleavings at live capacity one;
a separate three-candidate projection reaches generation exhaustion at two.
It omits lookup/watch interleavings already checked in the first instance.
Thirteen negative configurations cover the removed guards. The accompanying
source regression fixes publication beyond the positive `i64` NS result range.

Kernel identity and watch-result authenticity, trusted launch budget preparation,
full-name parsing, actual allocation/IRQ/destructor behavior and physical cleanup
remain boundary assumptions. Capacity one does not represent scan ordering
among several simultaneously live watches. No liveness, 64-entry native saturation or composed
Rust refinement proof is claimed. The [review](../reports/audits/2026-10-18-name-publication-model.md)
records source correspondence, checked bounds, counterexamples and evidence.

## DMA and SMMUv3 model

`CharlotteDMA.tla` separates CPU mappings, IPC loans, coherent and exclusive
DMA pinning, page-table installation, translation revocation, pin release,
driver exit, and memory reclamation. A domain-destruction timeout enters a
quarantined state: mappings and pins remain live because hardware may still
hold a usable translation.

The invariants require every hardware-visible mapping to belong to the same
driver as its memory object, every mapped or transitional object to remain
pinned and unfreed, one domain per requester stream, acknowledged stream
revocation before domain resources can be finalized, and exclusive DMA to
overlap with neither CPU authority, an IPC loan, nor another DMA pin.

Developing the model found two concrete error-path defects. A failed map could
drop its `DmaPin` token without decrementing the memory object's pin count, and
domain destruction ignored failure to install and acknowledge the aborting
stream-table entry before releasing pins. The implementation now rolls
pre-installation failures back, retains an unpublished pinned mapping after an
uncertain invalidation, and quarantines a domain on teardown failure.

`CharlotteDMA_unsafe.cfg` deliberately creates an exclusive pin without the
ownership precondition and must violate `ExclusiveDmaHasNoCpuAuthority`.

## DMA operation model

`CharlotteDmaOperation.tla` supplements the pinning model with one map and its
owned unmap/CPU teardown. Map admission retains the original root, public
capability claim, pending pin and source storage through fallible preparation
and waiting. The same fixed unit must supply both its actual engine and domain
cell. A producer can restore availability before registration, or another
claim can win before wake delivery; every wake rechecks the unit. Busy masked,
nonscheduled and rejecting callers fail admission. An immediate ready claim
needs no readiness wait. An abandoned holder stays fenced.

Map errors grant no replay permission. For unmap, only typed pre-claim busy
rejection rearms the same owner. Started maintenance errors and rejected final
public completion retain the recorded attempt; Drop cannot submit it again.
Backend acknowledgement is separate from successful public completion.
Confirmed DMA completion survives a later CPU-unmap failure, so retrying CPU
teardown performs no additional DMA unmap. Failed shared preparation retains
its CPU mapping without volatile or Rust access. Local CPU teardown cannot
discharge an uncertain DMA pin or original backing charge. A fresh CPU mapping
requires all DMA pins to be gone; an existing coherent mapping keeps its
volatile-access contract.

The bounded instance explores shared/exclusive memory, four caller contexts,
three unit-availability epochs and at most two unmap submissions. Thirteen unsafe
configurations check incomplete-unit claims, false completion from readiness,
premature preparation disposal, map replay, started/final-error unmap rearming,
Drop resubmission, forgotten confirmation, failed-shared access, CPU-only
release of uncertain backing, abandonment, masked waiting and fresh CPU remapping
while a coherent pin remains.

Exact root/sponsor capture, typed backend acknowledgements and confirmed
physical release are boundary inputs. The model omits actual MMIO/cache/IRQ
mechanics, byte and metadata admission, multiple simultaneous public operations,
grant creation/cancellation, and reset-based quarantine recovery. The older
`CharlotteDMA` model continues to check stream ownership and CPU/loan exclusion.
Neither is a composed Rust refinement proof or a liveness result.

## Raft election model

`CharlotteRaft.tla` is the first layer of the consensus specification. It
models a fixed voter set, durable term and vote updates, election retries,
majority formation, higher-term step-down, crashes, restarts, lost requests,
and duplicate grants. A three-voter, two-term configuration explores 22,838
distinct states.

The invariants require volatile state recovered by a running node to match its
durable state, every observed vote to have been durably recorded, every leader
to hold a majority, and at most one leader per term. Log matching, commit
safety, snapshots, and temporal election liveness are not hidden inside this
election model. Membership changes are checked by a separate layer below.

`CharlotteRaftLog.tla` is the second layer. It composes the election layer's
one-leader-per-term guarantee with durable log append, one-entry
`AppendEntries` conflict repair, current-term majority commit, commit
propagation, crash, and restart. Its bounded three-node configuration explores
122,240 distinct states. The invariants check log matching, agreement of
committed entries, leader completeness, and that commit indices never extend
past durable logs.

The log model exposed an important abstraction boundary while it was being
developed: replaying an already matching entry must preserve the follower's
suffix. Only a conflicting entry permits suffix truncation. The Rust
`handle_append_entries` implementation already has that ordering; the model
was corrected to reflect it. Snapshots and membership changes are checked by
the following layers; storage write failures, state-machine application, and
temporal replication liveness remain outside this model.

`CharlotteRaftMembership.tla` models stable and joint configurations, separate
voter/learner roles, old-configuration commitment of JOINT, joint-majority
commitment of FINALIZE, and automatic proposal after every proposed remote
member reaches the JOINT fence. This restores the Graft Java/Rust catch-up
policy in Charlotte. Next-only voters may campaign with both voter majorities;
finalized learners relinquish leadership while remaining members. The finite
three-node, two-entry instance explores 6,232 distinct states. Durable voting,
log freshness and prefix agreement remain assumptions from the other layers.

Separate required coverage exercises a next-only election and a rejected
finalization while a proposed member lags. Three counterexamples weaken the
catch-up, eligible-leader and dual-quorum boundaries. The
[source and upstream comparison](../reports/audits/2026-10-18-raft-membership-parity.md)
records the failed Charlotte role regression and its correction. This model
covers automatic proposal; explicit/manual FINALIZE commands are outside it.

`CharlotteRaftJoin.tla` models the MAC-level admission layer that precedes
joint consensus. A non-member selects one anchor, cannot campaign while
joining, accepts replication only from that anchor, and cannot be promoted
until the old configuration committed its `JOIN` entry and the joiner caught
up to that fence. The negative configuration admits replication from an
arbitrary peer and must violate `JoiningAcceptsOnlySelectedAnchor`. The model
also crashes a joiner, erases its volatile posture, and requires restart to
restore the selected anchor and election suppression from durable admission
state. A second negative configuration forgets that state on restart and must
violate `RestartPreservesAdmission`. The implementation snapshots the joint
membership before clearing the durable fence; volatile-storage deployments do
not attempt dynamic admission.

`CharlotteRaftSnapshot.tla` adds chunked snapshot reception, stale-snapshot
discard, atomic durable installation of the state-machine image and current/
next membership, matching-suffix retention, activation, crash, and restart.
Its fast configuration explores 145,170 distinct states.
The invariants require an installed snapshot and its retained suffix to form
one consistent durable image, prevent snapshot or application progress from
moving backwards, and require restart to restore both the durable state-machine
image and membership before declaring the snapshot applied. The recovered
membership also determines whether the local node is decommissioned.

This layer found two implementation defects. Construction advanced
`last_applied` to the stored snapshot index without restoring its bytes into
the supplied state machine, and a delayed snapshot could replace newer
committed progress. It also exposed that three individually atomic object
writes do not form one atomic snapshot/log update. The implementation now
restores on construction, acknowledges stale snapshots without installing
them, retains a suffix with a matching boundary term, and serializes snapshot
metadata, bytes, and log suffix into one copy-on-write object.

`CharlotteRaftStream.tla` separately checks continuation ownership when different
leaders share a snapshot boundary. Its safe instance explores 400 distinct
states: each partial buffer belongs to its captured leader and RPC term, term
changes discard it, and installation requires strict offsets. Two controls
permit cross-episode continuation or a wrong offset and violate
`InstalledSingleOwner` or `InstalledInOrder`. Host regressions reproduce the
former mixed-byte installation. See the
[19 October stream review](../reports/audits/2026-10-19-raft-snapshot-streams.md).
The model assumes immutable bytes within one sender/term/boundary and abstracts
rejected packets as observations; Rust tests exercise their exact fields.
Authentication, same-episode image replacement and allocation are outside scope.

`CharlotteRaftResponse.tla` checks delayed acknowledgements separately from log
replication. An honest response for an old prefix cannot commit a new prefix at
the same index after re-election. Current-term append and completed-snapshot
successes preserve confirmed progress; failure backoff retains that floor.
Matching admitted source and local-tail checks precede credit. A fixed voter,
learner and pending-joiner projection explores 49,395 distinct states, with four
required counterexamples. Transport authenticity, request correlation and read
lease timing remain separate. The [source regression and evidence](../reports/audits/2026-10-18-raft-response-fencing.md)
record the unsafe commit before correction and the passing repeat.

`CharlotteRaftRead.tla` now checks fresh per-read log fences, including delayed
replies and an external leader replacement unseen by the old node. Admission
appends a new no-op; a prior acknowledgement cannot prove its index. A captured
quorum acknowledgement of that fence can arrive later, so the proof concerns
confirmation after admission rather than authority at reply-delivery time.
The safe instance explores 9,916 distinct states. All fourteen required actions
have positive coverage, including cancellation, application lag, membership
change and domain replacement. Four controls remove current-term commitment,
application, fresh-fence or owner-binding guards. The old receive-time policy
is now a weakened control, not an accepted known source failure.

The [read-fence review](../reports/audits/2026-10-19-raft-read-fences.md) records
the former three-node stale-read schedule rejecting its fresh fence, and the
owned DNS reactor phases. Node/domain identity, term and membership epoch bind
each owner; completion consumes it once. Compaction preserves a committed proof.
Quorum/log/election correctness remains an assumption from the other models.
Allocation, physical cleanup, byte codecs and readiness delivery are outside
this projection. Bounded status/ingress observations retain receive-time
freshness separately; no general lease qualification is claimed.

`CharlotteSnapshotRestore.tla` checks the application acceptance boundary
separately. Unsupported headers and malformed later sections leave application
state, durable snapshots, Raft progress, membership and durable join admission
unchanged. Recovery exposes a node only after restore succeeds. A newer accepted
membership snapshot can complete admission; the original snapshot or membership
excluding this node cannot. Seven negative configurations weaken these checks
individually, including durable publication before successful restore.

The finite instance has two snapshot indices, three membership classes and three
application verdicts. It explores 168 distinct states. Separate streamed/recovery action coverage
requires both cleared and retained join admission; the
[follow-up](../reports/audits/2026-10-18-raft-response-fencing.md) corrects an
assignment-precedence omission in the preceding instance. Parsing is an abstraction
of catalog validation, not a proof of the byte decoder. Streamed restore precedes
persistence. Definite or ambiguous persistence failure stops the calling Raft
thread; fresh recovery validates the whole old or new durable image. This assumes
the existing atomic object-store replacement contract. It proves neither an
application-plus-disk transaction nor concurrent-reader isolation, endpoint
withdrawal, sibling-thread termination, physical cleanup or storage fault handling.
The [source review and evidence](../reports/audits/2026-10-18-snapshot-restore-model.md)
record the corresponding host tests and limits.

## Cluster placement and DSR ingress model

`CharlotteClusterIngress.tla` begins after Raft commits a control-plane
mutation. It models stable and joint membership, deployment generations,
replica replacement, exact-generation readiness, drain intent, discovery
routes, immutable per-router snapshots, VIP advertisement, and flow bindings
to retained policy epochs. This is a deliberate composition layer over the
separate election, log, membership, and snapshot specifications; it does not
repeat Raft's replication mechanics.

The model also treats VIP assignment as committed control-plane state. The
initial assignment represents launch-time bootstrap policy. A later
`CommitIngressAssignment` transition carries a strictly increasing operator
sequence and either assigns or explicitly withdraws the modeled service. The
cryptographic signature, operations-key role, cluster binding, and trusted-UTC
admission checks precede this abstraction's Raft linearization point; the model
checks that only the committed assignment can contribute eligible backends to
subsequent router snapshots.

The safe transition system requires a new flow to use the router's installed
policy while that policy has a fresh local lease. Eligible backends are exactly
the intersection of active members, desired replicas, exact-generation
readiness, and nodes not draining. A router installs a snapshot only when every
member has a discovery route, and re-installing the current version is a
content no-op that renews the lease; the concrete router keeps one request in
flight at a time, so installs cannot reorder. The model deliberately treats
both selection functions as parameters: it checks that a policy names one
non-draining advertiser and that a flow stays pinned to one eligible backend of
its policy version, so the concrete leader-preferred advertiser and
rendezvous-hash backend are valid instances rather than privileged choices.
The model also allows a leased snapshot to lag the latest globally committed
policy during asynchronous propagation; an abstract fair expiry action bounds
that authority. Once expired, the lease drops unbound traffic. When bounded history
evicts an epoch, its flow binding remains pinned but no packet action may
reinterpret that binding through a different snapshot. An explicit assignment
withdrawal, or the departure of the pinned backend from the installed member
set, ends the binding instead of retaining a black hole; `CommitDrain` is the
graceful path that keeps established flows and blocks new ones.

Three negative configurations preserve the cross-layer hazards:

- `CharlotteClusterIngress_stale_snapshot_unsafe.cfg` admits a new SYN through
  an old snapshot after its freshness lease expires;
- `CharlotteClusterIngress_history_unsafe.cfg` evicts an epoch still named by
  a flow and then remaps that flow through the current snapshot; and
- `CharlotteClusterIngress_readiness_unsafe.cfg` treats readiness from an old
  deployment generation as authority for its replacement.

All three are retained regressions. The frame router now gives each complete
snapshot a five-second monotonic lease for VIP advertisement and unbound-flow
admission. A flow whose pinned epoch has left history remains a fail-closed
tombstone instead of falling back to the current snapshot. The model includes
the corresponding stale-new-flow and unretained-flow drop actions without
pretending that asynchronous policy delivery is instantaneous.

## Placement feedback-control model

`CharlottePlacementControl.tla` isolates the controller that precedes a
generation-fenced placement replacement. Raw per-node observations and the
filtered control signal are separate state. Candidate selection reads only the
filtered value; pressure movement requires a stable dwell and an expired
cooldown, while membership or drain can remove a node from the eligible set and
force immediate movement. A second state dimension accounts static resource
promises independently of transient free-memory observations.

The invariants require every admitted promise to preserve the node reserve and
forbid a pressure movement that bypasses dwell or cooldown. Two negative
configurations retain those failure modes. The finite model uses an EWMA alpha
of 1/2 to keep the state graph small; Rust uses 1/4. The coefficient changes
response speed, not the checked authority and sequencing properties.

## Remote-call model

`CharlotteRemoteCall.tla` models the complete caller-node/session/call identity,
target-generation fencing, execution, reply delivery, explicit uncertain
timeouts, duplicate requests, transport settlement, session retirement, and a
bounded completed-result cache. Its contract is deliberately narrower than
global exactly-once execution: a result must remain cached while a request can
still recur, and eviction is permitted only after relmsg settlement or explicit
retirement of an uncertain caller session.

The invariants require stale target generations never to execute, successful
completion to have one execution and a delivered reply, uncertainty never to
masquerade as success, and executed identities to remain cached until safe.
The model corresponds to DNS v4 scalar invocation semantics; the CDNSCL01
client frame and owned local CQ phases are outside this model. Transactional effects,
durable deduplication across DNS reboot, and general remote capability transfer
remain outside it.

## Reliable-message session model

`CharlotteReliableMessage.tla` models session allocation across bounded send
retries and unilateral service restart, together with receive-side session
replacement. A session is the ordered pair `(service generation, retry
epoch)`. The invariants require that distinct logical epochs never share one
wire identity and that accepting delayed traffic cannot move the receiver's
session backwards.

Two retained negative configurations capture the August 2026 review findings.
The former flat `generation + retry` allocation aliases generation N's first
retry with generation N+1's initial session. The former bounded retired-session
window eventually allows an older delayed SYN to replace a newer session. The
Rust wire helper now packs generation and retry epoch into disjoint 32-bit
fields, accepts only strictly newer sessions, and fails closed if either field
is exhausted.

## Relationship to formal verification

The models specify ownership transitions at the abstract protocol level.
Selected operations are split into preparation, publication, waiting and
retirement phases; each individual action remains atomic in TLA+ and is
checked against bounded safety invariants.

They do not yet answer “does the kernel implement that transition atomically?”
[`CONFORMANCE.md`](CONFORMANCE.md) documents, for every modeled operation:

1. its Rust entry point and participating registries;
2. the linearization point at which the abstract transition takes effect;
3. errors possible before and after that point;
4. rollback or cleanup behavior;
5. a mapping from concrete Rust state to the TLA+ variables.

The map makes remaining abstraction gaps explicit. Implementation-level traces
and property tests can subsequently be checked against it. TLAPS proofs and
temporal liveness properties are later steps; neither is claimed here.

## Thread metadata admission

`CharlotteThreadMetadata` follows two unequal prepaid context/node bundles
through allocation, publication, confirmed context disposal, node deallocation
and refund. Two roots share a principal; one slot can retire and restart under
another principal while its old charge remains. Byte/count ceilings and original
sponsorship remain invariant. Abandonment retains admission. The safe graph has
4,132 distinct / 12,559 generated states, depth 15, with every required action
exercised. Four unsafe configurations admit by generation, refund before node
free, refund abandonment or bill a replacement principal.

This is a finite ordering projection. Stack-release proof is an input; node
reserve, allocator overhead, observers/table capacity, IRQ contexts and progress
are outside it. See [policy](../reference/thread-metadata-admission.md) and
[execution evidence](../reports/audits/2026-10-23-security-thread-metadata.md).

## Principal-history storage

`CharlottePrincipalHistory` follows three nodes through preparation, attachment,
new-record/reuse decisions, logical exit, confirmed root disposal and abandonment.
Two candidates can resolve to one principal with different captured classes.
Original classification and charges survive reuse; published records remain for
the boot lifetime. The safe graph has 2,539 distinct / 5,410 generated states,
depth 19, with all required actions exercised. Six controls ignore ordinary
admission, refund logical exit or Drop, reclassify an allocation, evict history
or free an attached node before confirmation.

Allocator/root-invalidation proofs are inputs. The projection omits exact bytes,
lookup/IRQ hold times, general metadata and pressure fairness. The seven focused
cases pass. The [24 October follow-up](../reports/audits/2026-10-24-security-root-metadata-admission.md)
also completes the expanded 190-case suite: 47 safe configurations and 143
expected counterexamples across 44 modules. See
[policy](../reference/principal-history-admission.md) and
[audit evidence](../reports/audits/2026-10-23-security-history-admission.md).


The [IPC namespace publication follow-up](../reports/audits/2026-10-24-security-ipc-namespace-publication.md)
rechecks `CharlotteRootRetirement` and `CharlotteNamespaceDrain`: two safe cases
and seven expected counterexamples pass. The existing operation lease projects
the fresh-root publication hold. Namespace allocation and IPC relinking remain
native qualification; `CharlotteIpcPublication` assumes already-live namespaces.
No model/configuration changed, and the preceding 190-case full-suite checkpoint
is preserved. See the [conformance boundary](CONFORMANCE.md#ipc-namespace-construction-and-root-handoff).


The [pending-call storage follow-up](../reports/audits/2026-10-24-security-ipc-pending-storage.md)
rechecks IPC publication and root retirement. Shared authority/delivery and
exact-lease gates are existing model projections; pending-family node allocation,
post-guard disposal and count refund are native/host qualification. See the
[conformance boundary](CONFORMANCE.md#pending-call-node-lifetime). No model or
configuration changed, and the preceding full-suite checkpoint is preserved.


The [reply-token storage follow-up](../reports/audits/2026-10-24-security-ipc-reply-storage.md)
checks IPC publication/transaction and root retirement: three safe cases and
17 expected counterexamples pass. Native controls qualify the two-node prefix,
post-guard token retirement, captured class and complete fallback retention;
shared collection tests check node destruction before refund. No model or
configuration changed. See the [conformance boundary](CONFORMANCE.md#reply-token-node-lifetime).

The [endpoint storage follow-up](../reports/audits/2026-10-24-security-ipc-endpoint-storage.md)
rechecks the same 20 IPC publication/transaction and root-retirement cases:
three safe configurations and 17 expected counterexamples pass. Unlocked
node/list/backing preparation, FIFO replacement, original sponsorship and
post-guard disposal remain native/host qualification. No model/configuration
changed; see the [conformance boundary](CONFORMANCE.md#endpoint-storage-and-queue-growth).
