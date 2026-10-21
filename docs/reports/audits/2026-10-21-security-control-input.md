# Security audit: bounded structured control inputs

Date: 2026-10-21. Reviewed baseline: `887dbd0`.

Deployment, release, shutdown and ingress-policy file inputs now share bounded
regular-file admission before further processing. The [tracked evidence](evidence/2026-10-21-security-control-input.json)
retains baseline controls, diagnostics and source/artifact hashes. Findings
remain **29 corrected / 6 partial / 3 open**. This host-input correction leaves
management fairness, trust and secret-state acceptance separate.

## Finding and correction

Eleven remaining structured read sites used whole-file `fs::read` before codec
bounds: deployment verify/notify/apply, release sign/verify/notify/apply, shutdown
verify/notify and ingress-policy verify/notify. Release signing loaded its
referenced descriptors before final encoding checked their count. Existing
codecs already rejected malformed/oversized records and excess descriptor
counts. The gap was preceding file loading; symlinks were followed and FIFO
opens could block.

Each site now uses the existing `read_bounded_regular`. Descriptor and release
limits are **3,584 bytes**, shutdown **144**, and ingress policy **1,184**, taken
from their current codec constants. The shared helper is unchanged: one opened
descriptor, regular-file metadata and size checks, a maximum-plus-one sentinel
read and rejection on excess. Unix no-follow/nonblocking opens reject
final-component symlinks, directories and FIFOs. Shutdown still requires its
exact 144-byte format at decode; file admission supplies the upper bound.

Release signing checks its existing **1..=16 descriptor** count before reading
references. Required key reading remains earlier; this changes referenced-file
admission order. Deployment apply retains its existing prepare-all-then-notify
order, with the per-descriptor file bound. Its independent input list receives
no new aggregate limit. Signatures, context/time checks, CLI arguments, wire
formats and receiver authorization are unchanged. Notify/apply do not gain
local authentication or a replay allowance from these checks.

The existing [C18 row](../../reference/cleanup-recovery.md) records the local
byte-owner scope. Rejection releases ordinary host bytes before output or
network publication. No capability/root cleanup, guard, wait, remote teardown,
registry/custody or physical/replay proof is added. Living Markdown and manual
LaTeX are updated; the PDF is untouched.

## Controls and validation

Baseline tests call the original committed command functions directly, with
only the new unit-test fixture and cases added. They use a public fixed signing
seed, a signed descriptor and production release/shutdown/ingress signers.
No file or decoder adapter replaces production operations. The evidence embeds
the exact test-only patch and production source hashes. These are function-level
controls; no new baseline CLI-dispatch result is inferred.

A baseline ordinary-file verification control passes for all four families.
Three separately retained rejection controls fail:

- All four verifier symlinks return `Ok(())`, and release signing through a
  descriptor alias also returns `Ok(())`, creating its output in the fixture.
- Each zero-filled maximum-plus-one file reaches its family's signature error,
  rather than file-admission rejection. The assertion records all four errors.
  It stops before the later release-sign oversize assertion; no baseline result
  for that assertion is inferred.
- Seventeen descriptor references to an absent file produce `read .../absent:
  No such file or directory` before count rejection. The codec already rejected
  excess counts later; this is no invalid-release acceptance claim.

No baseline FIFO, notify or apply socket is exercised. These small controls do
not demonstrate large-file/OOM exhaustion. Expected failures remain separate
from passing repeats; baseline executables are not separately archived.

Five new unit tests cover normal verification, the five symlink paths, early
size rejection, early descriptor-count rejection and notification/apply failure
before connection. Directory, FIFO, symlink and oversized inputs reject across
all changed command functions, including release-sign references. A local
listening socket has no queued connection after the synchronous calls return;
one nonblocking accept checks that result without a polling loop. Rejected
release signing creates no output. Each fixture owns its unique mode-0700
directory before fallible setup; the private fixture file uses the existing
restricted writer.

The existing raw-byte boundary unit test now also covers exact 144/1,184-byte
reads and one-byte excess, alongside the other bounds. Those public opaque
bytes qualify helper file admission; they are not valid maximum-size signed
records. Existing integration targets still invoke the real operational CLI.
No new positive notification delivery or applied-rollout result is claimed.

Debug and optimized tool suites pass **48 runtime tests**: 29 unit, 5 key-file
encoding, 6 operational-input, 7 plaintext-profile-input and 1 public-key-input.
Both full runs use approved loopback access for the existing/new no-connection
checks. Strict all-target tool Clippy, workspace formatting and whitespace
checks pass. No corrected build/test failure was observed in this batch.

The AArch64 build and freshness/signature check pass with **41 signed ELFs**,
all identical to preceding operational-input evidence. Existing staged x86
hashes also match; no fresh x86 build or guest run is claimed. Kernel/userspace
sources are unchanged. All **180 formal inputs** match preceding evidence:
34 modules, 36 safe and 109 negative configurations plus the runner. **0 fresh
TLC cases** ran; these host file reads lie outside current projections.

## Limits and remaining inputs

Five production whole-file read sites remain: four raw ELF reads in sign,
verify and deployment-variant preparation, plus the arbitrary-file `sha256`
command. They need their own policy; this batch introduces no universal tool
size bound. Apply's aggregate descriptor list, argv parsing, output-write
atomicity and wider heap admission also remain separate.

No-follow checks the final component. Parent directories, hard links,
permissions/ACLs and concurrent file content remain operator policy. Descriptor
capture supplies no content snapshot or slow-filesystem deadline. Partial I/O,
file-growth races and non-Unix no-follow behavior are unqualified. Public byte
vectors may grow within their bounded reads; there is no exact heap-capacity
or erasure guarantee.

File admission does not authenticate chosen keys, prove network flood fairness
or implement production custody. Protected enrollment, rollback-resistant state
and recipient custody remain missing; production trust stays disabled.
