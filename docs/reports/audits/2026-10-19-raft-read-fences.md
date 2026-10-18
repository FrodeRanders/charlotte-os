# Raft per-read quorum fences — 19 October 2026

## Result

Baseline `f489c8f` commits the elapsed-time correction and stale-read witness.
This batch replaces the catalog query lease with a fresh quorum-committed log
fence for each admitted read. The former three-node stale-read schedule now
rejects. DNS retains requests while servicing the traffic needed to commit their
fences, then rechecks the original deadline before resolving or invoking a
service.

The [evidence manifest](evidence/2026-10-19-raft-read-fences.json) retains source
and artifact hashes, host/native results, model action coverage and complete
counterexamples. Historical reports and their unsafe executions remain unchanged.

## Query owner and proof

`begin_read(query, now)` prepares a `PendingRead` and appends a fresh empty no-op
using the existing shared log grammar. The owner captures its exact node/domain,
term, index, membership epoch and query bytes. A lazy, fallibly allocated Arc
identity distinguishes nodes even when their scalar terms and indices match.
Pending owners retain that identity across node destruction; `begin_joining`
invalidates the old domain before replacing its history.

`read_ready(&read)` observes commitment without executing a query. It requires
the same domain, term and membership epoch, commitment through the new index,
and application through the current commit. `handle_client_query(read)` consumes
the owner, repeats those checks and executes once. A newer compacted current-term
anchor preserves commitment through an older fence. Leadership or membership
change rejects conservatively. The old synchronous query API is removed.

An acknowledgement created before read admission cannot confirm its new index.
A quorum may confirm that index after admission but reply after another leader
is elected; the read can still linearize within its interval. The required proof
is quorum confirmation after admission, rather than a receive-time promise that
no later election happened. This relies on the existing truthful-peer, durable
log, election and quorum contracts.

Dropping a read abandons only its query. Its no-op may commit later, without
executing that query or a business invocation. Preparing the domain can reject
before append. Existing log-store append/persistence uses its established
infallible-or-fail-stop boundary; this change does not qualify general heap or
storage-failure recovery.

Each read adds a log entry and quorum commit, including durable write and
snapshot costs. No throughput or latency qualification is claimed. A future
correlated-probe optimization needs a shared core/runtime contract and the same
request-relative proof.

## DNS transaction phases

The bounded read operation owns its fence, full request name, typed destination
and original kernel-monotonic deadline. It covers leader-local lookups, local
invocation routing and remote query requests. Follower routing retains typed
lookup reply authority in the existing correlated query operation.

Queue storage and query preparation precede no-op publication. The reactor
continues to process Raft traffic and advances reads after progress; no blocking
reply wait or extra per-read timer is added. Existing raw transport readiness
qualification remains separate. The earliest original read/operation deadline
participates in the existing shared CQ wait.

DNS placement lookup now returns metadata only: a local scalar result carries
no capability, and a remote result carries its node identity in owned moved
memory. The previous optional local-connection forwarding path is removed.
Invocation retains its own NS lookup/call owners on the shared CQ, through the
runtime's established result adoption and cleanup policy. Lookup/call receives
adopt their complete incoming transaction before scalar fallback guards.

A stronger native draft exposed the authority distinction. Its local lookup
attempted to re-mint an NS-returned SEND/CALL connection and produced IPC
cancellation (`-3`) instead of a local result (`0`). Source review shows that
public NS lookup strips MINT_CONNECTION while reply delegation requires it;
the consuming reply wrapper closes on mint rejection. The exact failed mint
status was not logged separately. One guest panicked at that assertion and the
other later timed out waiting for full-name echo convergence; both failed runs
are retained. The correction removes the attempted re-delegation, preserving
NS/grant policy instead of widening rights. The final fixture asserts the local
metadata result has neither a connection nor memory attachment. The manual's
delegation example is corrected too: its source is the retained publisher grant,
not an ordinary NS lookup result.

Invocation routing rechecks the original deadline before admitting local or
remote business work. Timeout, abandoned read, rejected preparation or changed
leadership cannot replay an invocation. Local RPC and remote invocation owners
retain their established cancellation/uncertain-result semantics after admission.
The [C18 row](../../reference/cleanup-recovery.md) records this extension of the
existing userspace transaction family. No new custody registry or physical
completion proof is introduced.

## Regressions and model

