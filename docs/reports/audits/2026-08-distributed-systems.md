# Distributed Systems and Remote IPC Audit — 2026-08

> **Historical report:** this is point-in-time evidence and may describe
> defects or document paths that were later corrected. See the
> [documentation index](../../README.md) for current sources of truth.

## Scope

This audit covers the 34 commits made from 2026-07-29 through 2026-07-31,
starting with the executable TLA+ and Raft-conformance work and continuing
through raw inter-QEMU Ethernet, `relmsg`, discovery, the Raft-backed
distributed name catalog, remote invocation, smoltcp/TCP, HTTP observability,
fragmentation, and timing hardening. The range changed 168 files with roughly
14,386 insertions and 2,040 deletions.

The review compared the Rust implementation with:

- `docs/research/related-systems.md`;
- `docs/tla/README.md` and `docs/tla/CONFORMANCE.md`;
- `docs/architecture/networking.md`;
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

Repair status: wire protocol v2 binds frames to a 64-bit session and asserts
it with `FLAG_SYN`. The 2026-08-11 TLA+ sync found that incrementing a raw
service generation for retry epochs collided with the next service
generation, and that a bounded retired-session window could eventually accept
an older delayed SYN. Sessions now pack the service generation and retry epoch
into disjoint fields; receivers reset ordering only for a strictly newer
well-formed identity. Receive delivery and fragment accumulation remain
bounded. The normal two-guest DNS/Raft path passes; an automated unilateral
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

Repair status: DNS v2 now identifies calls by caller node/session/call ID,
accepts replies only from the expected peer, bounds in-flight state, returns an
explicit `ERR_UNCERTAIN` on deadline, and keeps a bounded completed-result
deduplication window. The replicated catalog now retains generation tombstones,
increments generations on replacement, includes them in snapshots and
observability, and rejects a remote call whose target generation is stale. A
fresh two-guest run passed 21/21 on both nodes. Model-checking the retry/cache
state machine remains open, so this finding is only partially repaired.

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

Repair status: `OP_LOOKUP` and the routing decision for `OP_CALL` no longer
read follower-local catalog state. A follower sends a source-bound, correlated
query to its known leader; the leader answers only through Graft's
quorum-contact read barrier. Query failure is safely retryable as
`ERR_NOT_LEADER`, while a timeout after call dispatch remains `ERR_UNCERTAIN`.
Raft time now advances from a persistent detached-timer completion, so
sustained endpoint traffic cannot keep the lease clock artificially frozen.
The follower path passed a fresh two-guest 21/21 test; partition fault
injection remains open.

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

Repair status: registration is now a replicated two-phase protocol. `prepare`
allocates a new monotonic generation but leaves it invisible; after node-local
connection publication succeeds, a generation-fenced `activate` command makes
the entry visible. Failed local publication leaves an inactive replicated
tombstone rather than a resolvable ghost. Catalog snapshot v3 preserves the
phase, while v1/v2 snapshots migrate existing entries as active. A fresh
two-guest run passed 21/21. The kernel now exposes a waitable connection-close
completion. DNS retains one for each local publication and proposes the same
owner-and-generation-fenced tombstone when its endpoint closes. A follower
sends an authenticated, idempotent request to its known leader and retries at
one-second intervals across leader changes. The request shares relmsg's
per-peer queue, where queued AppendEntries heartbeats are coalesced. A fresh
two-guest acceptance run was still required for this final automatic path.

Follow-up audit found and repaired four integration defects before that
acceptance run could be authoritative. Syscall 60 existed in the shared enum
and kernel dispatcher but was absent from the AArch64 `svc3` emitter, aborting
DNS when it installed the watch. Automatic proposals are now term-scoped and
identical queued relmsg frames are deduplicated. Scalar DNS registration adopts
an already-published local service connection (without granting callers
minting authority), which supplies the endpoint identity required by the
watch. The cross-node test now waits for the echo-mutating lifecycle/NVMe
suites and for the newly spawned echo's serving stage, avoiding adoption of a
stale generation. The final rerun was inconclusive because one TCG guest
stopped advancing at 0.53 seconds and its peer hit the discovery deadline;
automatic tombstone certification was therefore pending at that point. The
2026-08-09 acceptance run documented below subsequently exercised the path
successfully on both guests.

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

`docs/research/related-systems.md` accurately describes remote authority, replay,
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

## 2026-08-09 branch follow-up

A second review covered the `disco-raft-bridge` work from `49e5e01` through
`3eba685`, together with the uncommitted repair pass that followed it. This
slice combined changes to boot waiting, address-space lifetime fencing,
direct-Ethernet Raft admission, discovery, the frame router, and the
two-guest deployment acceptance test. The review found and repaired the
following integration defects:

- join requests and replies contained the message tag twice: once in the
  hand-written join body and once in the Raft transport envelope;
- direct-Ethernet Raft decoded the entire padded Ethernet payload as protobuf
  input. The direct transport now carries an explicit body length and rejects
  truncated or non-canonical join bodies;
