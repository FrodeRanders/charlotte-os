# Committed ingress availability

Raft membership supplies the complete trusted/routable member set. New-flow
eligibility additionally requires the catalog's committed availability
projection, intersected with signed shutdown drains and, for named services,
the exact deployed generation's ready placements. An unavailable backend
remains a voter and retains its member route; health observation grants no
retirement, removal or physical cleanup authority.

The current leader derives a candidate only after a current-term prefix commits
and its existing receive-time quorum check succeeds. It includes itself and
voters with a recent successful replication response that acknowledges a log
entry in the current term. Unknown/old-term responses retain Raft's existing
rejection rules. Followers never filter the set using their own peer timers.
This is receive-time availability evidence, not authentication, an instantaneous
failure detector or a promise that the peer remains alive.

One `Publisher` retains the accepted log index and exact expected generation/view
until commitment. It checks the current catalog record rather than depending on
Raft's bounded result cache. Changes observed while a proposal is pending wait
for it; rejection after commitment fails closed instead of replaying it. A new
leader derives a separately fenced projection. No application mutation or
uncertain accepted request is automatically retried.

`NameCatalog` stores one bounded record: generation, leader term, membership
epoch, leader node key and sorted unique eligible node keys. The current term,
membership and recognized leader must match before DNS materializes a snapshot.
Absent/mismatched projections reject; no all-voters fallback exists. The
availability generation fences the service's flow epoch, including unnamed
bootstrap VIPs. Recovery of the same backend set gets a fresh generation.

The router retains the existing bounded flow bindings and snapshot history.
An existing binding, including a retransmitted SYN, keeps its original epoch;
availability changes do not rehash it to another backend. Missing historical
epochs reject. Only new unbound flows use the current set. This does not preserve
TCP/TLS sessions at a failed backend or replicate ingress-local flow caches.
Missing discovery routes still reject the whole member snapshot, preserving the
existing all-or-nothing routing contract.

## Configuration

The contact-age bound uses the existing
`catten_services::frouter::SNAPSHOT_SOURCE_MAX_AGE_MS`, currently **1000 ms**, in
`crates/catten-services/src/lib.rs`. It derives from `SNAPSHOT_REFRESH_MS`, also
used for route/assignment/membership refresh. The router's local admission lease
is five refresh intervals. `cluster_ingress::MAX_BACKENDS`, currently **64**,
bounds both the projection and member snapshots. These are compile-time
settings requiring service rebuild/restart. Review election timing, replication
latency, projection convergence and bounded flow-history retention together.
Raising the backend count also requires checking Raft command-size and complete
membership-record/frame limits; it cannot authorize a clamped prefix.
The leader's existing, separate quorum-observation window is 750 ms in
`RaftNode`; changing the router refresh interval does not retune that check.
Unrelated readiness wakes cannot extend an accepted request's deadline.

## Current persistence grammar

Catalog command `0x0f` is exactly:

```text
tag:u8 | term:u64 | membership_epoch:u64 | leader_node:u64 |
eligible_count:u16 | eligible_nodes:[u64]
```

Integers are little endian. Term, leader and node keys are nonzero. Count is
1..=64; nodes are strictly increasing and include the leader. The command term
must equal its Raft log-entry term. Older terms/membership epochs and a different
leader in the same term reject without mutation. An identical view is idempotent;
a changed view increments its positive, checked generation without wrap.

Catalog-v19 (`CATALOGJ`) appends a mandatory availability presence flag. Zero
means explicitly unknown. One carries `generation:u64 | command_len:u32 |`
the complete command above. Other flags, malformed/bounded lengths, zero or
unrepresentable generations, missing sections and trailing bytes reject before
any live catalog state changes. Earlier catalogs reject. Pair-v4
(`CRFTSTA4`/`CRFTLOG4`) rejects earlier stores, including log-only old grammar,
before writes/replay. There is no automatic reset, conversion or decoder
fallback. Future deployed stores require an explicit interruption-tested
migration; current pre-production fixtures use fresh storage.
Rebuild/restart the cluster service images together; mixed-format rolling
upgrades are not supported or qualified by this change.

See [catalog snapshots](catalog-snapshots.md), [persistence](raft-persistence.md)
and [execution evidence](../reports/audits/2026-10-27-security-ingress-availability.md).
