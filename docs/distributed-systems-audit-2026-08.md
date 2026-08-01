# Distributed Systems and Remote IPC Audit — 2026-08

## Scope

This audit covers the 34 commits made from 2026-07-29 through 2026-07-31,
starting with the executable TLA+ and Raft-conformance work and continuing
through raw inter-QEMU Ethernet, `relmsg`, discovery, the Raft-backed
distributed name catalog, remote invocation, smoltcp/TCP, HTTP observability,
fragmentation, and timing hardening. The range changed 168 files with roughly
14,386 insertions and 2,040 deletions.

The review compared the Rust implementation with:

- `docs/related-research.md`;
- `docs/tla/README.md` and `docs/tla/CONFORMANCE.md`;
- `docs/charlotte-networking-architecture.md`;
- the local IPC, capability, scheduler, DMA, and Graft implementations; and
- the two-guest boot tests.

## Overall assessment

The local kernel mechanisms are substantially stronger than the distributed
prototype. Reply authority is installed at delivery, scheduler wakeups carry
thread generations, the tagged capability namespace detects cross-registry
mistakes, DMA teardown retains pins until invalidation is acknowledged, and
the Graft core has meaningful focused tests and bounded formal models.

The cross-machine path proves useful end-to-end functionality: two QEMU guests
exchange Ethernet frames, elect a Raft leader, replicate a small catalog, and
perform a scalar remote echo invocation. It must nevertheless still be treated
as a prototype. Its present durability, membership-bootstrap, restart, retry,
deduplication, and stale-read behavior is not yet a safe distributed
capability/name-service contract.

## Findings

### F1 — DNS uses volatile Raft state (critical)

`catten-services/src/bin/dns.rs` constructs its `RaftNode` with
`InMemoryLogStore` and `InMemoryPersistentStateStore`. A restarted DNS replica
forgets its term and vote and can vote twice in one term. This invalidates the
durable-vote assumption used by Raft election safety and by
`CharlotteRaft.tla`.

Follow-up:

- give each DNS replica a stable object-store namespace;
- use the existing disk-backed log and persistent-state implementations;
- refuse clustered operation when required durable state cannot be opened;
- test replica restart while the other members remain live.

### F2 — Membership is independently inferred from transient discovery (critical)

DNS waits for an expected peer count, but after 2,400 50-ms rounds it proceeds
with whichever peers it happened to discover. Nodes can therefore construct
different stable voter sets after asymmetric discovery or partitioned boot,
potentially forming independent clusters.

Discovery is suitable for finding transport routes, not for independently
deciding consensus authority.

Follow-up:

- persist an authoritative initial cluster configuration, or join through an
  already-authoritative member;
- require an exact expected identity set rather than only a count;
- use Graft joint consensus for subsequent changes;
- fail closed instead of silently shrinking the voter set;
- test asymmetric discovery and delayed-node startup.

### F3 — Relmsg Raft snapshots cannot advance (critical)

`RelmsgRaftTransport` converts every snapshot response into a completion with
`sent_next_offset = 0` and `sent_done = false`. The Raft core uses these fields
to advance chunk transmission and to finish installation, so network snapshot
catch-up remains at offset zero.

Follow-up:

- retain the request's next offset and `done` flag until its response arrives;
- associate responses with the correct outstanding request and peer;
- test multi-chunk transfer, final-chunk completion, loss, and retry.

### F4 — Large transport payloads overflow a one-page mapping (critical)

`relmsg_transport::send_payload` accepts up to 65,535 bytes but allocates only
one 4-KiB memory object before copying the complete payload. Payloads above one
page can write beyond the mapped object.

Follow-up:

- allocate `ceil(length / PAGE_SIZE)` pages;
- reject lengths that cannot be represented by the protocol;
- validate declared lengths against memory-object capacity at trust
  boundaries;
- test payload sizes around 4 KiB and the 65,535-byte ceiling.

### F5 — Relmsg has no restart/session epoch (high)

Per-MAC transmit and receive sequences start at one. If one relmsg service
restarts while its peer survives, the survivor treats the restarted sender's
sequence-one traffic as an old duplicate. The ACK also does not exactly match
the restarted sender's outstanding sequence, so communication remains broken
until both sides reset.

