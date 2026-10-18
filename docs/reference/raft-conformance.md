# Graft Conformance in CharlotteOS

`catten-graft` is a `no_std` adaptation of the general Graft implementation,
not an independent Raft design. Consensus behavior should remain equivalent to
the Java, C++, and Rust implementations in the Graft repository. CharlotteOS
substitutes capability IPC, name-service discovery, completion queues, and the
NVMe object store at the platform boundary.

## Required behavioral parity

| Concern | CharlotteOS implementation |
|---|---|
| Membership identity | Peer IDs are normalized; duplicate IDs cannot manufacture a quorum. |
| Elections | Only voters campaign, only active voters receive votes, and joint configurations require a majority of each voter set. |
| Leadership | A newly elected leader appends an empty no-op entry; no-op entries are never delivered to the application state machine. |
| Replication and commit | Responses require a matching admitted source and exact current term; success cannot exceed the local tail or reduce confirmed progress, and backoff retains that floor. The leader counts itself once, counts configured voters, requires a current-term entry and uses the candidate index's configuration. |
| Membership changes | `JOIN`, `JOINT`, and `FINALIZE` are shared protobuf internal commands. Direct local membership mutation is rejected. Automatic FINALIZE waits for the committed JOINT fence and every proposed remote voter/learner to catch up, restoring Graft Java/Rust behavior. Its commit still requires both voter majorities. |
| Learners and removal | Learners replicate but neither campaign nor vote. Finalization makes a non-voter a follower immediately, preserving its existing-term vote. Removed nodes become decommissioned; retained learners remain members. |
| Reads | Catalog queries consume a fresh `PendingRead` fence after current-term quorum commit and application. Node/domain, term and membership epoch are rechecked. A prior delayed acknowledgement cannot complete a newly admitted read. Bounded status/ingress observations remain a separate receive-time freshness policy. |
| Snapshots | Snapshots carry current and next membership, peer roles/addresses, and application state in the shared JSON envelope. Partial buffers bind leader ID and RPC term as well as boundary/offset; a new election discards them. Application restore must succeed before startup exposes a node or network installation publishes snapshot progress/membership. |
| Wire contract | Vote, append, snapshot, peer, and internal-command messages use field-compatible `raft.proto` protobuf payloads. |
| Liveness | Capability RPCs have bounded deadlines and at most one outstanding call per peer/RPC type/term. Append batches are reduced to the largest payload fitting the current one-page IPC attachment. |

