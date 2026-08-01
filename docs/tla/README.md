# Executable TLA+ Models of CharlotteOS

This directory contains finite, executable specifications for eleven
CharlotteOS subsystems:

| Subsystem | Module | Fast configuration |
|---|---|---|
| Endpoint IPC and memory transfer | `CharlotteIPC.tla` | `CharlotteIPC_small.cfg` |
| Completion queues and waits | `CharlotteCQ.tla` | `CharlotteCQ_mini.cfg` |
| Scheduler thread lifecycle | `CharlotteScheduler.tla` | `CharlotteScheduler_small.cfg` |
| Service publication and teardown | `CharlotteServiceLifecycle.tla` | `CharlotteServiceLifecycle_small.cfg` |
| Unified tagged capability namespace | `CharlotteCapability.tla` | `CharlotteCapability_small.cfg` |
| DMA pinning and SMMUv3 teardown | `CharlotteDMA.tla` | `CharlotteDMA_small.cfg` |
| Raft election and durable voting | `CharlotteRaft.tla` | `CharlotteRaft_small.cfg` |
| Raft log replication and commit safety | `CharlotteRaftLog.tla` | `CharlotteRaftLog_small.cfg` |
| Raft joint membership and decommissioning | `CharlotteRaftMembership.tla` | `CharlotteRaftMembership_small.cfg` |
| Raft snapshot installation and recovery | `CharlotteRaftSnapshot.tla` | `CharlotteRaftSnapshot_small.cfg` |
| Remote-call identity, uncertainty, and bounded deduplication | `CharlotteRemoteCall.tla` | `CharlotteRemoteCall_small.cfg` |

These are abstract safety models checked with TLC. They are useful for finding
protocol and state-machine errors, but they are not a proof of the Rust
implementation. In particular, an action is atomic in TLA+ by definition.
Establishing that a Rust operation implements the same atomic transition will
require identified linearization points and a refinement argument.

## Running the models

Download a TLA+ tools release and run:

```sh
docs/tla/check.sh /path/to/tla2tools.jar
```

Alternatively, set `TLA2TOOLS_JAR`. The script:

- runs all ten complete fast configurations;
- enables TLC action coverage;
- places checkpoints and traces in a temporary directory;
- rejects structural TLC warnings in addition to invariant failures.

The individual commands are:

```sh
java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteIPC -config CharlotteIPC_small.cfg -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteCQ -config CharlotteCQ_mini.cfg -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteScheduler -config CharlotteScheduler_small.cfg -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteServiceLifecycle -config CharlotteServiceLifecycle_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteCapability -config CharlotteCapability_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteDMA -config CharlotteDMA_small.cfg \
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
  CharlotteRaftSnapshot -config CharlotteRaftSnapshot_small.cfg \
  -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteRemoteCall -config CharlotteRemoteCall_small.cfg \
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

`StateConstraint` bounds the otherwise unbounded generation counter for TLC.
This is a model-checking bound, not a statement that the implementation's
counter cannot wrap.

## Scheduler lifecycle model

`CharlotteScheduler.tla` separates thread admission, dispatch, blocking,
wakeup, migration, abort, and reaping. In particular, `Dead` means that
`abort_thread` has removed the thread from `MASTER_THREAD_TABLE` but its owned
context is still staged in the dying LP's `DEAD_THREADS` list. Only `Reap`
makes the reusable slot absent. Wake actions carry the generation captured by
the waker and cannot affect a later occupant of the same thread ID.

The checked invariants require one running thread per LP, valid placement and
pinning, a valid owner for every non-absent thread, matching blocked-waker
generations, and migration authority for every movable thread.

## Service lifecycle model

`CharlotteServiceLifecycle.tla` models loading, starting, name-service
publication, replacement publication, shutdown, exit, scheduler reaping, and
address-space teardown. Publication is a single linearization point: the new
generation becomes visible while the old generation ceases to be published.
Teardown requires explicit evidence that the scheduler reaped the domain's
initial thread, matching `wait_domain_exit` followed by `teardown_domain`.

The model checks unique and internally consistent publication, monotonic
bounded generations, and the prohibition against teardown before reaping.

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

## DMA and SMMUv3 model

`CharlotteDMA.tla` separates memory pinning, page-table installation,
translation revocation, pin release, driver exit, and memory reclamation. A
domain-destruction timeout enters a quarantined state: mappings and pins remain
live because hardware may still hold a usable translation.

The invariants require every hardware-visible mapping to belong to the same
driver as its memory object, every mapped or transitional object to remain
pinned and unfreed, one domain per requester stream, and acknowledged stream
revocation before domain resources can be finalized.

Developing the model found two concrete error-path defects. A failed map could
drop its `DmaPin` token without decrementing the memory object's pin count, and
domain destruction ignored failure to install and acknowledge the aborting
stream-table entry before releasing pins. The implementation now rolls
pre-installation failures back, retains an unpublished pinned mapping after an
uncertain invalidation, and quarantines a domain on teardown failure.

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
voter and learner roles, old-configuration commitment of the `JOINT` entry,
joint-majority commitment of `FINALIZE`, the implementation's all-proposed-peer
catch-up fence before automatic finalization, leader eligibility, crashes,
restarts, and decommissioning. Its two-entry, three-node configuration explores
5,656 distinct states.

The invariants require voters and learners to remain disjoint, both voter
majorities during joint consensus, finalization only after every proposed
member reaches the joint-entry fence, and decommissioning to follow the active
member union rather than voter status. Durable vote mechanics remain in the
election layer and entry conflict repair remains in the log layer.

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
The model corresponds to the scalar DNS v3 prototype; transactional effects,
durable deduplication across DNS reboot, and general remote capability transfer
remain outside it.

## Relationship to formal verification

The models answer “are ownership transitions formally specified?” only at the
abstract protocol level: the desired transition is written as one atomic TLA+
action and checked against bounded safety invariants.

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
