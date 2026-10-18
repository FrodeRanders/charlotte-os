# Raft response term and progress fencing — 18 October 2026

## Result and source regression

Baseline `62c49a3` commits the preceding snapshot-model batch. Reviewing the
remaining source/model differences found an unsafe commit in
[`handle_append_entries_response`](../../../crates/catten-graft/src/node.rs).
The handler rejected higher-term responses by stepping down, but still credited
successful replies from an older term.

A host regression reproduces this without a dishonest peer:

1. Node n1 leads term one and appends through index three. Voter n2 acknowledges
   that prefix, but its response is delayed.
2. A different leader, n3, installs a shorter two-entry term-two prefix on n1.
3. Node n1 wins term three with that newer prefix and appends its no-op at index
   three. Voter n2 can legitimately vote for this newer-term log while still
   holding the old prefix.
4. The delayed term-one acknowledgement arrives. The old handler credits index
   three against the replacement and commits/applies through it. The regression
   fails with **commit index 3; expected 0** (exit 101).

A separate failing pre-fix run confirms that an older success replaces confirmed
match index three with one, and an unadmitted higher-term source can step the
leader down. These failed runs are preserved separately in the
[evidence manifest](evidence/2026-10-18-raft-response-fencing.json), including
assertions, output fingerprints and the reviewed source hashes. No native
old-term response injection was performed; the runtime reproduction is a host
consensus-core test.

## Correction

Append responses now require a matching admitted source and exact current term.
Known higher-term responses still step the node down. A successful index beyond
the local log tail rejects before arithmetic, progress or contact credit.
Reordered successes retain the strongest confirmed match and next index;
current-term failure cannot back off below that confirmed prefix. Old-term
failure cannot back off current work or refresh quorum contact.

Completed-snapshot responses use the same source admission, local-tail bound
and monotonic progress helper. Both the request-completion indicator and the
response's `done` must permit final progress; a successful partial chunk cannot
commit it. Committed pending joiners remain valid replication peers before
promotion. Only configured voters contribute commit quorum.

The change adds no resource owner, cleanup path or recovery registry. Existing
transport source identity remains an input. Relmsg source/embedded peer checks
and direct IPC source binding do not establish authenticated cross-machine
identity; a peer lying about its durable prefix remains outside Raft's failure
model.

## Executable response model

[`CharlotteRaftResponse`](../../tla/CharlotteRaftResponse.tla) separates delayed
acknowledgements from the existing log model's atomic replication action. The
finite schedule crosses terms one, two and three, with a fixed self-plus-one
voter quorum and distinct learner/pending-joiner progress. Indices are bounded
through three, and a qualified higher-term observation ends leadership at four.
Current-prefix replication is distinct from response delivery, so a lower
acknowledgement can arrive after a stronger one.

The safe instance passes: **687,124 generated states, 49,395 distinct states,
depth 16**. All ten required actions have positive total successor coverage.
Four controls fail for their intended invariant:

| Weakened action | Required violated invariant |
|---|---|
| `Commit` | `CommitHasCurrentPrefix` |
| `UnsafeRegressSuccess` | `ConfirmedProgressRetained` |
| `UnsafeBackoff` | `BackoffRespectsConfirmedPrefix` |
| `UnsafeInvalidCredit` | `AcknowledgementsHaveProof` |

The old-term control reaches a commit without the current voter prefix; it is
not merely a rejected-field assertion. Invalid-response credit is modeled as
crediting the voter without prefix proof; detailed field/source rejection is
qualified by the Rust tests. Election legitimacy, truthful durable peer replies,
fixed roles and the chosen replacement prefix are assumptions from other layers.
Request correlation, snapshot chunk offsets, removal/re-admission epochs,
Byzantine behavior, persistence and allocation are outside this model.

## Snapshot model correction

Validation of the new model caught an unparenthesized boolean-assignment error.
Reviewing the preceding snapshot model found the same mistake in its `joining`
assignment. TLA+ parsed the trailing conjunction as an action guard rather than
the complete assignment value. Successful join-clearing activations had no
successor, while the broader streamed/recovery action labels still had coverage
from admission-retaining cases.

The historical [snapshot report](2026-10-18-snapshot-restore-model.md) and
[evidence](evidence/2026-10-18-snapshot-restore-model.json) remain unchanged. Their
124-state instance checked rejection but **did not establish its claimed
join-completion coverage**. The corrected expression is parenthesized, and the
harness now separately requires streamed completion, streamed retention,
recovery completion and recovery retention. All four have positive coverage.

The corrected snapshot instance passes **372 generated states, 168 distinct
states, depth 14**; all seven counterexamples still fail as intended.
No snapshot runtime behavior changed in this correction. The first response-model
attempt exited 75 for an incompletely specified successor; that model-authoring
failure is recorded separately from passing checks and production regressions.

