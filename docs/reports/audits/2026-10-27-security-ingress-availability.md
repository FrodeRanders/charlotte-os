# Committed backend availability and native management VIP failover

Date: 2026-10-27. Parent: `434be15a`. Follow-up to
[frame-router readiness](2026-10-27-security-frame-router-readiness.md).
SEC-10 remains partial; the ledger remains **29 corrected in scope, seven
partial and two open**. Production remains disabled.

## Gap and correction

The previous full campaign failed because a new VIP flow selected the killed
node, still a member of the three-voter configuration. Its trace identifies
SYNs forwarded to `52:54:00:12:34:03`, without a SYN-ACK. Direct TLS controls
passed, so that failure remained distinct from operator authentication.
The earlier failed run and diagnostics remain in its version-controlled report
and [evidence](2026-10-27-security-frame-router-readiness-evidence.json).

DNS now publishes a bounded availability projection through its existing Raft
catalog. A leader must have a committed current-term prefix and recent quorum
contact before deriving it. Peers additionally need recent successful
replication acknowledging a current-term entry. The record carries generation,
leader term, membership epoch, leader identity and the canonical eligible set.
Term/membership/recognized-leader mismatches reject projection materialization;
there is no all-voters fallback. Followers use applied state, not independent
peer timers. Discovery still supplies routes without authenticating peers.

The publisher retains one accepted log index and exact expected generation/view
until commitment. Its proof survives eviction of Raft's bounded command-result
cache. Failure cannot authorize replay. A changed projection fences the flow
epoch; recovery of the same set receives another generation. Unavailable nodes
remain members, and existing bindings, including retransmitted SYNs, retain
their old epoch/backend. No accepted application mutation is reissued.

The contact-age bound reuses the existing 1000 ms source-freshness setting;
the existing 64-backend bound covers the projection. These compile-time settings,
the independent quorum gate and the refresh/lease/flow-history relationships are
documented in the [contract](../../reference/ingress-availability.md),
[Configuration guide](../../guides/configuration.md) and manual.

## Complete native campaigns

Two fresh three-node ARM campaigns pass the full default scope. Each uses the
ordinary signed/encrypted management profile, verified standard TLS clients,
DHCP and unauthenticated SNTP. No guest clock, role or request budget override is
introduced. All three initial kernel suites pass **18/0/0** in each run.
The three guests intentionally share the fixture's development service
certificate; these runs do not enroll individual node identities or authenticate
the consensus transport.

Both runs observe node 0 leading term 1, admit signed policy generation/sequence
1 through a follower, terminate that leader, observe node 2 leading term 2 and
admit generation/sequence 2 through a surviving follower. Readers are denied the
same mutation body. Both survivors read back sequence 2 with the same validity
bounds. Each signed mutation is submitted once.

The observer requires fresh agreeing membership/leader observations and complete
matching ingress projections before network controls. It first sees all three
eligible members, then exactly the two survivors while the voter count remains
three, on both VIP interfaces and both surviving observers. These bounded,
read-only convergence observations neither replay a mutation nor turn a failed
TCP/TLS attempt into readiness. Request/probe/phase budgets remain unchanged.

Each campaign completes **eight direct-node and four VIP controls**:
unknown/missing client on both management interfaces, with a healthy reader
before/after each rejection. Unknown clients require a new native policy event;
any application byte, server-verification failure or timeout fails the assertion.
There are **24 completed rejection/recovery controls across both runs**.
Neither run needs a read-observation retry. Final projections still agree on the
two survivors, and no further Raft term change is logged during the controls.

TCP metadata identifies ten new post-election VIP flows per run, including the
negative clients and healthy reads. None sends a SYN to the killed backend's
MAC. This is separate from retained old-flow traffic, which is not rehashed.
The service ELF hashes match between the two runs; credentials/resources and
kernel images differ because each uses freshly prepared disposable material.

| Run | Kernel SHA-256 | Result |
| --- | --- | --- |
| Full native 1 | `b1f7238dccc9a10f4318c1546c517f40f9fc4364fb55a0ebdf8c9a4ad43ffaca` | Pass: 8 direct + 4 VIP controls |
| Full native 2 | `d6bbcfa6a1e6ea37a3cb77824b0c10d5a8fdc9b5cc03a1015e79597d630084bb` | Pass: 8 direct + 4 VIP controls |

The [tracked evidence](2026-10-27-security-ingress-availability-evidence.json)
contains concise observations, receipts, projection rows, TCP metadata and
source/resource/ELF hashes, without temporary-log dependencies. Reproduce using
`scripts/build-catten-services.sh --embed` followed by
`python3 scripts/run-management-failover-test.py --firmware /path/to/ARM-UEFI.fd --trace-tcp --trace-limit 4096`.

## Persistence, tests and scope

The mandatory catalog-v19 `CATALOGJ` availability section preserves its complete
record. Pair-v4 `CRFTSTA4`/`CRFTLOG4` rejects earlier records before replay,
including old logs without an application snapshot. Missing/partial/extended
sections, bad flags, malformed/noncanonical sets and invalid generations reject
without changing live catalog state. Earlier stores are not migrated or reset.
Future deployed stores require an explicit interruption-tested migration.

All **257 service-library tests** pass, including three new tests using real
Raft vote/replication handlers, commitment-before-publication, follower agreement,
expiry/recovery, old-flow retention, receipt eviction, term/configuration fences,
canonical bounded codecs, generation exhaustion and rejection without live-state
mutation. Existing disk tests exercise interrupted I/O and reject retired
v1/v2/v3 pairs without writes. Six observer and three trace tests also pass.
The earlier sandboxed library attempt passed 251, with its three loopback TLS
fixtures denied by OS error 1; the authorized final run passes all 257. Both
service bundles rebuild and strict DNS/frouter/httpd/deployd Clippy passes on the
actual ARM/x86 service targets. Rustfmt, Python syntax and whitespace checks pass.
No fresh full host-suite, TLC, advisory scan or native x86 multi-node run is claimed.

The manual rebuild has 254 pages and no undefined references. Pages
123/124/133/154/155/201/202/203 are rendered and inspected; the existing 190
global overfull warnings remain, with no new count or visible defect on these
pages. Markdown, LaTeX and current persistence references advance together;
historical reports keep their original results and format tags.

This qualifies fresh connections **after committed convergence**, not instant
failure detection, established-session migration, accepted mutations interrupted
at leader loss, whole-cluster restart, node rejoin, missing-route recovery or
broad fault/pressure behavior. Receive-time observations are not peer
authentication or authenticated time. Protected enrollment/custody/anti-rollback,
live session retirement and the historical firmware intermittence remain
separate. No voter removal or physical cleanup authority is inferred, and no
numbered finding closes.
