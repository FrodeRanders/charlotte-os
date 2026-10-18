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
| Reads | The current query gate requires leadership, a nonzero current-term commit, application through that commit and recent receive-time quorum contact. Exact-term response fencing rejects stale-term credit; lease duration, delayed same-term responses and election/clock assumptions still need qualification before claiming linearizability. |
| Snapshots | Snapshots carry current and next membership, peer roles/addresses, and application state in the shared JSON envelope. Application restore must succeed before startup exposes a node or network installation publishes snapshot progress/membership. |
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

## Read prerequisites after election

The [18 October read review](../reports/audits/2026-10-18-raft-read-prerequisites.md)
reproduces a leader accepting the query gate after quorum contact below its
uncommitted election no-op. The gate now also requires a current-term commit
and application through that commit. Snapshot anchors use the ordinary term
lookup, so compaction preserves admission. Single-voter elections commit and
apply their no-op before reads become eligible. Leader-side bounded reads use
the same guard; follower bounded-staleness policy is unchanged.

Java and C++ already check application progress in the inspected core gates.
The inspected Java, C++ and Rust core gates do not explicitly require a
current-term commit. Their runtime refresh and query paths differ; this static
comparison does not establish a reproduced upstream failure. The new Charlotte
checks are necessary prerequisites, with two executable negative controls.
Receive-time contact can still be refreshed by a delayed same-term response;
complete read-lease correctness and request correlation remain open.