The wire format reserves `FLAG_SYN`, but no handshake or epoch currently uses
it.

Follow-up:

- introduce a boot/session identifier and SYN/reset handshake;
- bind sequence and acknowledgement state to the session;
- define behavior for delayed frames from an old session;
- test unilateral restart after traffic has advanced the sequence.

Repair status: wire protocol v2 now binds frames to a 64-bit service-instance
session, asserts it with `FLAG_SYN`, resets both sequence directions when a
new non-retired session appears, and rejects a bounded window of retired
sessions. Receive delivery and fragment accumulation are bounded. The normal
two-guest DNS/Raft path passes after this change; an automated unilateral
restart fault test remains to be added.

### F6 — Remote calls lack a complete identity and failure contract (high)

DNS records an in-flight call as only `(call_id, local_reply_token)`. Replies
are matched solely on `call_id`, not source peer, destination service,
generation, or session. IDs restart at one. There are no deadlines or
uncertain-outcome results, and relmsg send failure is discarded by the Raft
transport rather than propagated to the DNS caller.

Consequences include permanently blocked calls, unbounded in-flight state,
completion by a reply from the wrong peer, and collision with a delayed reply
after restart.

Follow-up:

- define a call identity containing caller node/session and monotonic call ID;
- bind replies to the expected source and target generation;
- add bounded deadlines and explicit transport/uncertain-outcome errors;
- define at-most-once or at-least-once semantics and a deduplication window;
- retain enough completed-call state to answer safe retransmissions;
- model this state machine before claiming distributed capability invocation.

### F7 — Frame routes retain stale capabilities after service restart (high)

`frouter` installs one route per EtherType and never replaces it. If relmsg,
discovery, or TCP/IP restarts, forwarding continues through the stale
connection. Failure only increments the dropped counter; it does not invalidate
and re-resolve the route.

Follow-up:

- record the name-service generation with each route;
- remove and re-resolve a route when submission or completion reports a stale
  or closed endpoint;
- periodically check for a newer generation;
- test consumer shutdown and replacement without restarting frouter or net.

### F8 — DNS reads are not linearizable (high)

`OP_LOOKUP` and `OP_CALL` read the local catalog on followers without a Graft
read barrier, leader forwarding, or proof that the replica has current quorum
contact. A stale or partitioned replica can therefore return obsolete routing
information and authorize an invocation based on it.

Follow-up:

- document which operations require linearizability;
- use the existing Graft read barrier for authoritative lookup/call routing;
- optionally expose a separately named stale/local-cache read;
- return a redirect or unavailable result when linearizability cannot be
  established.

### F9 — Committed catalog entries can lack a local service (high)

DNS commits `name -> node` before registering the supplied connection with the
node-local name service. If local registration fails, the replicated catalog
entry remains even though the owner cannot resolve or invoke it. Service exit
also has no replicated unregister/generation transition.

Follow-up:

- stage and validate local publication before committing ownership, or append
  a compensating removal on failure;
- replicate a service generation and lifecycle state;
- connect supervisor teardown/restart to unregister/replace commands;
- prevent a catalog-only local result from masquerading as a usable service.

### F10 — Relmsg buffering is insufficiently bounded (medium)

The completed receive queue is unbounded. Fragment maps can retain many small
or incomplete fragments, and incomplete reassembly has no deadline. Outbound
Raft queues are also unbounded and remove from the front of a `Vec`, producing
quadratic copying under backlog.

Follow-up:

- bound queued messages, bytes, fragments, and per-peer outbound RPCs;
- expire incomplete reassembly and old sessions;
- validate non-overlapping canonical fragments;
- use `VecDeque` for FIFO transport queues;
- expose drop/expiry/backpressure counters through observability.

### F11 — Documentation overstates distributed-name-service status (medium)

`docs/related-research.md` accurately describes remote authority, replay,
deduplication, service generations, stale-replica fencing, and invocation
guarantees as open. The networking architecture currently says that clustered
generations, access keys, revocation, proxy capabilities, and policy metadata
are realized. The implemented catalog stores only `name -> node`.

Follow-up:

- distinguish proven smoke-test behavior from target design;
- mark clustered generations, keyed policy, proxy capabilities, revocation,
  and linearizable reads as planned;
- retain the related-research terminology of implemented, model-checked,
  partial, and open.