The executable TLA+ suite checks this behavior in complementary bounded
layers: durable election safety, log replication, joint voter/learner
membership and automatic finalization, and atomic snapshot recovery of both
application state and membership, plus application rejection and asynchronous
response fencing. See
[`../tla/README.md`](../tla/README.md#raft-election-model) and
[`../tla/CONFORMANCE.md`](../tla/CONFORMANCE.md#raft-membership-and-joint-consensus).

## Intentional platform differences

- The general runtimes frame protobuf envelopes over TCP. CharlotteOS passes
  the protobuf payload in a moved memory capability and carries its exact byte
  length in the scalar IPC argument/result.
- A peer address is encoded as `charlotte:<service-name>` in `PeerSpec.host`.
  The local transport obtains the corresponding connection capability through
  a waitable name-service lookup. Discovery follows committed membership, not
  only the boot manifest.
- CharlotteOS stores term/vote/log/snapshot records in object-store namespaces.
  Snapshot replacement and retained-suffix persistence use one atomic disk
  record update; this is deliberately stronger than a two-write
  compact-then-install sequence. The current [persistence contract](raft-persistence.md)
  opens two tagged records jointly and rejects incomplete or unsupported
  stores; it never migrates an older layout or falls back to memory on error.
- Time is supplied by the EL0 reactor's bounded completion-queue wait rather
  than `std::time`.
- The current memory-capability transport limits one RPC attachment to 4 KiB.
  This is a transport batching limit, not a Raft log or persistent-object size
  limit.
- Cross-machine Raft RPCs (the distributed name service) travel over relmsg v3,
  whose 32-bit lengths support an initial 1 MiB message policy ceiling; the
  local direct capability IPC attachment remains one page.

## Validation and drift control

The authoritative AArch64 boot suite starts two Raft EL0 domains in
registration-order-independent fashion and requires one leader plus completed
capability RPC traffic. A separate NVMe-backed single-voter test tears down and
restarts a Raft process and verifies recovered term/vote state.

`scripts/run-host-tests.sh` runs the portable consensus and service tests from
a temporary invocation directory, avoiding the root's freestanding
`build-std` configuration. Persistence probes cover tagged records, opening
and failure admission, snapshot/suffix recovery and explicit reset. These
tests complement the native storage fixture; they do not replace device flush,
power-loss or multi-node restart qualification. Consensus changes must still
be reviewed against this matrix and exercised in the relevant boot fixtures.

## Response-ordering follow-up

The [18 October response review](../reports/audits/2026-10-18-raft-response-fencing.md)
reproduces an old-term acknowledgement committing a replaced prefix, then checks
the corrected core for both append and snapshot responses. Committed joiners
remain eligible for replication before membership promotion. Transport source
identity is an input: these checks do not implement authenticated networking or
protect against a peer lying about its own durable prefix. The [membership follow-up](../reports/audits/2026-10-18-raft-membership-parity.md)
subsequently corrects demoted leadership and restores the automatic catch-up gate.

## Upstream synchronization baseline

The [18 October membership/parity review](../reports/audits/2026-10-18-raft-membership-parity.md)
compares the clean Graft checkout at `1a818d416a40330bea2055e03f61580679baa9f6`.
Its version-controlled evidence retains the relevant Java, C++ and Rust source
extracts and file hashes, so the comparison does not depend on that local checkout.

| Boundary | Observed upstream behavior | Charlotte qualification |
|---|---|---|
| Delayed responses | C++ checks exact response term and keeps monotonic successful progress. The inspected Java/Rust handlers differ; caller scheduling also matters. | The preceding Charlotte unsafe-commit regression is reproduced and corrected. No equivalent upstream failure is claimed from static inspection alone. |
| Automatic finalization | Java/Rust require the committed JOINT fence and proposed-member catch-up. | Restored after a Charlotte-specific early-finalization change introduced in `4d2856df`. Explicit/manual FINALIZE remains separate. A lagging proposed learner can intentionally delay automatic completion. |
| Retained learner leadership | Java has a periodic non-voter role check. The Rust core has one too, with different runtime scheduling. | Charlotte's former timeout check merely returned false; finalization now relinquishes authority immediately without erasing the term's vote. |
| Hostile membership scenarios | The Jepsen harness covers non-leader demotion and separate leader removal, along with partition, pause/restart and mixed-implementation scenarios. | The new retained-leader-as-learner regression complements those definitions. Jepsen was not rerun here. |
| Mixed clusters | Graft's suites include Java/C++, Java/Rust and three-language cases. | Shared protobuf message shapes do not establish native interoperation. Charlotte uses capability IPC/relmsg and a different application/environment; no Graft TCP gateway or cross-runtime cluster is qualified by these ARM checks. |

Consensus reviews must compare core **and runtime** behavior across the reference
implementations. Capture each deliberate difference and its regression before
claiming parity. A matching single handler or passing local model is not a mixed
cluster test. Read-lease timing and request correlation remain separate work.

## Per-read quorum fences

The [19 October read-fence correction](../reports/audits/2026-10-19-raft-read-fences.md)
replaces the receive-time query lease. `begin_read(query, now)` appends a fresh
no-op and returns `PendingRead`. Retain it while driving Raft; `read_ready`
observes progress without querying. `handle_client_query(read)` consumes it and
rechecks exact node/domain, term, membership epoch, commitment and application.
Dropping a read abandons its query; the no-op remains ordinary replicated state.
A compacted fence remains valid through a newer committed current-term anchor.

DNS holds local lookup/invocation routing and remote queries behind these
owners. Queue storage is admitted before log publication. Each operation carries
one kernel-monotonic deadline through the existing owned NS lookup/invocation
phases on the shared CQ. DNS placement lookup returns metadata only: a local
result grants no capability, and a remote result carries its node identity in
owned moved memory. It cannot re-delegate ordinary NS-returned SEND/CALL
authority. Read preparation, timeout or leadership rejection cannot execute
the business call.

The former stale-read witness now rejects: after n2 commits value 2, its delayed
term-one reply acknowledges only the old prefix on n1, below n1's newly admitted
read fence. Four model controls retain the omitted prerequisite/fence/binding
failures. Historical reports retain the original unsafe execution.

This uses the shared empty no-op log grammar. It adds one log entry and quorum
commit per read, with durable write and snapshot costs; no throughput or latency
qualification is claimed. Graft core/runtime read paths differ, and no new
upstream execution, Jepsen result or mixed Charlotte/Graft cluster is claimed.

## Bounded observations and service time

Both reactors sample checked kernel-monotonic elapsed time after waits and
before inbound/scheduled work. Invalid clocks abort with pending work live;
timers and CQ watchdogs remain wake mechanisms. The
[clock review](../reports/audits/2026-10-18-raft-clock-lease-gap.md) preserves the
old wake-count failure and the original stale-read witness.

`can_serve_bounded_read` remains a receive-time freshness policy for status and
ingress projections, including its leader branch. It grants no catalog query
authority. Follower and control-plane freshness, clock/election timing,
authenticated network identity and raw transport readiness remain separate
qualification boundaries. This correction does not establish general lease
safety or close a numbered security finding.

## Snapshot continuation identity

The [19 October stream review](../reports/audits/2026-10-19-raft-snapshot-streams.md)
reproduces installation of a former sender's prefix followed by a new term's
suffix at the same snapshot boundary. The private pending buffer now captures
leader ID and RPC term. Local elections and higher-term observations discard it;
a replacement starts at offset zero. Invalid stale/source/boundary/offset
requests leave an accepted current stream intact.

Java's inspected receive owner retains leader ID. The inspected Rust core and
C++ handler have different checks and no equivalent sender/term binding;
this is a static comparison, with no upstream failure reproduction or Jepsen
rerun. Shared wire/storage formats are unchanged. An immutable byte image per
sender/term/boundary remains an assumption; same-episode image replacement and
transport authentication need separate qualification.
