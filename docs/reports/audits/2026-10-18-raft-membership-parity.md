# Raft membership and Graft parity — 18 October 2026

## Result

Baseline `65eff0f6` commits the preceding response-fencing batch. This follow-up
restores Graft's automatic membership catch-up policy and corrects a Charlotte
leader that retained leadership after becoming a learner. It also compares the
port with the Java, C++ and Rust implementations and the upstream Jepsen scenario
definitions. The [evidence manifest](evidence/2026-10-18-raft-membership-parity.json)
retains source extracts, hashes, failed assertions, model traces and native
result excerpts. Reviewing the results does not require temporary logs or the
upstream checkout.

## Automatic finalization

Charlotte previously proposed FINALIZE immediately after applying committed
JOINT. That deliberate divergence was introduced in `4d2856df`; the historical
source and full revision are captured in evidence. Java and Rust Graft instead
wait for the committed JOINT fence and every proposed member to reach it. The
user selected restoration of that upstream behavior during this review.

The automatic proposer now requires a nonzero, committed fence and confirmed
progress through that fence from every proposed remote voter and learner. The
leader already owns the committed prefix locally. A slow proposed learner can
therefore delay automatic completion, even when both voter majorities are
available. FINALIZE commit still requires both voter majorities. The explicit
manual proposer remains a separate path; the catch-up model describes automatic
proposal only.

A host regression commits JOINT with a voter while a proposed learner lags.
Repeated voter acknowledgements append no FINALIZE. After the learner catches
up, FINALIZE is proposed and commits with the voter quorums. Separate checks
reject a zero or uncommitted automatic fence.

## A retained learner relinquishes leadership

The old finalization code stepped down only a decommissioned node. A leader
retained as a learner remained a cluster member, so it also remained in the
leader state. A new host regression reproduced this before correction:

```text
assertion `left == right` failed
  left: Leader
 right: Follower
test result: FAILED. 0 passed; 1 failed; 37 filtered out
exit status: 101
```

Applying membership now makes any non-voter a follower immediately, clears its
leader/contact/election authority, and preserves its existing-term vote in both
memory and durable state. A retained learner remains a member and cannot
campaign, submit commands or serve leader queries. The change deliberately
avoids a step-down helper that clears the vote in the same term.

The regression also checks that one FINALIZE acknowledgement cannot commit
without the new voter majority. Another test covers a next-only voter winning
an election during joint consensus only after obtaining both majorities.

## Upstream comparison and Jepsen scope

The clean reference checkout was Graft revision
`1a818d416a40330bea2055e03f61580679baa9f6`. It was read without modification.
Eleven upstream source files have hashes and relevant extracts retained in
evidence. This is a dated source comparison; upstream tests and Jepsen were not
rerun.

Graft's Java, C++ and Rust implementations are intended to form mixed clusters.
Its Jepsen suite includes Java/C++, Java/Rust and three-language schedules,
including partition and persistence-loss scenarios. That work is a reference
for the port's contract and for future integration tests.

The inspected Jepsen demotion scenario deliberately chooses a non-leader.
Leader removal has a separate scenario. Retaining the current leader as a
learner is therefore a distinct transition covered by the new regression.
Java has a periodic non-voter role check. The Rust core has one too, but its
runtime leader timer callback has different control flow. Core and caller
behavior must be reviewed together before drawing conclusions about an
upstream execution.

The preceding response-fencing correction also has an upstream comparison:
C++ checks the exact response term and preserves monotonic successful progress.
The inspected Java/Rust handlers differ. Static handler differences alone do
not reproduce an upstream failure; the unsafe commit was dynamically reproduced
in Charlotte. The [conformance reference](../../reference/raft-conformance.md)
records these boundaries.

No wire grammar changes are made here. Shared protobuf shapes do not establish
mixed-cluster interoperability: Charlotte uses capability IPC/relmsg and a DNS
application, while Graft's TCP and Jepsen application paths have different
integration boundaries. No Charlotte/Graft gateway or mixed-runtime history
was tested in this batch.

## Membership model

The existing [membership model](../../tla/CharlotteRaftMembership.tla) now
explicitly covers automatic catch-up, a next-only voter election, and a
finalized leader becoming a learner. The safe instance passes with **35,565
generated states, 6,232 distinct states and depth 13**. All ten required actions
have positive successor coverage, including the stalled-member rejection and
next-only election witnesses.

| Weakened action | Required violated invariant |
|---|---|
| `UnsafeFinalizeBeforeCatchUp` | `ProposedMembersCaughtUp` |
| `UnsafeFinalizeLearnerLeader` | `LeaderIsEligible` |
| `UnsafeFinalizeOldOnly` | `FinalizationHasBothQuorums` |

Each control fails for the intended invariant after exercising its named unsafe
action. The quorum proof is captured before replacing the configuration. The
model omits durable voting, log freshness, allocation and transport; Rust
regressions check existing-term vote retention. An initial model draft used
unsupported set-difference syntax and exited 150; that authoring error is
recorded separately from the production regression and passing checks.

The final membership harness ran one safe and three negative configurations.
The other **131 cases** reuse the committed evidence chain. All **162 unchanged
model/config hashes** match; restoring only the old membership block recovers
the previous harness byte for byte. This is incremental verification. The
configured inventory is **32 modules, 34 safe configurations and 101 expected
counterexamples**, with **168 model/config/harness hashes** retained in evidence.

## Validation

- Raft host suite: **39 passed, zero failed**. Services host suite: **145 passed,
  zero failed**.
- Warning-denied Clippy passes for all host Raft targets. Workspace formatting,
  Bash syntax and diff whitespace checks pass.
- `scripts/build-catten-services.sh --embed` builds and signs the ARM bundle;
  `--check` confirms its staged signatures match the final source.
- The final two-node ARM DNS run exits zero on both nodes: **20 passed / zero
  failed / zero pending each**, bitmap `0xfffff`. Both report discovery success,
  Raft catalog replication, remote invocation and generation-fenced endpoint-death
  unregistration. These are integration smoke checks; the targeted membership
  transitions above are host regressions.
- The 233-page manual builds in a temporary directory. Contents pages 14–16
  and Chapter 21 pages 196–214 pass visual review. No Chapter 21 overfull boxes
  occur. The tracked user PDF remains unchanged.

Both native runners use kernel SHA-256
`8fefd47edfe60e750091daa051775f485fa8c6b416e5ecc8c646c47339b67962`.
The service bundle fingerprints and per-guest result excerpts are retained in
the evidence manifest. Service-only changes can leave this kernel fingerprint
unchanged; the signed service ELF fingerprints identify the updated port.

## Limits and remaining work

Read-lease timing, delayed same-term responses and request correlation remain
open. Authenticated cross-machine identity, Graft TCP interoperation and mixed
implementation histories need their own implementation and execution evidence.
Manual FINALIZE is outside the automatic catch-up projection. The source
comparison establishes neither all-port equivalence nor a new Jepsen result.

Historical reports and evidence remain unchanged. Numbered finding dispositions
are unchanged. This batch introduces no cleanup owner, recovery registry or
physical completion proof.