## Validation

- Raft host suite: **36 passed, zero failed**. Four new tests cover replaced-prefix
  old-term success/failure, reordered append/snapshot success and failure floors,
  source/local-tail checks, higher-term step-down and completed-snapshot gating.
  Existing admission promotion, election, membership and persistence tests pass.
- Services host suite: **145 passed, zero failed**.
- Warning-denied Clippy passes for all host `catten-graft` targets. Workspace
  formatting, Bash syntax and diff whitespace checks pass.
- `scripts/build-catten-services.sh --embed` builds/signs the complete ARM bundle;
  `--check` confirms that the final source reproduces its staged signatures.
- ARM base boot: **17 passed / zero failed / zero pending**, bitmap `0x1ffff`.
  NVMe Raft term/vote restart passes. Kernel SHA-256
  `53786db1afe18262eba044f163ee32480521d08ffcac165a3952108ebd0d403c`.
- Two-node ARM DNS repeat: **20 passed / zero failed / zero pending on each**,
  bitmap `0xfffff`. Catalog replication, remote/full-name invocation and
  generation-fenced endpoint-death retirement pass. Kernel SHA-256
  `8fefd47edfe60e750091daa051775f485fa8c6b416e5ecc8c646c47339b67962`.
  The first pair exited 1 on each node with **18 passed / one failed / one pending**,
  bitmaps `0x3ffff / 0x40000 / 0x80000`: discovery timed out and DNS remained
  pending. Its serial files began 128.18 seconds apart; the coordinated repeat
  began 80.94 seconds apart and passed with the same kernel/bundle. Late peer
  startup is the observed fixture difference; this does not establish other
  discovery/Raft failure causes. The source diagnostic was
  `el0_disco.rs:116:26: self-test deadline expired while waiting for EL0 disco cross-node peer`.
  Both failed captures and the repeat have separate inline evidence.
- The 232-page manual builds; contents pages 14–16 and Chapter 21 pages
  196–213 pass visual review, with no Chapter 21 overfull boxes. The tracked
  user PDF is preserved.

One intermediate new host test incorrectly expected an empty snapshot-offset map,
although leader construction initializes configured peers to zero. It was
corrected to compare the unchanged map; that fixture failure (35 passed / one
failed) is retained separately. No production correction was needed for that
assertion.

The final harness runs the five new response cases and all eight corrected
snapshot cases. The other **119 cases** reuse the committed baseline chain.
All **157 prior model/config hashes except the changed snapshot module** match;
149 of those inputs underpin the reused cases. Removing the response block and
restoring the former snapshot coverage list recovers the prior harness byte for
byte. This is incremental verification, not a fresh full-suite invocation. The
configured inventory is now **32 modules, 34 safe configurations and 98 expected
counterexamples**, with **165 model/config/harness hashes** retained in evidence.

## Limits and remaining work

This correction establishes response term/progress fencing in the reviewed core
and finite projection. It does not establish linearizable reads from the current
receive-time contact lease: lease duration, delayed same-term replies and
clock/election assumptions need separate work. The existing membership model's
all-member finalization gate remains stronger than the implementation, and a
leader retained only as a learner still needs a separate behavior review. The
[conformance map](../../tla/CONFORMANCE.md) keeps those differences explicit.

Numbered finding dispositions remain unchanged. No new cleanup/custody family,
physical completion proof, production enrollment or authenticated network
boundary is claimed. The initial security-model batch list is now addressed in
its documented finite scopes; that is not complete subsystem qualification.

## Artifact fingerprints

- Response model: `3d18cf384a8769934d40b538d1b66a62c347f24429e03e347076c359972fa3fa`.
- Corrected snapshot model: `c71563b3814d1f58b6bee72dac6bfd78e76d55d50895dad5c70372002f42eb88`.
- Harness: `c09413db2254ab6f1f50a3d18c93fbff0af98989b758abbc8202acf21f96763e`.
- TLC JAR: `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88` (TLA+ Tools v1.7.4, TLC 2.19 revision 5a47802).

The evidence manifest includes complete counterexample states, positive action
coverage, source and artifact hashes, failed-run diagnostics and native result
excerpts. Reviewing this report does not require temporary logs.

Signed ARM service bundle SHA-256 (sorted `filename:ELF-digest` lines):
`c7708f21a71a6711445d524f54b3ba8fd5faddc92198de36c6b42922fd577632`.
Raft ELF: `da15e2b69cf716e9691ada836b2877db01c70788e2a5ac8c958cd7d9213ab878`;
DNS ELF: `722ae26b6733773059744f90d5423068edd793da496b48e859a12b126b34723b`.
