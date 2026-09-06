# TLA+ / implementation sync — 2026-09-06

## Review scope and method

This synchronization revisits the formal suite after CharlotteOS gained
replica-set placement, exact-generation readiness, distributed L2 ingress,
cluster-wide TCP load sharing, operationally separated connector launch, and
cluster drain and shutdown. The review began from implementation `11d5c3f`; a
7 September follow-up checks the freshness and history repairs made after
formalization commit `81ebb00`. It concentrates on state shared by the cluster
control plane and packet path.

The existing Raft specifications already check how election, log replication,
joint membership, admission, and snapshots establish committed state. The new
model therefore begins at state-machine application and explores asynchronous
materialization into each frame router. This avoids duplicating Raft while
making the cross-layer assumptions executable.

## Impact by model area

| Model area | Later implementation change | Result |
|---|---|---|
| Raft membership and catalog | Membership, desired replica sets, deployment generations, readiness, and shutdown intent now contribute to ingress policy | Existing Raft models remain applicable to commitment. Added a composition layer over their committed output. |
| Service lifecycle | A service may have several concrete owners and per-node readiness for one desired generation | The singleton lifecycle model remains useful locally. Replica readiness is modeled separately in `CharlotteClusterIngress`. |
| DSR ingress | A VIP advertiser selects ready placed backends, retains established flows by epoch, and forwards at L2 | Material new state machine. Added `CharlotteClusterIngress`, one safe configuration, and three negative configurations. |
| Cluster shutdown | Committed drain must stop new placement and new flows before service teardown | The new model includes the control-plane-to-ingress portion. Reverse-order local service and device shutdown remains outside it. |
| Role-separated operational launch | Signed references, replay fences, S3 retrieval, and transient HPKE decryption now precede connector launch | Not yet modeled. A future admission model should abstract cryptography and check role, sequence, expiry, digest, binding, and plaintext-lifetime state. |
| Kafka transactional step | Consumer generation, procedure call, produce, offset inclusion, commit, abort, retry, and DLQ now form a service workflow | Not yet modeled. It merits a separate transaction/fencing specification rather than expansion of the cluster-ingress model. |

## New model boundary

`CharlotteClusterIngress.tla` represents:

- stable voters and the old/new voter intersection during joint consensus as
  the set eligible to participate in ingress;
- replacement deployment generations and concrete replica sets;
- readiness publications bound to an exact deployment generation;
- committed node drain;
- discovery route availability and all-or-nothing snapshot installation;
- asynchronous, immutable per-router snapshot history;
- deterministic VIP advertiser and backend selection; and
- flow bindings retained against a bounded policy history.

Policy versions represent the full immutable identity of the Rust
`BackendSnapshot` inputs. Lease freshness is modeled as a local Boolean with a
weakly fair expiry action: this proves that expired authority cannot admit a
flow and that an unrenewed lease cannot remain fresh forever, but not the real
five-second duration. The model does not prove collision resistance of the
64-bit implementation digest, Ethernet authentication, ARP convergence, TCP
correctness, or globally unique VIP advertisement.

## Findings

1. **Exact-generation readiness composes correctly.** The safe derivation
   admits a backend only when it is an active member, a desired replica, ready
   for the current deployment generation, and not draining. The retained
   negative readiness action demonstrates the stale-generation failure that
   the Rust catalog already rejects.

2. **A complete snapshot needed a new-flow lease.** The frame router retains
   its previous snapshot if DNS cannot materialize a complete replacement.
   That remains useful for established flows. A successful refresh now grants
   five seconds of VIP advertisement and unbound-flow admission; timer failure
   or expiry suppresses both. DNS renews it only from a quorum-fresh leader or
   a follower with a leader-log match no more than one second old; rejected
   replication cannot keep the source witness alive. The negative model retains
   the former unbounded stale-new-flow behavior as a regression witness.

3. **Flow and snapshot bounds are independent.** A flow records an epoch, while
   snapshot history can evict that epoch. Classification now retains the
   binding as a fail-closed tombstone and reports a missing-epoch drop instead
   of selecting from the newest snapshot. The negative configuration retains
   and demonstrates the former remap.

These repairs choose the conservative production contract exposed by the
model: lease freshness is required only to admit unbound traffic, and an
unretained epoch fails closed. Bounded flow-table eviction remains a separate
availability limitation because DSR intentionally has no distributed
connection tracker.

## Validation

- The revised safe two-node, one-flow, two-generation, four-policy
  configuration exhaustively explored 3,522,889 generated states, 576,764
  distinct states, and depth 17 without an invariant violation.
- Required action coverage includes readiness publication and withdrawal,
  replacement, drain, stable-to-joint and joint-to-stable membership, route
  discovery loss/recovery, snapshot installation and lease expiry, new flow,
  existing packet, stale-new-flow rejection, missing-epoch rejection, and flow
  termination.
- The stale-snapshot, history-eviction, and stale-readiness configurations each
  produce their named invariant violation through the intended unsafe action.
- `docs/tla/check.sh` runs the new safe model and all three counterexamples as
  part of the same CI harness as the earlier specifications.

## Next formal work

1. Add advertiser handover and packets observed by different ingress nodes.
2. Model the complete replicated drain-to-local-shutdown ordering.
3. Model role-separated operational admission and connector replacement.
4. Model Kafka transactional-step fencing and uncertain external effects.