The [former stale-read witness](../../../crates/catten-graft/examples/audit_receive_lease.rs)
uses the same three-node vote/append/application history. n2 commits value 2;
n1 then admits a new read and receives n2's delayed term-one acknowledgement of
the old prefix. The new fence remains uncommitted and the query returns
`ERR_READ_BARRIER`, without returning value 1. The executable now exits zero for
the corrected behavior, rather than reproduction of the old gap.

Host checks cover distinct fences for concurrent reads, readiness without query
execution, consuming completion, abandoned reads, uncommitted and unapplied
fences, compaction past a fence, identity/membership/term changes, domain
replacement, preparation rejection and non-leader rejection. Earlier
receive-time tests now qualify bounded observations rather than query authority.

The revised [`CharlotteRaftRead`](../../tla/CharlotteRaftRead.tla) includes the
external leader replacement and delayed-response environment in its safe case.
It passes **38,300 generated states, 9,916 distinct states, depth 15**. All
fourteen required actions have positive successor coverage.

| Weakened action | Required violated invariant |
|---|---|
| `UnsafeReadBeforeCommit` | `ReadHasCurrentCommit` |
| `UnsafeReadBeforeApply` | `ReadHasAppliedCommit` |
| `UnsafeReceiveLease` | `ReadHasFenceProof` |
| `UnsafeReadBinding` | `ReadBoundToAdmission` |

The old receive-time gate is now a weakened control. Its counterexample includes
read admission, reply capture, external election and delayed delivery. A ghost
proof records quorum confirmation after the particular admission, including
confirmation before a delayed delivery. The model omits bytes, allocation,
syscall readiness, persistence internals and physical cleanup. Other Raft models
supply the assumed quorum/log/election contracts; Rust checks exact Arc identity
and snapshot term lookup.

The first draft omitted old-prefix contact from the environment, so its
current-term omission control unexpectedly satisfied its invariant. The harness
rejected that lost sensitivity. Restoring the source regression's old-prefix
contact path makes the intended control fail. This model-authoring failure is
retained separately from passing checks.

The final read block runs one safe and four negative cases. The remaining **135
cases** reuse the committed evidence chain. All **170 unchanged prior
model/config hashes** match; restoring only the prior read block recovers the
old harness byte for byte. The configured inventory is **33 modules, 35 safe
cases and 105 expected counterexamples**, with **174 model/config/harness
fingerprints**. No fresh full-suite run is claimed.

## Validation and limits

- Raft host suite: **51 passed, zero failed**. Services host suite: **145 passed,
  zero failed**. The former stale-read executable passes with query rejection.
- Warning-denied Clippy passes for all host Raft targets and the ARM DNS target.
  Workspace formatting, shell syntax, local documentation targets and diff
  whitespace checks pass.
- The ARM service bundle builds/signs with `scripts/build-catten-services.sh
  --embed`; `--check` verifies the staged signatures against final source.
- Fresh-storage two-node ARM DNS integration checks catalog lookup, invocation
  routing and generation-fenced endpoint-death retirement. The final fixture
  also checks that local placement returns no capability and closes its
  completed call. Both final guests report 20 passed / zero failed / zero pending,
  bitmap `0xfffff`; per-guest results and hashes are in evidence. Native delayed
  packets, allocation/mapping faults, waiter-binding
  rejection and timeout/drop of these new phases were not injected.
- The 233-page manual builds in temporary output. Contents pages 14–16,
  the changed delegation pages and Chapter 21 receive visual review; the tracked
  user PDF remains unchanged. An initial service build used two incorrect owned
  API method names and reported unused imports; those were corrected without
  suppressing warnings, with the failure retained separately. A later artifact
  check detected a stale staged DNS ELF; a sequential rebuild/sign/check and
  repeated native run provide the final artifact evidence. No cause is inferred
  from that mismatch alone. A manual command also used the wrong working
  directory; its failed output is retained separately from corrected builds.

Bounded status/ingress observations still use receive-time freshness, including
the leader branch. They grant no catalog query authority and remain a separate
lease qualification boundary. Authenticated network identity, raw transport
ownership/readiness, broader metadata admission and general storage-failure
recovery remain open. Shared protobuf/no-op grammar, durable formats and runtime
ABI are unchanged. The Graft checkout is unchanged; no new Jepsen, upstream
execution or mixed Charlotte/Graft cluster result is claimed. Numbered security
finding dispositions remain unchanged.
