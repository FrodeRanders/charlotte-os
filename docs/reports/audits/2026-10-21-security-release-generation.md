# Security audit: release component admission and observation

Date: 2026-10-21. Reviewed baseline: `7bf380f1`.

Release admission now returns the component deployment generations captured by
catalog apply. `release-apply` uses those generations when observing readiness.
Optimized regression tests also exposed capacity charges hidden inside
`debug_assert!`; those charges now run in both profiles. The
[tracked evidence](evidence/2026-10-21-security-release-generation.json) records
results, failure diagnostics and source/artifact hashes. Findings remain
**29 corrected / 6 partial / 3 open**; production trust stays disabled.

## What changed

A release generation counts revisions of a named release. Component deployment
generations advance independently, including through other releases and placement
reconciliation. The former scalar acknowledgement could not identify the
component records admitted by a particular request. Reading the catalog after
commit would leave a race with supersession and stale follower observations.

The shared `CRLADM01` codec carries the exact signed envelope SHA-256, release
generation and ordered component generations. Catalog apply prepares the complete
result before map mutation, using its existing serialized component plan. An
exact retry validates that every live component still has the original signed
descriptor bytes, then returns their current generations. A superseded or missing
component rejects without restoring older state. Snapshot restore needs no new
fields or format: the existing signed descriptors and generations supply that
check. Tests cover independent generations `[4,1]`, exact retry, snapshot restore
and rejection without mutation after a component advances.

DNS prepares an owned reply token and mapped output before local submission or
follower relay, reserves pending storage before publication, and retains them in
its existing reactor lists. The leader prepares bounded remote reply bytes before
Raft submission. Commit consumes the existing output; term change, redirection
and timeout consume the same terminal owner. The follower checks expected leader,
session and request ID, then checks the full receipt against its retained digest
and count. The shared relay counter now rejects exhaustion rather than wrapping.
Raw receive branches adopt the complete incoming transaction once before scalar
attachment guards. Clusterctl validates and forwards owned receipt memory;
deployd validates it and returns its complete lowercase hex in the sole HTTP
`admission` field.

Release IPC success now moves a receipt and reports its byte length. Release
relay tags are `0x2b/0x2c`; `0x19/0x1a` are retired. Negative errors remain exact
eight-byte scalar results on the relay. Earlier positive scalar successes,
truncated/extended frames, invalid generations and nonzero reserved bytes reject.
Operations admission validates the full catalog result before extracting its
existing scalar release generation. Signed input formats, persistent catalog/log
grammar, CLI arguments and runtime ABI are unchanged. The
[contract](../../reference/release-admission.md) specifies every field.

The host reserves observation storage before POST and borrows names from its
retained prepared component set. Descriptor and release apply share one
observation policy: older generations remain pending, matching ready generations
complete, and a newer generation fails as superseded. Malformed admission stops
before status queries; malformed HTTP-200 status leaves accepted work unconfirmed.
The client sends each notification once. These outcomes grant no replay or
rollback authority. The existing [C18 row](../../reference/cleanup-recovery.md)
records the owner scope; no kernel root, pin, physical completion proof, custody
registry or recovery family is added.

## Optimized capacity failure

The first optimized service test run failed
`release_reservations_are_charged_cumulatively`: actual `Ok([[1], [1]])`, expected
`Err(InsufficientCapacity)`; **146 passed / 1 failed**. The same control passed
in debug. Two calls to `reserve_node_capacity` appeared solely as
`debug_assert!` arguments, so optimized builds omitted their mutations. The
singleton and replica branches could reuse projected headroom within one release.
Both calls now execute in normal rejecting control flow. Additional controls
exercise fixed replicas and every-eligible-node placement, with rejecting tight
capacity and accepting roomy capacity in both profiles.

`CharlottePlacementControl` already charges promises in `Admit` and rejects
reserve overcommit. Its safe configuration passes with **145,800 distinct states**
and depth 25. The reserve and dwell negative configurations each produce their
expected invariant violation. The model remains unchanged; its
[conformance notes](../../tla/CONFORMANCE.md) record the concrete divergence and
correction. Receipt bytes and host observation remain outside those abstractions.
The 180 formal inputs retain their preceding hashes; this is a selected three-case
TLC run, without a new full-suite claim.

## Validation and limits

Host checks cover the shared codecs, catalog, operations admission and actual CLI.
The scripted release peer exercises **seven methods / 13 cases** in each host
profile; descriptor controls retain **nine methods / 13 cases**. Every case checks
POST count, and invalid admission performs no status query. Service tests run in
both profiles so debug assertions cannot hide admission behavior. The evidence
records **233 Rust tests per profile** (148 service, 10 launch, 75 signer),
formatting, strict Clippy, service builds and guest results. The staged bundles
contain 41 AArch64 and 37 x86_64 images.

The first AArch64 ingress run passed before the capacity correction; a separate
fresh run includes that correction. Guest evidence retains complete receipt,
matching ready status, kernel/service hashes and deferred-test summary. The
current four-LP AArch64 guest passes 20 deferred tests; the four-LP x86_64 guest
passes 18. Both complete signed upload, release admission, fetch, launch and
publication with matching receipt/readiness generations. These are leader-local
single-component runs. Startup
connection refusal and HTTP `-2`/`-23` rejections are retained separately from the
accepted request. The existing boot fixture retries whole CLI invocations during
startup; its success does not establish a general safe replay policy for uncertain
admission. Historical reports and the manual PDF are unchanged.

Initial validation also caught incomplete call sites, scalar-result expectations,
a receipt test with noncanonical unused array slots, and a new every-node fixture
with an invalid fixed replica count. Those development failures are separate from
the optimized production defect. TLC's first sandboxed launch failed to admit its
local RMI listener and was rerun with network permission. Relevant diagnostics
are retained inline in the evidence; passing repeats do not replace them.

Remaining work includes host end-to-end deadlines, bounded connect/DNS/write
phases, readiness notifications, ingress peer authentication and rollout policy.
The timeout still starts after notification, and very large timeout arithmetic
is not qualified. Live mapping/allocation/wait-admission rejection, follower
failure races, accepted multi-component guest supersession and wider destructor
contexts remain unqualified. The codec/catalog/CLI controls do not prove those
paths. Existing Raft/security audit gaps and SEC-15 remain open as previously
classified.
