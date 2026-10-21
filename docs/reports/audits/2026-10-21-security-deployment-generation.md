# Security audit: binding descriptor rollout observation to admission

Date: 2026-10-21. Reviewed baseline: `e0e3f615`.

`deployment-apply` now retains each admitted deployment generation and requires
matching readiness. The [tracked evidence](evidence/2026-10-21-security-deployment-generation.json)
preserves baseline CLI failures, corrected results and source/artifact hashes.
Findings remain **29 corrected / 6 partial / 3 open**. The release client's
component-generation binding remains open; production trust stays disabled.

## Finding and correction

`deployd` returns `{"generation":N}` after descriptor admission. Its rollout
response includes `deployment_generation`, state and replica counts. The host
descriptor-apply client discarded the acknowledgement generation and accepted
any HTTP 200 body containing `"state":"ready"`. A stale follower observation
or a superseding rollout could therefore complete the wrong operation. Duplicate,
trailing or inconsistent status data could also satisfy that substring test.
Earlier living documentation overstated the client's generation checks.

The private `rollout` module decodes the current scalar records. Admission
requires one positive generation within the kernel's positive signed-result
range. Status requires all six current fields exactly once, with valid integer
syntax and ranges, known states and consistent replica counts. Ready requires
a nonempty fully ready set and positive service generation. Field order and
JSON whitespace are accepted. Unknown, missing, duplicate, nested or extended
records reject. This is a codec for current deployd records, not a general JSON
parser or an older-format compatibility path.

Descriptor notification returns an `AdmittedDeployment` containing the decoded
generation and existing body text. Standalone `deployment-notify` retains its
successful output but rejects an invalid acknowledgement as an uncertain
outcome. Descriptor apply reserves pending storage before its first submission;
name/generation pairs borrow names from the complete retained prepared descriptor
set. Every notification is submitted once. Matching ready observations complete;
older observations remain pending. A newer generation reports supersession.
Malformed HTTP-200 status stops with the admitted generation unconfirmed.

Malformed acknowledgement stops before status observation or later descriptor
submission. This and earlier descriptors may already be committed. Closing host
sockets and releasing public byte/string/vector fields does not roll back remote
work or authorize replay. The existing [C18 row](../../reference/cleanup-recovery.md)
records the owner scope; no capability/root cleanup, guards, waits, registry,
recovery family or physical completion proof is added.

Server encoding, routes, kernel/service code, signature bytes, descriptor formats,
runtime ABI and CLI arguments are unchanged. Living Markdown and manual LaTeX
now distinguish descriptor apply, current-name status and the release-client
gap. Historical reports and the manual PDF are untouched.

## Baseline and corrected controls

The new [CLI test script](../../../scripts/tests/test-deployment-generation.py)
uses public development keys and a minimal host ELF section-table fixture in
owned mode-0700 directories. It signs real descriptors and invokes the actual
CLI against a scripted loopback peer. The peer supplies current HTTP/1.1 and
JSON shapes; these are client controls, without a live cluster or guest run.
Every scenario checks the submitted descriptor length and POST count.

The seven-method baseline matrix has **11 cases: nine expected failures and two
passes**. It demonstrates false success on older and newer ready generations,
early completion before the expected generation arrives, three invalid
acknowledgements and three malformed/inconsistent ready records. Exact-generation
success and matching pending-to-ready progress pass. The baseline executable
hash matches preceding evidence. Its source and exact baseline test snapshot
are embedded; no executable was separately archived.

An initial harness mistakenly used Python's default HTTP/1.0 status line. The
client rejected it before generation observation, invalidating those results
as generation evidence. That run is preserved separately. Setting the peer to
HTTP/1.1 required no production change; only the subsequent baseline run supports
the nine generation/record failures above.

All **13 final cases in nine methods pass against debug and release CLIs**.
Two added controls cover independent generations for two components and stopping
a three-component batch after an uncertain second admission. The latter observes
two POSTs, no status request and no third submission. Other controls observe
one POST each, including timeout, supersession and malformed responses. There
is no notification replay. Six new Rust tests cover codec fields, counters,
whitespace/order, ambiguity and ready/pending distinctions.

Complete debug and release signer suites each pass **75 runtime tests**: 47 unit,
4 control-output, 5 file-hash, 5 key-encoding, 6 operational-input,
7 plaintext-profile-input and 1 public-key-input. Strict all-target Clippy,
workspace formatting, Python parsing and whitespace checks pass. Loopback tests
use approved access; no corrected build or test failure was observed.

The AArch64 build/signing and freshness checks pass for **41 ELFs**, matching
preceding hashes. All **37 existing x86 ELFs** match too; there is no fresh x86
build or guest run. Kernel/userspace sources, package manifests and lockfile are
unchanged. All **180 formal inputs** match prior evidence; **0 fresh TLC cases**
ran. Host HTTP parsing and client observation are outside current projections.

## Remaining gaps

`release-apply` still checks component readiness by name. Its acknowledgement
contains only the release generation; catalog source assigns component deployment
generations independently. That scalar cannot safely stand in for the missing
component-generation map. Concurrent supersession can still satisfy that client
with another rollout's readiness. The standalone `deployment-status` command
also observes the current name rather than an admitted operation. Server-side
generation-safe replica counting does not repair these client distinctions.

Operation-wide deadlines remain separate work. Apply still starts its readiness
budget after notification, individual connection/response bounds can exceed the
remaining operation budget, numeric deadline overflow is not addressed, and
status observation retains its one-second polling. No missing readiness surface
is presented as event-driven. These controls use two-second rollout budgets
and qualify generation decisions, not strict wall-clock limits.

The selected ingress peer and HTTP observations still rely on operator network
policy. This adds no authenticated receipt, TLS or trusted freshness proof.
A matching observation is a snapshot, without a guarantee of lasting readiness.
Multi-component descriptor admission remains independent and can leave a
committed prefix; this batch adds no atomic admission, rollback or recovery.

The private-key input review found no missing current-capacity erasure owner:
it installs `Zeroizing<Vec<u8>>` before reading, and pinned zeroize 1.9.0 erases
the vector's entire capacity. No private-input rewrite was made. Prior
reallocations, compiler/library copies, physical erasure and wider SEC-15
lifetimes remain separate. Protected enrollment, rollback-resistant state and
recipient custody remain missing.
