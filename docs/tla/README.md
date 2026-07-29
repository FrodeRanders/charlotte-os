# Executable TLA+ Models of CharlotteOS

This directory contains finite, executable specifications for two
CharlotteOS subsystems:

| Subsystem | Module | Fast configuration |
|---|---|---|
| Endpoint IPC and memory transfer | `CharlotteIPC.tla` | `CharlotteIPC_small.cfg` |
| Completion queues and waits | `CharlotteCQ.tla` | `CharlotteCQ_mini.cfg` |

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

- runs both complete fast configurations;
- enables TLC action coverage;
- places checkpoints and traces in a temporary directory;
- rejects structural TLC warnings in addition to invariant failures.

The individual commands are:

```sh
java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteIPC -config CharlotteIPC_small.cfg -workers auto -coverage 1

java -XX:+UseParallelGC -cp tla2tools.jar tlc2.TLC \
  CharlotteCQ -config CharlotteCQ_mini.cfg -workers auto -coverage 1
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
