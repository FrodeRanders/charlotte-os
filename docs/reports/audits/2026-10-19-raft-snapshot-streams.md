# Raft snapshot continuation ownership — 19 October 2026

## Result and reproduction

Baseline `c8fdfe1` commits the per-read quorum fences. The next source review
found that partial snapshot reception retained boundary index/term and byte
offset, without the leader identity or RPC election term that supplied them.
A higher-term snapshot continuation could append to the earlier buffer.

Two host regressions fail before the correction (exit 101): n2 begins a
term-one stream at snapshot boundary 7/1 with `old-prefix`; then either n3 or
n2 sends a term-two final suffix at the expected offset. Both install
`old-prefixnew-suffix`, with snapshot index, commit and applied progress all
**7**. The corrected assertions require rejection, zero next offset and no
application/durable/progress publication. The
[evidence manifest](evidence/2026-10-19-raft-snapshot-streams.json) retains both
failures, diagnostics, hashes and the complete model counterexamples.

These are direct receiver tests with supplied chunks and a recording application,
not a reproduced network schedule or dishonest-peer safety claim. Snapshot
encodings need not be byte-identical between senders. Boundary/offset equality
alone supplies no proof that their byte streams can be concatenated.

## Correction and ownership

The private `PendingSnapshot` owns leader ID, RPC term, snapshot boundary,
offset and bytes together. Local elections and higher-term observations discard
the former partial buffer, including term observation through votes or appends.
A replacement must start at offset zero; a nonzero continuation without its
exact owner rejects. Successful reception still requires the exact boundary and
offset before appending, application acceptance before durable publication, and
the existing atomic snapshot/suffix store update.

Stale-term, conflicting current-term leader, wrong-boundary and wrong-offset
requests preserve the accepted current stream. A correct continuation can then
complete it. An explicit offset-zero request replaces a partial stream. The
regressions check both paths, rejected publication, successful replacement,
local election and indirect higher-term observation. Public mutable access to
the receive buffer is removed; no external caller used it.

Discard releases only ordinary core heap storage in the serialized userspace
reactor. No capability, kernel root, hardware operation or retry receipt is
introduced. The [C18 row](../../reference/cleanup-recovery.md) records that
containing context and its limits. Existing snapshot size bounds remain; general
fallible heap admission is still separate work.

## Comparison with Graft

The unchanged Graft checkout is at
`1a818d416a40330bea2055e03f61580679baa9f6`. Version-controlled evidence retains
narrow source extracts and complete file hashes for each inspected port.

| Port | Inspected receive behavior |
|---|---|
| Java | `PendingSnapshot` retains leader ID and boundary. A sender/boundary change starts an empty buffer; exact offset is checked. The owner has no captured RPC term. |
| C++ | Restarts on offset zero or boundary change; checks byte offset against buffered size. The inspected receive fields do not bind sender/RPC term. |
| Rust | Offset zero creates a buffer with boundary/offset/bytes. The inspected continuation appends to that buffer without Charlotte's boundary/offset checks or an explicit sender/RPC-term binding. |
| Charlotte | Captures sender and RPC term as well as boundary/offset, clears on election/term change, and requires an explicit zero-offset replacement. |

This comparison identifies source differences. Upstream core/runtime execution,
Jepsen and mixed Charlotte/Graft clusters were not run. It does not establish an
upstream failure from isolated handlers. Protobuf, snapshot envelope, persistent
formats and runtime ABI remain unchanged.

## Executable model

[`CharlotteRaftStream`](../../tla/CharlotteRaftStream.tla) separates continuation
identity from the existing durable snapshot and application-rejection models.
Its two senders share one boundary, with two election terms and unit-sized
prefix/final chunks. Term change may retain the leader or select its replacement;
local election can leave the leader unknown until recognition.

The safe configuration passes **1,122 generated states, 400 distinct states,
depth 9**. All ten required actions have positive total successor coverage.
Two weakened configurations fail through actual final-chunk acceptance:

| Weakened contract | Required violated invariant |
|---|---|
| Retain the old buffer across term change and omit continuation binding | `InstalledSingleOwner` |
| Accept a final chunk at the wrong offset | `InstalledInOrder` |

Rejection actions abstract packet observations. The Rust tests exercise exact
term, leader, index, boundary-term and offset fields. The model assumes one
immutable byte image per sender/term/boundary. The wire has no stream ID or
snapshot digest; same-episode image replacement and stale retries need separate
qualification. Authentication, bytes/parsers, allocation, crash recovery,
persistence and raw transport scheduling are outside this model; the existing
snapshot models retain their separate contracts.

The three new cases run freshly; the **140 unchanged cases** reuse the committed
read-fence evidence chain. All **173 unchanged prior model/config hashes** match.
Removing only the new stream block restores the prior harness byte for byte.
The inventory is now **34 modules, 36 safe cases and 107 expected
counterexamples**, with **178 model/config/harness fingerprints**. No fresh
full-suite run is claimed.

## Validation and limits

- Raft host suite: **55 passed, zero failed**; warning-denied Clippy passes for
  all host Raft targets. Four new tests cover continuation identity, successful
  restart/replacement, preserving rejection and election/term invalidation.
- The signed ARM service bundle builds with
  `scripts/build-catten-services.sh --embed`; `--check` verifies final-source
  freshness and staged signatures before native execution.
- Fresh-storage ARM DNS/Raft pair: **20 passed / zero failed / zero pending
  on each guest**, bitmap `0xfffff`. Full-name invocation, local placement
  without capability authority and generation-fenced endpoint-death retirement
  pass. The kernel hash is unchanged from the read-fence run; the signed DNS and
  Raft ELF hashes changed and the disk images stage those verified new objects.
  Native tests do not inject a cross-leader snapshot stream, allocation failure
  or a crash during assembly.
- The 234-page manual builds in temporary output. Contents pages 14–16 and
  Chapter 21 pages 196–215 pass visual review. The tracked user PDF is unchanged.
  Workspace formatting, shell syntax, documentation link targets and diff
  whitespace checks pass.
- The first TLC launch hit the sandbox's local Java socket restriction and
  exited before exploration. The authorized repeat checks all three cases.
  An intermediate warning-denied Clippy run rejected a redundant borrow in a
  new test's failure diagnostic; the corrected repeat passes. Both failures
  are retained separately.

Bounded status/ingress receive-time freshness, authenticated network identity,
same-episode byte-image stability, raw transport ownership/readiness and general
heap/storage-failure recovery remain open. Numbered security finding dispositions
are unchanged.
