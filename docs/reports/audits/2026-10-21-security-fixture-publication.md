# Security audit: publishing the local deployment fixture bundle

Date: 2026-10-21. Reviewed baseline: `12d6bed2`.

The local deployment fixture now prepares a complete staging bundle before
replacing its named output. The [tracked evidence](evidence/2026-10-21-security-fixture-publication.json)
preserves baseline diagnostics, corrected results and source/artifact hashes.
Findings remain **29 corrected / 6 partial / 3 open**; production trust remains
disabled.

## Finding and correction

`scripts/prepare-local-deployment-fixture.sh` already prepared its six signed
descriptors in a fresh private directory. Its final Python block then used
`output.write_bytes` to publish their length-prefixed bundle. That truncated an
existing file and followed a symlink. Hard-link targets and readers holding an
open descriptor observed the changed bytes. A partial write could damage a
previous bundle or expose a partial new one under the final name.

The block now opens `bundle.pending` exclusively inside the same private run
directory. It writes the unchanged assembled bytes and closes that file before
calling `os.replace(staging, output)`. The normal helper keeps both paths on the
same filesystem. Successful publication replaces the named file, including a
symlink or hard link, while leaving the linked target untouched. Earlier open
readers retain the old file. There is no in-place fallback.

Staging-write or close failure occurs before replacement, leaving the previous
named output alone. Failed staging remains in the run directory for inspection.
An occupied staging name rejects without truncation. The tested replacement
rejection retains the complete staged bundle and existing destination directory.
Errors stop the script; there is no cleanup unlink or automatic retry.

This is the fixture bundle's intentional named-replacement contract. The
signer's exclusive creation and captured-ELF rewrite contracts stay unchanged.
The six-record grammar, signed descriptors, argument syntax and kernel cache
admission are unchanged. Successful publication creates a new inode with the
current creation/umask policy; previous destination metadata is not preserved.

The Python file context owns local close through success/error. Its filesystem
artifacts are inspection state, without retry ownership, physical completion or
recovery custody. No Charlotte capability/root operation, guard, wait, registry
or owner family is added. The existing [C18 row](../../reference/cleanup-recovery.md)
records this scope. Living Markdown and manual LaTeX are updated; the PDF is
untouched.

## Baseline and corrected controls

The new [publication script](../../../scripts/tests/test-local-fixture-publication.py)
extracts and executes the actual Python block from the production shell script.
It supplies public opaque descriptor bytes in an owned mode-0700 directory;
these phase tests qualify filesystem publication, without claiming signature
validation. The full helper validation below uses actual signed descriptors.

Eight baseline tests yield **six expected assertion failures, one expected
error and one pass**:

- Symlink and hard-link targets are overwritten.
- An earlier open reader sees replacement bytes through its original file.
- A real 17-byte file-size-limit failure damages previous output.
- The same failure leaves partial output under a previously absent final name.
- An occupied staging name is ignored and publication proceeds.
- A directory destination rejects, but the expected complete staging file is
  absent; reading it raises `FileNotFoundError`. This is evidence of the missing
  staging phase, separate from the six assertion failures.
- Fresh and repeated publication preserve the expected assembled bytes.

The baseline block and unchanged test source are identified in the evidence.
No production hook or substituted assembler is used. The child-only size limit
ignores `SIGXFSZ` so real file writing returns an error. No concurrent pathname
replacement, power-loss event or delayed-close fault was injected.

All **eight corrected tests pass**. The existing bundle's bytes and inode survive
staging-write failure; an absent final name stays absent. Both failures retain
17-byte staging files. Replacement rejection retains the complete staging
bytes. Existing-reader and link-target controls pass. The 204-byte public test
bundle matches the baseline SHA-256:
`b2fd9d77e308396276df0f279dbf5b357cdd1107b37f8fecda04401e7ccaf16a`.

The actual full fixture helper also runs twice against the same bundle path.
It produces identical **1,896-byte** bundles, retains distinct mode-0700 part
directories and verifies all twelve signed descriptor files. Its bundle hash
matches the [earlier control-output evidence](evidence/2026-10-21-security-control-output.json):
`d7102e6be81a76490d72a602b8c15e99ed18924142b17c84241851f416acbd82`.
The evidence includes the full verification harness and distinguishes its
runner-allocation statement checks from full helper execution. Shell syntax,
both Python ASTs, changed/new text whitespace and local documentation links pass.

No Rust source or package input changes. The preceding 69-test debug/release
signer results, 17 failure subcases per binary and strict Clippy results remain
historical evidence; they are not counted as fresh runs here. The AArch64
build/signing and freshness checks pass for **41 ELFs**, all matching preceding
evidence. The **37 existing x86 ELFs** also match; no fresh x86 build or guest
run is claimed. All **180 formal inputs** match and **0 fresh TLC cases** ran.
These host file operations remain outside current model projections.

## Limits

The operator still controls output selection and parent-directory integrity.
Concurrent namespace changes, mount changes and inode writers are outside this
qualification. Publication intentionally replaces the current named entry;
it does not promise to preserve its prior metadata or prevent a later writer.

There is no fsync, crash journal or durable-publication proof. Local rejection
controls do not qualify ambiguous filesystem errors, network filesystems,
power loss, non-Unix behavior or recovery after an interrupted replacement.
Retained staging does not authorize replay; the script provides no resume API.
Disk erasure, other host output contracts and production trust provisioning
remain separate work.