### F12 — Service linting and TLA+ are outside CI (medium)

`catten-services` is excluded from the root workspace, while CI runs Clippy
only with `--workspace`. Direct AArch64 service Clippy currently reports
warnings in `relmsg`, `tcpip`, and `raft`. The executable TLA+ suite also
requires a manually supplied TLC jar and has no CI job.

Follow-up:

- add a standalone `catten-services` Clippy command to CI;
- pay the current warning debt;
- add a pinned/checksummed TLC dependency and model-check job;
- retain required-action coverage checks so accidentally dead model actions
  fail CI.

### F13 — Test coverage proves the happy path, not the failure contract (medium)

The two-node test proves discovery, election, one replicated registration, and
one scalar remote echo call. It does not exercise unilateral relmsg/DNS
restart, message duplication, snapshot transfer, leader replacement,
asymmetric membership, stale reads, remote-call timeouts, or service
generation changes.

The audit run also found that the leader could publish local test completion
before the follower's remote invocation completed, causing the runner to stop
the leader VM prematurely. The test now holds the leader's success result
until its DNS replica has served the follower's remote call.

Follow-up tests should cover all of those cases and add a three-voter scenario;
two voters are useful for transport testing but cannot remain available after
one failure.

## Validation performed during the audit

- AArch64 service bundle built successfully.
- `charlotte-protocol-msg`: 4 tests passed.
- `catten-graft`: 16 tests passed.
- `charlotte-smoltcp`: 2 tests passed.
- Workspace and standalone-service formatting checks passed.
- Direct service Clippy found four warning groups, confirming the CI gap.
- All ten TLC configurations completed without invariant violations and with
  the required action coverage: IPC, completion queue, scheduler, service
  lifecycle, capability namespace, DMA, Raft election, Raft log, Raft
  membership, and Raft snapshot.
- After the first repair slice, a fresh two-guest `--dns-test` run completed
  all 21 registered tests on both guests. The leader remained alive until it
  had served the follower's remote call, and both DNS replicas used durable
  object-store-backed Raft state.
- After authoritative voter bootstrap was added, a second fresh two-guest
  `--dns-test` run again completed all 21 registered tests on both guests.
  Each replica used the same exact two-node voter manifest; discovery supplied
  routes for those identities but did not determine Raft membership.
- After relmsg wire protocol v2 and bounded receive state were added, a third
  fresh two-guest run completed all 21 tests on both guests. The leader now
  waits for transport acknowledgement of its remote-call reply, avoiding the
  runner race in which QEMU stopped before the follower received that reply.

These TLC results establish safety only for the modeled projections. The
models deliberately omit the relmsg session, remote-call, discovery bootstrap,
transport queue, and network snapshot-correlation state machines in which the
principal distributed findings occur.

## Recommended implementation order

1. Repair memory safety in large transport payload allocation.
2. Preserve snapshot request progress across network responses.
3. Make frouter routes generation/restart aware.
4. Add relmsg sessions and bounded buffering.
5. Define and implement the remote invocation contract.
6. Move DNS to durable Raft state and authoritative membership bootstrap.
7. Add linearizable reads and replicated service lifecycle/generation policy.
8. Correct status documentation and expand CI/fault testing.

## Repair progress

- [x] Repair memory safety in large transport payload allocation.
- [x] Preserve snapshot request progress across network responses.
- [x] Make frouter routes generation/restart aware (invalidate and re-resolve
  stale connections; proactive generation refresh remains follow-up work).
- [x] Add relmsg sessions and bounded buffering (the automated unilateral
  restart fault test remains under the CI/fault-testing item).
- [ ] Define and implement the remote invocation contract.
- [x] Move DNS to durable Raft state and authoritative membership bootstrap.
  Clustered DNS now requires durable term/vote/log/snapshot stores and an
  exact launch-manifest voter identity set. Discovery resolves those voters to
  transport routes but cannot add, omit, or replace voting authority; missing
  configured voters fail closed.
- [ ] Add linearizable reads and replicated service lifecycle/generation policy.
- [ ] Correct status documentation and expand CI/fault testing.

Direct AArch64 service Clippy warnings identified by the audit have been
repaired, and the standalone service Clippy invocation has been added to CI.
Adding TLC to CI remains part of the final checklist item.
