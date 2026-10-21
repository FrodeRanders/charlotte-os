# Security audit: exclusive structured control outputs

Date: 2026-10-21. Reviewed baseline: `c6ab458`.

Deployment, release and shutdown signing now require fresh output paths. The
[tracked evidence](evidence/2026-10-21-security-control-output.json) preserves
baseline CLI failures, corrected results and source/artifact hashes. Findings
remain **29 corrected / 6 partial / 3 open**; production trust stays disabled.

## Finding and correction

The three commands published signed bytes with `fs::write(output, &bytes)`.
That truncated existing regular files and followed symlinks. A hard link could
also direct the write into an existing file. This differed from the exclusive
creation already used for key, operational, trust-policy and ingress outputs.

All three now borrow their prepared public bytes through `write_new_file`.
Its `create_new(true)` open rejects an existing final path before any write.
There is no overwrite option or alternate publication branch. Arguments,
signature construction, record formats and successful output bytes are unchanged.
Public output mode uses the shared 0644 request, subject to the host's umask.
ELF signing keeps its separate captured-file rewrite contract.

Creation rejection releases the existing prepared byte owners without an output
descriptor. Ordinary success closes the local file. No new Charlotte capability,
root operation, wait, registry or recovery custody enters these host paths.
The existing [C18 row](../../reference/cleanup-recovery.md) records this scope.

The ARM and x86 boot runners allocate a fresh private directory beneath their
deployment fixture parent. Their worker result/log paths follow that directory.
The ARM shutdown fixture also gets a fresh directory. Local deployment fixture
generation puts its six signed parts in a new private directory on every run.
These directories remain available for inspection; the assembled local fixture
bundle retains its existing replacement policy. The security-fixture generator
already requires an empty directory and needs no change. Caller review covers
the tracked repository; exported SDK consumers must choose fresh output names.

Living Markdown and manual LaTeX describe the new output requirement. The manual
PDF is untouched.

## Baseline and corrected controls

Four new integration tests invoke the actual CLI with public seed-7 fixtures
inside exclusively created mode-0700 directories. The fixture owner is established
before fallible setup and disposes only its own directory. A minimal ELF supplies
host section-table metadata; these are not executable guest tests.

The unchanged baseline production code yields **three expected test failures**:
each of the regular-file, symlink and hard-link tests exercises all three signing
commands. All **nine** invocations return success and replace the marker bytes.
Diagnostics preserve command status, preservation failure and output. The fresh
output control passes and verifies all three signatures. The evidence identifies
the exact baseline revision and test source; no baseline executable was archived.

The corrected tests pass. Existing output bytes, target bytes, output inode and
file type survive rejection, and rejected commands emit no success output.
The fresh-output sizes and SHA-256 digests match the baseline exactly:

| Record | Bytes | SHA-256 |
| --- | ---: | --- |
| Deployment | 219 | `5487a2a60f136d3015a666292773e2b4af78f0b4c1fa8d215b3cb0603c4df526` |
| Release | 347 | `a47d89b13d8ff2cb8d7374cd1761c48245fb22428a01e6d25809a745b34ee191` |
| Shutdown | 144 | `ac6fd11c81f96d23e590aa34185eb586cbbdb460871c1ae743442569824518d0` |

The complete debug and release suites each pass **69 runtime tests**: 41 unit,
4 control-output, 5 file-hash, 5 key-encoding, 6 operational-input,
7 plaintext-profile-input and 1 public-key-input. Existing loopback checks run
with approved access. Strict all-target Clippy, workspace formatting and shell
syntax checks pass.

The actual local deployment fixture script runs twice against the same output
bundle. Both runs pass, produce identical six-record bundles and retain distinct
mode-0700 part directories. All retained part signatures verify. Host controls
also exercise the runners' directory-allocation statements twice. They qualify
host preparation only; no fresh guest, Docker ingress or shutdown run is claimed.
The first temporary bundle-check harness compared lexical filename order with
the fixture's explicit node order and failed. Correcting that comparison required
no production change; the failed check and corrected harness are kept separately
in the evidence.

The AArch64 bundle build/signing and freshness checks pass for **41 ELFs**.
All hashes match the preceding artifact-edit evidence. The existing **37 x86
ELFs** match too; there is no fresh x86 build. Kernel/userspace sources and the
lockfile are unchanged. All **180 formal inputs** match prior evidence;
**0 fresh TLC cases** ran. These host file operations are outside current models.

## Remaining work

Exclusive creation protects admission to an absent final name. Parent-directory
integrity, namespace changes after creation, concurrent inode writers and file
permissions remain operator concerns. It supplies no atomic or durable record
publication, I/O deadline, fsync, partial-write rollback or crash recovery.

The shared writer still closes its descriptor and unlinks by pathname after a
write error. Key-pair generation also removes its private pathname when public
output creation fails. Those cleanup paths need ownership review against name
replacement; this batch does not claim that exclusive creation qualifies them.
No partial-I/O fault or concurrent replacement during cleanup was injected.
The fixture bundle assembler's replacement policy is also outside this fix.

Protected enrollment, rollback-resistant state and recipient custody remain
missing. This host correction does not enable production trust or close SEC-15.
