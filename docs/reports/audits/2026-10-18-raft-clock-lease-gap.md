# Raft elapsed time and the receive-time lease gap — 18 October 2026

## Result

Baseline `4468939` commits the read-prerequisite batch. This follow-up corrects
elapsed-time accounting in both service reactors. It also **reproduces a stale
application read** after an honest delayed reply refreshes an obsolete leader.
That second issue remains open; the clock correction does not close read safety.

The [evidence manifest](evidence/2026-10-18-raft-clock-lease-gap.json) retains
source hashes, the failed clock regression, host witness output, complete model
counterexamples and native result excerpts. Historical reports are unchanged.

## Elapsed time correction

DNS and the generic Raft service previously advanced `node.millis()` by 25 ms
when a timer cookie or watchdog indicated a tick. A long reactor pause therefore
counted as one tick. Old contact evidence, election deadlines and transport
timeouts could remain eligible after much more real time had passed. Earlier
watchdog work ensured another tick would happen; it did not measure its delay.

A host regression reproduces the old call-site arithmetic with a committed
leader and contact at 202 ms. After a simulated five-second pause, counting just
one tick leaves the read gate open. The assertion that contact expired fails
(exit 101, zero passed / one failed / 42 filtered). This is an arithmetic/core
fixture, not a native suspended-reactor injection.

The new `MonotonicClock` converts the kernel counter from one captured epoch,
using checked wide arithmetic and preserving fractional ticks across samples.
Both services sample after waits, before inbound work and transport completion
processing, and before scheduled work. Repeated samples cannot manufacture
elapsed time. Timers and the existing CQ watchdog retain their wake-up role.

Zero frequency at initialization, changed frequency, counter regression and
millisecond exhaustion reject explicitly. The service adapter aborts the domain
with pending owners live; it does not synthesize time or restart an epoch.
Host checks cover the entire pause, fractional accumulation, repeated samples,
invalid samples preserving the original epoch and conversion bounds. The core
regression now rejects stale contact and observes the expired election deadline.
The invalid-clock abort branch was not injected in a native guest.

No resource owner, wait source, physical cleanup or recovery policy changes.
Clock values are local protocol time, with no UTC or cross-machine synchronization
claim. Sampling is not atomic with all subsequent service work or read admission.

## Delayed reply: a stale read is still possible

The version-controlled host witness
[`audit_receive_lease.rs`](../../../crates/catten-graft/examples/audit_receive_lease.rs)
drives three actual consensus nodes and two queryable application histories:

1. n1 wins term one with n2 and commits value 1. n2 acknowledges the committed
   prefix again; that honest term-one response is retained without delivery.
2. n1 is isolated. n2 wins term two with n3 and commits value 2 at index four.
   Its application query returns 2.
3. At 5000 ms, n1's old contact has expired and its read gate rejects.
4. At 5001 ms, the retained response arrives. n1 still believes it leads term
   one, so exact-term checks accept it. Its current-term commit and application
   prerequisites remain satisfied. Receive-time credit reopens the gate, and
   its application query returns 1 after the newer write has committed.

The executable prints:

```text
new leader n2: term=2 committed=4 value=2
old leader n1 at 5000ms: expired contact rejects read
COUNTEREXAMPLE: n1 at 5001ms, term=1, committed=2, returned=1 after value=2 committed
```

Run `cargo run --locked --manifest-path <repo>/crates/catten-graft/Cargo.toml
--example audit_receive_lease` from outside the root Cargo configuration.
Exit zero means the counterexample was reproduced; this is an audit witness,
not a passing safety regression or application skeleton. It uses actual vote,
append, commit, application and query methods, with deterministic in-memory
transport delivery. No peer lies, no log is replaced manually, and no native
packet-delay or upstream execution is claimed.

The 750 ms receive-time gate therefore remains unsafe for this schedule.
Leader-side bounded reads use that gate too. Follower freshness is also a
receive-time observation, outside this particular witness. Correct elapsed
time and a shorter lease do not associate a reply with a read's admission.

## Model qualification

The existing [`CharlotteRaftRead`](../../tla/CharlotteRaftRead.tla) retains its
safe prerequisite projection: **50 generated states, 20 distinct states,
depth 7**, with all nine required actions exercised. Its commit and application
controls still violate their intended invariants.

A new diagnostic configuration enables `CaptureDelayedContact`,
`ElectOtherLeader` and `DeliverDelayedContact`. External authority changes while
the isolated node keeps its local term/role. The ordinary `Read` action then
violates `ReadHasAuthority`, even though its current-term commitment and
application conditions hold. This configuration removes no source guard.
It records a known implementation gap. The safe configuration excludes that
environment and does not establish the stronger authority property.

The clock conversion and its sampling contexts are Rust/source obligations,
outside this model. This batch freshly checks one safe case, two weakened
controls and the diagnostic counterexample. The other **135 cases** reuse the
committed evidence chain; all **170 unchanged prior model/config inputs** match.
Removing only the new diagnostic harness block restores the previous harness
byte for byte. The configured inventory is **33 modules, 35 safe cases and 104
expected counterexamples**, with **173 model/config/harness hashes** in evidence.
No fresh full-suite run is claimed.

## Validation and upstream boundary

- Raft host suite: **47 passed, zero failed**. Services host suite: **145 passed,
  zero failed**. The separate counterexample executable reproduces the gap.
- Warning-denied host Raft Clippy, workspace formatting, shell syntax, local
  documentation targets and diff whitespace checks pass.
- The ARM service bundle builds/signs with `scripts/build-catten-services.sh
  --embed`; `--check` verifies staged signatures against the final source.
- Fresh-storage two-node ARM DNS integration: both runners exit zero with
  **20 passed / zero failed / zero pending**, bitmap `0xfffff`. Discovery,
  catalog replication, remote invocation and generation-fenced retirement pass.
  These smoke checks do not exercise the delayed-reply witness or clock fault.
- The 233-page manual builds in temporary output; contents pages 14–16 and
  Chapter 21 pages 196–214 pass visual review without clipping, overlap or
  broken rows. No Chapter 21 overfull boxes occur. The tracked PDF remains
  unchanged. An initial
  manual command used the repository root instead of the manual directory and
  failed to find its input; corrected builds are recorded separately.

The clean Graft checkout remains at
`1a818d416a40330bea2055e03f61580679baa9f6`, without modification. Previous source
extracts document its differing core/runtime read paths. This batch reproduces
Charlotte behavior only. It provides no new Graft Jepsen result, upstream failure
reproduction or mixed-implementation cluster evidence.

## Next read-safety boundary

The read path needs a request-specific quorum barrier, retained across the
reactor round, with leadership and application rechecked before answering.
A per-read committed log fence could preserve the shared wire grammar at the
cost of another durable entry. Correlated probes need an explicit shared
core/runtime contract. A latest-send timestamp alone cannot identify an old
response when requests overlap or are retried.

Numbered finding dispositions remain unchanged. The executable stale-read
witness must remain visible until the replacement read path rejects it.