- join admission required discovery's cached peer posture to identify a
  leader before sending the first request. Identity could already be stable
  while that advisory role remained stale or unknown, leaving both verifiers
  parked forever. The deterministic larger-id singleton now applies to the
  smaller-id anchor (or a known leader hint), while the receiving Raft node
  remains authoritative and rejects the request unless it is actually leader;
- the cross-QEMU join test reused the 150-ms election timeout from the
  in-process Raft test. After admission both voters could become candidates
  again before vote traffic cleared the boot-time queues, repeatedly splitting
  the two available votes. The networked join group now uses the same 2-second
  transport budget as the distributed DNS test;
- the standalone Raft service broadcast heartbeats on every reactor
  iteration. Queue coalescing could bound queued heartbeats, but could not
  limit a producer that filled each newly available stop-and-wait slot. Raft
  now broadcasts on an election-derived heartbeat cadence;
- DNS, Raft, and discovery treated successful detached-timer submission as
  proof that the CQ completion would arrive. Their bounded CQ wait is now an
  independent clock watchdog, and a new timer is submitted only after the old
  timer is observed or known not to be armed;
- the frame router synchronously waited for optional route consumers and for
  each forwarded frame. It now owns one deferred name-service lookup per
  absent route and a bounded set of asynchronous forwards per consumer, so a
  late or wedged protocol cannot block all EtherTypes;
- `clusterctl` read moved memory through an obsolete fixed virtual address
  instead of the address returned by `memory_map_any`, and several fallible
  capability paths leaked or double-closed handles;
- scratch-window allocation was keyed only by recyclable numeric ASID, while
  mapping and teardown could cross an ASID-reuse boundary. Scratch cursors now
  carry the address-space generation, and mapping/unmapping is serialized with
  address-space lifecycle teardown;
- an IPC waiter treated every observable notification as proof that its own
  call had completed. Waiters now re-check the call after every wake and park
  again on unrelated notifications;
- a verifier could run before its TID-to-test attribution was published. The
  thread is now inserted, attributed, and only then admitted to the scheduler;
- the serial administration console yielded while remaining Ready when no
  input existed, consuming a host CPU in steady state. Its no-input path now
  uses a short timer-backed sleep;
- a socket-linked test runner killed a guest immediately after its local
  authoritative result. In a two-node Raft group, the peer may already have
  replicated the final entry but still need the following heartbeat to learn
  the advanced commit index. Socket-linked runs now retain a successful guest
  for a 15-second drain window and fail if a kernel panic appears during that
  window. The larger bounded tail also covers a runnable verifier that has not
  yet consumed service state already published under slow TCG scheduling;
- relmsg retransmission used a relative CQ timeout that restarted whenever
  endpoint work arrived. Sustained inbound traffic could therefore prevent
  the timeout from ever winning while an outbound frame remained
  unacknowledged. Relmsg now drains a detached periodic-timer cookie on every
  reactor iteration, so retransmission cadence is independent of traffic;
- both AArch64 target specifications enabled NEON and the kernel enabled
  FP/SIMD execution, but exception entry saved no vector registers and thread
  switching omitted the ABI-preserved v8-v15 registers. An interrupt during
  Ed25519 verification produced an impossible table index after its live
  vector state was corrupted. Exception entry now preserves q0-q31 plus
  FPCR/FPSR in an out-of-line common handler, while context switches and
  initial frames preserve q8-q15 plus FPCR/FPSR;
- the deployment test treated a relmsg acknowledgement counter as proof that
  the follower had received its post-migration result. A bounded relmsg retry
  lease can end after the application result was delivered but its transport
  ACK was lost, leaving the leader verifier waiting for an event that can no
  longer occur. The follower now issues a second idempotent call only after
  receiving the first result; the leader observes that causally later request
  as the application-level barrier.

The GICv3 SGI encoding was also rechecked against `ICC_SGI1R_EL1`: the SGI
INTID belongs in bits `[27:24]`; bits `[55:48]` carry `Aff3`. The current code
uses the system-register layout and translates scheduler LP IDs through their
recorded MPIDRs. The two-LP mailbox, EL0 cross-LP completion, and ping-pong
tests passed in both guests. A dedicated regression that admits work onto a
fully idle remote LP remains worthwhile because the present bootstrap policy
admits service and verifier threads on the caller LP before normal scheduling
and migration take over.

The final authoritative acceptance rerun used two socket-linked QEMU guests with
two vCPUs each, listener first, fresh object-store images, and one identical
kernel hash. Both guests reported:

```
SELFTEST COMPLETE: passed=23 failed=0 pending=0 \
passed_bitmap=0x19fffff failed_bitmap=0x0 pending_bitmap=0x0
```

That run covered discovery, dynamic Raft admission, replicated DNS,
generation-fenced endpoint death, cluster-key ceremony, signed artifact
deployment, remote invocation, and migration. It does not replace the open
fault-injection work: unilateral service restart, loss/reordering around join,
partitioned linearizable reads, three-voter availability, and a fully idle
remote-LP wake regression remain appropriate follow-ups.

