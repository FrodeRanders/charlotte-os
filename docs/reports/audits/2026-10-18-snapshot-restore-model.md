# Application snapshot acceptance and recovery — 18 October 2026

## Scope and result

Baseline `21155f0` commits the preceding boot-trust model batch. This review
compares application snapshot rejection in the current Raft/catalog source and
adds [`CharlotteSnapshotRestore`](../../tla/CharlotteSnapshotRestore.tla).
Existing source already enforces the reviewed rejection ordering. No Rust change,
new runtime fix or audit-finding closure is claimed.

The safe instance passes: **278 generated states, 124 distinct states, depth 9**.
All thirteen required actions have positive successor coverage, including both
rejection classes, streamed/recovery activation and the two persistence-failure
branches. Seven weakened configurations fail for their intended invariant.
The Raft host suite passes **32 tests** and the services host suite passes
**145 tests**, both with zero failures.

## Source comparison

- [`StateMachine::restore`](../../../crates/catten-graft/src/state_machine.rs)
  requires rejection without changing state. Its default accepts only empty
  application bytes; the catalog requires its complete current format.
- [`NameCatalog::restore_bytes`](../../../crates/catten-services/src/name_catalog/snapshot.rs)
  builds every section in temporary collections. Unsupported magic, malformed
  fields, missing sections or trailing bytes return before live replacement.
  Publication uses separate section locks after all validation.
- [`RaftNode::new` and `handle_install_snapshot`](../../../crates/catten-graft/src/node.rs)
  propagate application rejection before exposing a constructed node or changing
  snapshot/log progress, live membership or durable join admission. Rejected
  network installation discards its pending stream and returns failure/not-done
  at offset zero. Normal term/vote and leader-contact processing may already have
  occurred; these are deliberately outside the preserved-state tuple.
- [`DiskLogStore`](../../../crates/catten-services/src/disk_raft.rs) persists after
  application restore. Write/flush rejection calls `persist_fail_stop`, which
  logs and calls `thread_exit`. The calling Raft operation cannot continue to
  publish progress. This is not evidence that sibling threads stop or endpoints
  are withdrawn.
- [`DNS startup`](../../../crates/catten-services/src/bin/dns.rs) stops startup
  on rejected persistent-store open or fallible node construction. Its
  [`CatalogMachine`](../../../crates/catten-services/src/bin/dns/catalog.rs)
  forwards the catalog's restore result.

## Finite contract

The model represents two indices, three membership classes (including self,
excluding self, and joint including self), streamed versus recovery entry and
three application verdicts. Unsupported headers reject early; malformed data
can reject after temporary sections have been parsed. Rejection must preserve
application state, durable snapshot, commit/applied progress, live membership
and durable join admission. Recovery cannot expose a node before acceptance.

A join admission records index one. A successfully restored newer snapshot can
clear it only when membership includes self. Recovery of the same old snapshot
retains admission even when that membership includes self. The model abstracts
full current/next peer sets and roles to these classes; the separate
[`CharlotteRaftSnapshot`](../../tla/CharlotteRaftSnapshot.tla) retains chunk,
suffix and membership-set coverage.

Streamed application restore precedes durable snapshot replacement. Definite
persistence failure leaves the old durable object; ambiguous failure may leave
the whole new one. Either stops this caller, and a fresh recovery validates
whatever durable object is present. This assumes the existing atomic whole-object
storage contract. It does not establish an application-plus-disk transaction.
`exposed` describes this Raft caller's ability to return/continue with the node;
it does not represent global service availability or physical lifetime.

| Weakened action | Required violated invariant |
|---|---|
| `UnsafeRejectApplication` | `RejectionPreservesApplication` |
| `UnsafeRejectDurable` | `RejectionPreservesDurable` |
| `UnsafeRejectProgress` | `RejectionPreservesProgress` |
| `UnsafeRejectMembership` | `RejectionPreservesMembership` |
| `UnsafeRejectJoin` | `RejectionPreservesJoin` |
| `UnsafeExposeRecovery` | `RecoveryExposureRequiresRestore` |
| `UnsafePublishBeforeRestore` | `PublicationRequiresRestore` |

Each rejection control changes the outcome after rejection without admitting a
successful restore. The ordering control publishes durable state after validation
but before restore. Complete counterexample state sequences, positive coverage,
source/input hashes and host-test results are retained in
[version-controlled evidence](evidence/2026-10-18-snapshot-restore-model.json).

## Verification and limits

The final eight new cases ran through the current `check.sh` functions and exact
new case block. Removing that block restores the committed harness byte for byte.
All **149 preceding model/config hashes** match the
[boot-trust baseline](evidence/2026-10-18-boot-trust-model.json); its recorded
results and prior evidence chain are reused. This is incremental verification,
not a fresh single full-suite invocation. The configured inventory is now
**31 modules, 33 safe configurations and 94 expected counterexamples**;
the evidence records all **159 model/config/harness hashes**.

Host validation ran `cargo test --locked --manifest-path
<repo>/crates/{catten-graft,catten-services}/Cargo.toml --lib` from outside the
repository Cargo-config directory. Relevant passing tests include:

- rejected restart preserves durable join admission, term/vote and snapshot;
- rejected streamed installation preserves log/progress/membership/join state,
  then a supported replacement succeeds;
- every truncated prefix and an extended catalog snapshot reject without live
  publication, alongside retired tags, duplicate keys and noncanonical flags;
- joint membership, persisted application bytes and matching suffix recover,
  and retired persistent pairs reject before replay.

There is no new native boot or storage fault injection. Byte decoding,
cryptographic validation and field relationships are source/test obligations,
not proved by the verdict abstraction. Infallible allocation and separate
publication locks remain: neither allocation failure nor concurrent-reader
snapshot isolation is established. The model also omits no-snapshot startup,
same-process replacement retries, raw membership-envelope parsing, election/log
replay, endpoint teardown and physical cleanup/quiescence. No cleanup owner or
recovery-custody behavior changes, so no cleanup family is newly qualified.

The manual and living conformance/reference documents describe this scope.
Manual source was built and its changed chapter rendered for visual inspection
in a temporary directory; the tracked user PDF remains untouched. The 231-page build and contents pages 14–15 / chapter pages 196–212 pass
visual review; Chapter 21 has no overfull boxes. Existing warnings elsewhere
remain outside this batch.

## Artifact fingerprints

- Model SHA-256: `b8db4b8d7d2b6ff71c2a615d2d9bc02f04e05c31feff88747ddfe276eddca24a`.
- Harness SHA-256: `51e45de1de2e1ac89cc5dd61cd4205b61646f3bc08ecc14fdc243fef3ec0788d`.
- TLC JAR SHA-256: `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88` (TLA+ Tools v1.7.4, TLC 2.19 revision 5a47802).
- Host output SHA-256: Raft `81b32cf776c088aa02e5ddce2e16019ece9bdf86fba11c342c8a70b6d9871011`; services `207f599aa72c7c8e674295e175e91ddc8edd0dc90aab5955ab870e11d3aabf8d`.

The JSON contains the complete checked-input manifest and counterexamples;
temporary logs are not required to review the results.
