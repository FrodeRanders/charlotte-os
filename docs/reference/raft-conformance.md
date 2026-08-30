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
| Replication and commit | The leader counts itself once, counts distinct configured voters, requires an entry from its current term, and evaluates the configuration in force at the candidate index. |
| Membership changes | `JOIN`, `JOINT`, and `FINALIZE` are shared protobuf internal commands. Direct local membership mutation is rejected. Joint consensus is automatically finalized after proposed members reach the joint-entry fence. |
| Learners and removal | Learners replicate but neither campaign nor vote. A removed node becomes decommissioned; a learner is not incorrectly treated as removed. |
| Reads | Linearizable queries require current leadership plus recent contact with a quorum. |
| Snapshots | Snapshots carry current and next membership, peer roles/addresses, and application state in the shared JSON envelope. Recovery restores membership before accepting elections or leader RPCs. |
| Wire contract | Vote, append, snapshot, peer, and internal-command messages use field-compatible `raft.proto` protobuf payloads. |
| Liveness | Capability RPCs have bounded deadlines and at most one outstanding call per peer/RPC type/term. Append batches are reduced to the largest payload fitting the current one-page IPC attachment. |

The executable TLA+ suite checks this behavior in complementary bounded
layers: durable election safety, log replication, joint voter/learner
membership and automatic finalization, and atomic snapshot recovery of both
application state and membership. See
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
  compact-then-install sequence.
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

Host unit-test execution is currently obstructed by the workspace-wide custom
`build-std` configuration producing duplicate `core` lang items. The
`catten-graft` test modules remain useful where a harness is available, but
they are not a substitute for a dedicated portable consensus test crate. Until
that exists, changes to `catten-graft` should be reviewed against this matrix,
checked with strict Clippy for the AArch64 target, and exercised by the full
boot suite.
