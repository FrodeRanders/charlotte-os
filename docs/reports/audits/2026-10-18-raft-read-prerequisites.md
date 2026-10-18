# Raft read prerequisites — 18 October 2026

## Result and regression

Baseline `941352f` commits the membership/parity batch. Continuing the read audit
found that Charlotte admitted the query gate after an exact-term response from
a quorum even when its election no-op remained uncommitted. Recent contact
alone did not establish the inherited committed prefix.

The host regression starts with a term-one log entry and an unknown commit
index. The node wins term two and appends its election no-op. A valid term-two
success acknowledges only index one. That records quorum contact but cannot
commit the term-two no-op at index two. The previous gate nevertheless returns
true. The pre-fix test fails with:

```text
assertion failed: !node.can_serve_linearizable_read()
test result: FAILED. 0 passed; 1 failed; 39 filtered out
exit status: 101
```

This reproduces incorrect read admission in the consensus core. It does not
execute a stale application query or a complete client history. The
[evidence manifest](evidence/2026-10-18-raft-read-prerequisites.json) retains the
failed assertion, source fingerprint, final checks and model counterexamples.

## Correction

The leader read gate now requires a nonzero commit index whose log term is the
current term, and application through exactly that commit. The existing
leadership and quorum-contact checks still apply. `term_at` resolves a compacted
snapshot anchor, so compaction does not remove a valid commitment proof.

The main regression checks rejection with both zero and known older-term
commit indices, then acceptance after the current-term no-op commits and
applies. Additional checks cover single-voter election/no-op commitment,
compaction, contact expiry, and an isolated application-lag fixture. That lag
fixture changes the public progress field directly: the synchronous apply loop
normally closes this interval before the service reactor can query the node.
It checks an explicit invariant rather than reproducing a second reactor bug.

`handle_client_query` rejects the premature query with `ERR_READ_BARRIER`.
Leader-side `can_serve_bounded_read` uses the same gate. Follower bounded-staleness
policy is unchanged. DNS catalog queries and leader ingress observations inherit
the guard through their existing call sites. There is no new RPC, wire format,
wait, resource owner or cleanup path.

## Graft comparison

The reference checkout remains clean at
`1a818d416a40330bea2055e03f61580679baa9f6`. Java and C++ already check applied
progress in the inspected core read gates. The inspected Java, C++ and Rust
core read gates do not explicitly require a current-term commit. Java has a
separate barrier path; C++ and Rust also have runtime refresh and query logic.
Their scheduling and state access differ from Charlotte's reactor.

Six source extracts and file hashes are retained in evidence, covering the
three core gates and the C++/Rust query or refresh boundaries. This is a static
comparison. No upstream failure is claimed from it, and no upstream tests,
Jepsen run or mixed Charlotte/Graft cluster were executed. The change adds a
necessary read prerequisite to Charlotte while leaving the shared message
grammar intact. The [conformance reference](../../reference/raft-conformance.md)
keeps that distinction explicit.

## Finite read model

[`CharlotteRaftRead`](../../tla/CharlotteRaftRead.tla) separates election,
old-prefix acknowledgement, current-term commitment, application and query
admission. Initial state permits either an unknown or known term-one commit.
Read evidence captures the state at admission, so later contact expiry or
step-down does not invalidate a historical read proof.

The safe instance passes: **50 generated states, 20 distinct states, depth 7**.
All nine required actions have positive successor coverage, including an
accepted read and both prerequisite rejection paths.

| Weakened action | Required violated invariant |
|---|---|
| `UnsafeReadBeforeCommit` | `ReadHasCurrentCommit` |
| `UnsafeReadBeforeApply` | `ReadHasAppliedCommit` |

Both negative controls exercise their named action and fail for their intended
invariant. The projection assumes the quorum/log/election properties checked by
the other models. It treats contact as an input; it does not prove that contact
establishes a safe lease or current leader authority. Snapshot term lookup is
checked by Rust rather than duplicated in this model.

The first temporary harness used the temporary directory as its model path and
failed to find the configuration. After correcting that path, TLC passed the
draft but the harness rejected the missing `Read` coverage label: TLC reported
the aliased `ReadWith` body. A conjunctive wrapper preserves the named action;
the coverage requirement was retained. Both authoring failures are recorded
separately from the production regression and final passing checks.

The configured suite now contains **33 modules, 35 safe configurations and 103
expected counterexamples**. This batch executes the three new cases; **135
unchanged cases** reuse the committed evidence chain. All **167 prior
model/config hashes** match. Removing only the read block restores the preceding
harness byte for byte. Evidence retains **172 model/config/harness hashes**.
No fresh full-suite invocation is claimed.

## Validation

- Raft host suite: **42 passed, zero failed**. Services host suite: **145 passed,
  zero failed**.
- Warning-denied Clippy passes for all host Raft targets. Workspace formatting,
  Bash syntax, local documentation targets and diff whitespace checks pass.
- `scripts/build-catten-services.sh --embed` builds and signs the ARM bundle;
  `--check` confirms the final source matches its staged signatures.
- Fresh-storage two-node ARM DNS integration: both runners exit zero with
  **20 passed / zero failed / zero pending**, bitmap `0xfffff`. Discovery,
  Raft catalog replication, remote invocation and generation-fenced endpoint-death
  unregistration pass. The targeted premature-read schedule is a host test;
  these native checks cover integration.
- The 233-page manual builds in a temporary directory. Contents pages 14–16
  and Chapter 21 pages 196–214 pass visual review without clipping, overlap or
  broken table rows. No Chapter 21 overfull boxes occur. The tracked PDF is
  unchanged.

Both native runners use kernel SHA-256
`8fefd47edfe60e750091daa051775f485fa8c6b416e5ecc8c646c47339b67962`.
This is a service-only correction; the signed service bundle and ELF hashes,
per-guest results and source hashes are retained in version-controlled evidence.
Final bundle verification checks that the release services still match the
native-run artifacts after the later host-test and comment edits.

## Remaining read boundary

The existing 750 ms receive-time contact window remains. An old same-term
success can refresh it on delivery, and neither current-term commitment nor
application progress establishes that this observation still proves authority.
Election timing, clock assumptions, delayed responses and request correlation
need separate work. Shortening the window alone would not establish that proof.

This batch establishes necessary read prerequisites and keeps the remaining
read-safety gap explicit. It does not establish complete linearizability or
close a numbered security finding. Historical reports and evidence remain
unchanged, and no cleanup/recovery family is reclassified.