## Validation performed during the audit

- AArch64 service bundle built successfully.
- `charlotte-protocol-msg`: 4 tests passed.
- `catten-graft`: 16 tests passed.
- `charlotte-smoltcp`: 2 tests passed.
- Workspace and standalone-service formatting checks passed.
- Direct service Clippy found four warning groups, confirming the CI gap.
- All eleven TLC configurations completed without invariant violations and with
  the required action coverage: IPC, completion queue, scheduler, service
  lifecycle, capability namespace, DMA, Raft election, Raft log, Raft
  membership, Raft snapshot, and bounded remote invocation.
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
  waited for transport acknowledgement of its remote-call reply, avoiding the
  original runner race in which QEMU stopped before the follower received that
  reply. The 2026-08-09 follow-up subsequently replaced this transport-coupled
  test barrier with the causal application request described above.
- The first DNS v2 remote-call validation rerun exposed an unrelated boot
  liveness failure: one connector guest stopped advancing at 0.255 seconds,
  before networking or the changed call path ran. A fresh rerun completed all
  21 tests on both nodes. This reproducible class of scheduling-sensitive boot
  failure remains evidence for repair-order item 8.
- After the bounded deduplication cache was aligned with the remote-call model,
  both guests again replicated the catalog and completed a remote invocation
  with result 42. The listener produced the authoritative 21/21 result. The
  connector completed DNS but remained at 20/21 on the independently tracked
  `scheduler-lifecycle` test, reproducing the scheduling-sensitive validation
  issue rather than a distributed-call failure.
- The runner can now capture scheduler traces and timer/waker snapshots from
  both guests simultaneously using distinct `--gdb-port` values and
  instance-qualified output files. Two subsequent two-guest runs (one complete
  DNS run and one shorter NIC run) passed scheduler lifecycle on both guests;
  the intermittent LP0 wake loss has not yet recurred under instrumentation.
- A fresh two-guest DNS v3 run completed 21/21 on both guests after adding
  explicit service removal. Registration returned the committed generation;
  the leader committed an owner-and-generation-fenced tombstone, both replicas
  observed the catalog contraction, and a stale unregister replay was rejected.
  The run also exposed and removed a synchronous local-cleanup wait from the
  DNS Raft reactor.
- The 2026-08-09 follow-up completed the full deployment suite with 23/23 on
  both two-vCPU guests. `catten-graft` passed 23 host tests,
  `charlotte-protocol-disco` passed 2, `charlotte-protocol-msg` passed 4, and
  `charlotte-smoltcp` passed 2. The cluster-sign metadata self-test passed,
  and both the root AArch64 workspace and standalone AArch64 services were
  Clippy-clean with warnings denied. Root and standalone-service formatting
  checks also passed.
- The final clean two-guest rerun after the causal deployment barrier used
  two vCPUs per guest, fresh object-store images, listener-first socket setup,
  and the same kernel hash on both nodes. Both guests again produced the
  authoritative 23/23 result; neither serial log contained a kernel panic.

These TLC results establish safety only for the modeled projections. The
models deliberately omit the relmsg session, discovery bootstrap, transport
queue, and network snapshot-correlation state machines in which several of the
principal distributed findings occur. The remote-call model covers bounded
identity retention, generation fencing, uncertainty, and safe dedup eviction;
it does not claim transactional or globally exactly-once execution.

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
- [x] Complete the bounded remote invocation contract. Caller/session/call identity,
  source binding, bounded deadlines/in-flight state, explicit uncertainty, and
  bounded deduplication and target-generation binding are implemented. The
  state-machine model requires completed results to remain until transport
  settlement; the implementation now returns `ERR_BUSY` instead of evicting an
  unsettled result when that bounded cache is full.
- [x] Move DNS to durable Raft state and authoritative membership bootstrap.
  Clustered DNS now requires durable term/vote/log/snapshot stores and an
  exact launch-manifest voter identity set. Discovery resolves those voters to
  transport routes but cannot add, omit, or replace voting authority; missing
  configured voters fail closed.
- [x] Complete replicated service lifecycle policy. Monotonic generations,
  two-phase prepare/activate publication, tombstones, generation-fenced calls,
  and linearizable lookup/call routing through follower-to-leader forwarding
  are implemented. Explicit unregister is fenced by owning node plus
  distributed generation, while its asynchronous local cleanup is separately
  fenced by the observed local generation. Endpoint death produces a waitable
  kernel completion and an authenticated, retried owner-to-leader tombstone
  request. The fresh two-guest acceptance run for this automatic path now
  passes; partition fault injection remains validation work.
- [ ] Correct status documentation and expand CI/fault testing.

Direct AArch64 service Clippy warnings identified by the audit have been
repaired, and the standalone service Clippy invocation has been added to CI.
Adding TLC to CI remains part of the final checklist item.
