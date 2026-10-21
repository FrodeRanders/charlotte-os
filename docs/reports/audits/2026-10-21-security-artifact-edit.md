# Security audit: rewriting the opened ELF file

Date: 2026-10-21. Reviewed baseline: `58cf025`.

ELF signing now retains the file that supplied its prepared image through
publication. The [tracked evidence](evidence/2026-10-21-security-artifact-edit.json)
records the deterministic controls, diagnostics and source/artifact hashes.
Findings remain **29 corrected / 6 partial / 3 open**. This corrects host write
redirection; durable atomic publication and broader trust work remain separate.

## Finding and correction

`elf-sign` read through a bounded no-follow descriptor, dropped that descriptor,
then called `fs::write(path, &image)`. Replacing the name between those phases
could redirect writing to a different regular file or symlink target. Removing
the name could cause publication to create a new file. Read admission alone
could not bind the later write.

The private `ArtifactEdit` now retains a read/write `File` and prepared image.
Shared `open_regular_options` keeps the existing no-follow/nonblocking descriptor
and metadata checks; bounded reading borrows that same file through
`read_bounded_contents`. Other readers retain their read-only wrapper. Signing
borrows the image for unchanged metadata/digest/signature work, then consumes
the edit. `commit(self)` checks the 4 MiB bound before writing, rewinds the file,
writes all prepared bytes and sets its final length. It never looks up, creates,
removes or reopens the output path. Neither opening nor ordinary Drop truncates
or publishes the buffered image. The signing digest is prepared before commit.

A renamed original remains the selected file; a replacement regular file or
symlink target stays untouched. An unlinked original can still be rewritten
through the open descriptor, without recreating its name. Success therefore
refers to the opened file. Human output now says `signed artifact opened at
<path>`; arguments, metadata/signature bytes and verification remain unchanged.
File modes continue through ordinary OS semantics; no chmod policy is added.

Commit consumes the owner on success or error. A write/set-length error may
leave partial or uncertain content; it supplies no retry owner or rollback.
Default field Drop closes the host descriptor and releases the public image.
This is local host ownership, without Charlotte capability/root cleanup,
guards, waits, remote teardown, a new registry/custody boundary or physical
completion proof. The existing [C18 row](../../reference/cleanup-recovery.md)
records the complete owner and remaining limits. Living Markdown and manual
LaTeX are updated; the PDF is untouched.

## Baseline and corrected controls

The baseline uses an explicit **phase adapter** of the original bounded read
and `fs::write` publication. It copies the original path into the test owner
for a stable lifetime and replaces cryptographic work with public byte edits.
This allows deterministic filesystem replacement between read and publication.
It is not an injected race inside the actual baseline CLI. Original command
source, exact adapter/test patch and relevant diagnostics are embedded in the
evidence; no baseline executable was separately archived.

Five baseline controls yield **three expected failures and two passes**:

- A replacement symlink redirects the write into an unrelated public fixture.
- A replacement regular file is overwritten after the original was renamed.
- Removing the original path causes publication to recreate it.
- Ordinary same-file write preserves inode/mode and removes trailing bytes.
- Dropping an uncommitted byte edit leaves its source unchanged.

All fixtures are confined to uniquely owned mode-0700 test directories. No
external files or real keys are involved. The failure bytes show the public
`signed public bytes` marker replacing the unrelated/replacement content.
The tests dispose only their owned directories. The baseline failures are kept
separate from passing normal/abandonment controls and corrected repeats.

The same five phase controls now exercise production `ArtifactEdit` directly
and pass. They verify that the renamed captured inode receives prepared bytes,
replacements stay unchanged and unlink does not recreate the path. Two more
controls pass: oversize edit rejection before writing, and first-write failure
without changing the source. The latter deliberately constructs a private owner
with a read-only descriptor to inject rejection; the real constructor requests
read/write access. It qualifies early write failure, without claiming an actual
partial-write, disk-full or power-loss result.

Existing ELF tests still sign/verify and check two variant digests. Both complete
tool suites pass **65 runtime tests**: 41 unit, 5 file-hash, 5 key-encoding,
6 operational-input, 7 plaintext-profile-input and 1 public-key-input. Full runs
use approved loopback access for existing controls. Strict all-target Clippy,
workspace formatting and whitespace checks pass. No corrected build/test
failure was observed.

The AArch64 service build exercises the actual signing CLI and passes with
**41 signed ELFs**; freshness/signature verification also passes. All ELF hashes
match preceding artifact-input evidence. Existing staged x86 hashes match too;
no fresh x86 build or guest run is claimed. Kernel/userspace sources and lockfile
are unchanged. All **180 formal inputs** match prior evidence: 34 modules,
36 safe and 109 negative configurations plus the runner. **0 fresh TLC cases**
ran; these host file operations lie outside current projections.

## Limits

Keeping a descriptor binds the selected file; it does not freeze that file's
content or the namespace. Other writers to the same inode and its hard links
remain possible. Parent-directory integrity, initial path selection and file
permissions/ACLs stay with operator policy. A successful rewrite does not prove
that the original pathname still names the selected file. The rename/unlink
controls are qualified on the Unix host; other platform semantics are untested.

Writes are in place. There is no temp-file replacement, sync/fsync, locking,
crash journal, automatic rollback or atomic/durable publication claim. Partial
write/truncate errors, concurrent inode writes, filesystem faults and power
loss remain unqualified. Other output writers and their cleanup policies remain
separate work. No source/OS/compiler/physical erasure proof is added.

Production enrollment, rollback-resistant state and recipient custody remain
missing; production trust stays disabled.
