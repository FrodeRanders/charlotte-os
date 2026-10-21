# Security audit: retaining host output after failure

Date: 2026-10-21. Reviewed baseline: `9f98ae5c`.

The signer now closes failed output descriptors without deleting by pathname.
All three key generators report incomplete generation and retain private output
when public output fails. The [tracked evidence](evidence/2026-10-21-security-output-retention.json)
preserves baseline failures, diagnostics and source/artifact hashes. Findings
remain **29 corrected / 6 partial / 3 open**; production trust stays disabled.

## Finding and correction

The shared exclusive writer closed its descriptor after a write error, then
removed the supplied pathname. Each key generator also removed the private
pathname after public output failed. A name can change after creation; these
four deletion sites could remove a replacement file. Comparing inode metadata
before unlinking would still leave a check-to-unlink window.

`write_new_file` now returns the write error with an explicit partial-output
diagnostic. Ordinary `File` Drop closes the captured descriptor; no pathname
deletion follows. The three generators share `write_new_keypair`, borrowing
their original key owners through the existing per-file erasing encoders.
Private text still drops before public encoding. A private-output failure
returns immediately. A public-output failure identifies incomplete generation
and the private output's original path, leaving the completed private file alone.
There is no rollback branch or implicit retry. Success messages, argument syntax,
key encodings, signed formats and private-mode policy are unchanged.

Created files can remain empty, partial or complete. This includes decrypted
operational-profile output through the same shared writer. Operators must inspect
the secured output directory and choose fresh paths for another operation.
A failed generation supplies no completed key-pair result. Diagnostics name
original paths; they cannot establish those paths' current bindings. External
rename/unlink can still move or remove a file.

This is local host file/error policy. Existing byte and key owners retain their
erasing Drop; no capability/root cleanup, guards, waits, remote teardown, registry
or recovery custody is added. Retained files are not retry owners or physical
completion proofs. The existing [C18 row](../../reference/cleanup-recovery.md)
records the scope. Living Markdown and manual LaTeX are updated; the PDF is
untouched. Boot fixture scripts keep their previously established fresh-directory
policy and need no changes.

## Baseline and corrected controls

The new [CLI test script](../../../scripts/tests/test-signer-output-failure.py)
runs five methods with **17 subcases**, confined to owned mode-0700 temporary
directories. It logs no key contents. Generation outputs are disposable test
keys; shutdown signing uses the public development fixture.

The unchanged release baseline produces **14 expected failing subcases**:

- Six private-write failures: three generators, each limited to zero or 17 bytes.
- Two public shutdown-output failures, with the same two size limits.
- Three occupied public-output paths after successful private writes.
- Three attempts using one pathname for both key outputs.

Every failure reflects deletion of the created output. Three private-creation
rejection controls pass, preserving existing input and leaving public output
absent. The baseline binary hash matches preceding tracked evidence; its source
and the new test script are identified by hash. The binary was not separately
archived.

The file-size controls apply `RLIMIT_FSIZE` only to a CLI child and ignore its
`SIGXFSZ`, allowing the real host write to return `File too large`. This is actual
CLI I/O, without a production hook or simulated writer. Corrected runs observe
zero-byte and 17-byte retained files, error status and no success output.
Private output retains mode 0600 under the test host's umask. Other controls
observe complete private lengths, unchanged occupied public files and explicit
incomplete-pair diagnostics. All **17 subcases pass against both debug and
release binaries**. No concurrent name-replacement race was injected; deletion
redirection is removed by eliminating every production deletion site.

Both complete Rust suites pass **69 runtime tests**. Existing unit and CLI key
tests now require retained, readable private output after public creation
rejection; they retain their success, erasing-buffer and output-privacy checks.
Strict all-target Clippy, workspace formatting, Python parsing and whitespace
checks pass. An initial passing debug run exposed a test-only `fs` import warning;
moving that import into the test module removed it before final validation.

The AArch64 build/signing and freshness checks pass for **41 ELFs**. Their hashes
match preceding control-output evidence. All **37 existing x86 ELFs** match too;
no fresh x86 build or guest run is claimed. Kernel/userspace sources and the
lockfile are unchanged. All **180 formal inputs** match prior evidence;
**0 fresh TLC cases** ran. These host filesystem operations lie outside current
model projections.

## Limits

Retention deliberately leaves filesystem material for operator inspection.
Private-mode creation and in-memory erasure do not erase disk blocks, OS buffers,
backups, crash dumps or copies. File permissions/ACLs, parent-directory integrity,
concurrent writers and namespace continuity retain their existing limits.

The tests qualify first-write rejection and short-prefix failures on this Unix
host. They do not qualify late partial writes for every command, delayed close
errors, filesystem faults, power loss, non-Unix behavior or concurrent cleanup
replacement. There is no atomic/durable publication, fsync, journal, automatic
recovery, secure deletion or resumed-generation API. The assembled fixture
bundle's separate replacement policy is unchanged.

Protected enrollment, rollback-resistant state and recipient custody remain
missing; SEC-15 stays partial and production trust remains disabled.
