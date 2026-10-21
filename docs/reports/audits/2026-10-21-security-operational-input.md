# Security audit: bounded operational file inputs

Date: 2026-10-21. Reviewed baseline: `da79205`.

Operational envelope and bundle inputs now use the existing bounded regular-file
reader before decoding, signing or notification. The [tracked evidence](evidence/2026-10-21-security-operational-input.json)
contains execution diagnostics, source/artifact hashes and baseline controls.
Finding counts stay **29 corrected / 6 partial / 3 open**. This host-input
correction leaves management fairness, trust and secret-state acceptance separate.

## Finding and correction

`operations-verify`, `operations-open`, `operations-bundle-verify` and
`operations-bundle-notify` used whole-file `fs::read` before bounded codec checks.
`operations-bundle-sign` likewise loaded its referenced release and envelopes
before final encoding checked their bounds and binding count. Decoders already
rejected oversized/malformed records; the gap was preceding file admission and
unbounded loading. Symlinks were followed and FIFO opens could block.

All these reads now share `read_bounded_regular`, with the codec's existing
maxima: **66,044 bytes** for COPSENC1, **1,048,064 bytes** for COPSBND2 and
**3,584 bytes** for its referenced release. The unchanged helper opens once,
checks that descriptor's regular-file metadata, reads at most maximum plus one
sentinel byte and rejects excess. Unix no-follow/nonblocking opens reject
final-component symlinks, directories and FIFOs. Bundle signing validates its
nonempty triple shape and **at most eight bindings** before opening referenced
release/envelope files. Its temporary vector of envelope paths is removed;
loading iterates the bounded triples directly.

File errors therefore precede decoding, plaintext creation, bundle output and
notification connection. Existing key reads still occur where required; the
count control concerns referenced release/envelope files. Verification, expiry,
context and HPKE checks are unchanged. Notify retains its existing decode and
transport contract; the receiver authenticates admission. CLI syntax, signed
formats, dependencies and kernel/runtime APIs do not change. Standalone
release/artifact commands and other tool inputs remain separate review work.

The existing [C18 row](../../reference/cleanup-recovery.md) records the local byte
owners and rejection scope. No capability/root cleanup, guard acquisition,
wait, remote teardown, registry/custody or physical/replay authority is added.
Living Markdown and manual LaTeX are updated; the PDF is untouched.

## Baseline controls

The actual built baseline CLI ran with the new integration test file and no
production-source edits. It uses a public fixed release-signing seed, public
development operational/recipient keys, one signed descriptor/release and an
opaque public encrypted payload. The signed release/envelope/bundle roundtrip
passes. No filesystem or decoder adapter substitutes for the tool.

Four expected rejection assertions fail in three separately retained runs:

- Two symlink tests observe **five accepted paths**: standalone envelope verify
  and open, bundle verify, and bundle signing through a release alias and an
  envelope alias. Each reports `exit status: 0`; baseline open actually creates
  plaintext and baseline signing creates outputs inside the owned fixture.
- With nine bindings and a missing referenced release, the baseline reports
  `read .../absent: No such file or directory`, showing a referenced read before
  count rejection. The codec already rejected excess bindings later. This does
  not demonstrate that an invalid bundle was accepted.
- An oversized zero-filled envelope reports `operational envelope signature
  verification failed`, where the control expects regular-file admission to
  reject. This first assertion stops the baseline size test; no result for its
  later cases is inferred. It establishes rejection stage, without a huge-file
  exhaustion or cryptographic acceptance claim.

No baseline FIFO or notification socket is exercised. Expected failures remain
separate from the passing baseline roundtrip and corrected repeats. Exact test
source, committed production-source hashes and relevant diagnostics are retained
in the evidence; baseline executables were not separately archived.

## Corrected validation

Six new actual-CLI tests cover the normal signed roundtrip, the five symlink
paths, oversized inputs for every changed command read, non-regular sources,
and early binding-count rejection. Missing referenced paths under nine bindings
now return `accepts at most 8 bindings`. Oversized verify/open, bundle verify,
bundle release/envelope signing and notify return file-admission diagnostics.
Directories and FIFOs reject across those paths. Rejected opens/signs create no
output. For oversized, directory, FIFO and symlink notification inputs, a local
listening socket observes no accepted connection after the child exits. That
single nonblocking observation is a test assertion, not a polling wait.

One unit test separately reads public opaque bytes at each exact file bound and
rejects one additional byte. It qualifies helper byte admission, without
claiming that arbitrary maximum-size bytes form valid signed records. Fixture
cleanup owns each unique mode-0700 directory only after exclusive creation;
fallible setup runs with that owner already installed. The opaque profile
qualifies the signed envelope path, not the S3 profile grammar.

The first sandboxed debug and release runs each fail **two tests** at
`TcpListener::bind("127.0.0.1:0")`: `PermissionDenied`, `Operation not permitted`
(OS error 1). Other new CLI cases pass. These are retained permission failures;
no source correction or skip conceals them. Reruns with approved loopback access
pass **43 runtime tests** in both debug and release: 24 unit, 5 key-encoding,
7 plaintext-profile-input, 1 public-key-input and 6 operational-input tests.
Strict all-target tool Clippy, workspace formatting and whitespace checks pass.
No other corrected build/test failure was observed.

The AArch64 service build and freshness/signature check pass with **41 signed
ELFs**, all identical to the preceding public-key evidence. Existing staged
x86 hashes also match; no fresh x86 build or guest run is claimed. Kernel and
userspace sources are unchanged. All **180 formal inputs** match preceding
evidence: 34 modules, 36 safe and 109 negative configurations plus the runner.
**0 fresh TLC cases** ran; these host file reads lie outside current projections.

## Limits

No-follow checks the final component; parent-directory security, hard links,
permissions/ACLs and concurrent content mutation remain operator policy. A
captured descriptor prevents path replacement from substituting the opened
file, without providing a content snapshot or slow-filesystem I/O deadline.
File-growth races, partial filesystem faults and non-Unix no-follow behavior
are unqualified. The existing public byte vector may grow within the bounded
read; no exact heap-capacity, aggregate admission or erasure guarantee is added.

These tests do not authenticate chosen keys, implement production custody,
qualify network flood resistance or establish successful notification delivery.
Other tool input families need separate review. Protected enrollment,
rollback-resistant state and recipient custody remain missing; production
trust stays disabled.
