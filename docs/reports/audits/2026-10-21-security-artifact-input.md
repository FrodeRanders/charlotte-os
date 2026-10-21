# Security audit: artifact input bounds and streaming hash

Date: 2026-10-21. Reviewed baseline: `c79b683`.

The host signer now applies Charlotte's existing artifact bound to ELF input
and note growth, admits variant count before image reads, and streams arbitrary
file hashing. The [tracked evidence](evidence/2026-10-21-security-artifact-input.json)
retains controls, failure diagnostics and source/artifact hashes. Overall
findings remain **29 corrected / 6 partial / 3 open**; this narrows host input
and scratch lifetimes without closing broader trust or secret-state work.

## Finding and correction

Four ELF read sites used whole-file `fs::read`: sign, verify, and deployment's
primary/extra images. The shared launch ABI already limits artifacts to
**4,194,304 bytes (4 MiB)**; host reads did not use it. Adding a signature note
could also turn a limit-sized input into an oversized output. Variant count
reached the descriptor encoder after reading images. The generic `sha256`
command loaded an entire file despite needing only incremental hash state.

All four ELF inputs now use `read_bounded_regular` with
`MAX_ARTIFACT_ELF_SIZE`. `add_note_section` computes the complete checked output
length, including note, names, alignment and section table, and rejects excess
before allocating/copying that output or writing the source path. It reserves
the accepted output length once. Existing-note signing keeps its bounded image.
Deployment checks **at most eight variants including the primary** before any
image read. Existing format/signature/target checks still follow admission.

`open_regular` factors the existing descriptor open/type checks out of the
bounded reader. ELF, structured public input and hash commands borrow the same
policy: one open and metadata inspection of that descriptor, with Unix
`O_NOFOLLOW | O_NONBLOCK`. Final-component symlinks, directories and FIFOs reject.
Private-key and plaintext-profile readers retain their existing specialized
policies. The bounded reader retains maximum-plus-one sentinel rejection.

The new `file_hash::digest` owns one initialized **16 KiB zeroizing array** and
the existing erasing SHA state before reading. It hashes only reported bytes,
retries Interrupted in the same owner, rejects impossible counts, and returns
no digest on read error. EOF returns the final digest. `sha256` opens a regular
file and streams to EOF; it has no artifact-size cap. Empty-file output and
lowercase digest plus LF are unchanged. There is no production `fs::read` left
in the signer. Bounded image/record readers still retain their byte vectors.

Command syntax, cryptography, signed bytes, runtime ABI and receiver authority
are unchanged. The existing [C18 row](../../reference/cleanup-recovery.md) and
[secret-state inventory](../../reference/secret-state-lifetimes.md) record the
local owners. Ordinary Drop adds no Charlotte capability/root cleanup, guard,
wait, registry/custody or physical/replay authority. Manual LaTeX and living
references are updated; the PDF is untouched.

## Baseline evidence

Function-level ELF controls ran against committed production code with only
new tests added. The section-table fixture supports host metadata/signature
checks; it has no runnable program segments and is not an execution test.
Normal sign/verify and two-variant descriptor generation pass.

Four ELF rejection controls fail separately:

- All four alias paths return `Ok(())`: ELF sign/verify and deployment primary/
  variant input. The assertion stops before later FIFO/directory checks; those
  baseline paths were not exercised.
- Each 4 MiB-plus-one input reaches ELF/architecture/verification rejection,
  rather than file admission. The recorded errors are `input is not a
  little-endian ELF64 image`, `verification failed: Unsigned`, `unsupported ELF
  architecture` and `unsupported variant ELF architecture`.
- Signing a 4 MiB unsigned section fixture succeeds and replaces it with
  **4,194,752 bytes**, 448 beyond the shared launch maximum.
- Nine requested variants with an absent primary reach `read .../absent:
  No such file or directory` before count rejection. The encoder already
  rejected excess later; no invalid-descriptor acceptance is inferred.

Actual baseline CLI hash controls preserve exact empty and 4 MiB-plus-one
Python-reference digests. The alias-rejection control fails because hashing a
symlink succeeds and prints its digest; it stops before non-regular checks.
No new baseline memory-exhaustion or allocator control was run. Whole-file
allocation is a source observation of the old `fs::read` path. Baseline test
source/patches and relevant diagnostics are embedded in the evidence; baseline
executables were not separately archived.

The first ELF baseline invocation ran from the repository directory with its
pinned compiler/build-std configuration and failed before tests with **E0152,
duplicate `core` lang item `sized`**. The same normal-file test passes from the
host directory `/private/tmp`. This invocation failure is retained separately
from expected rejection failures and successful repeats. Baseline hash-target
runs also record one unused-import warning; that test-only import was removed
before strict corrected checks.

## Corrected validation

Five new ELF unit tests pass. They cover normal signing/verification and two
variant digests, all four input bounds, alias/non-regular rejection, note-growth
rejection preserving exact source bytes, and count rejection before absent
image reads. The rejected paths create no deployment output.

Five new hash tests pass: ten independent Python `hashlib` vectors across SHA
block and 16 KiB chunk boundaries; fragmented generated reads with **zero host
heap allocations and zero growth** observed inside the successful hash loop;
Interrupted with a dirty destination; partial error and impossible count without
a returned digest; and real CLI normal/alias/non-regular controls. Python fixture
lengths include empty and **4 MiB plus one**, confirming the hash command's size
policy. The [Python fixture](../../../tools/cluster-sign/tests/fixtures/file-hash-vectors.txt)
records the generator expression and complete expected bytes.

The allocation observer counts calls while the hash helper runs over a generated
reader; it does not measure file opening, OS buffers or output formatting. No
freed memory, padding or uninitialized capacity is read. The scratch owner's
erasing Drop is a pinned type/source contract here, with a trait control; this
batch adds no dynamic retained-storage erasure probe. Dirty/fault readers qualify
the helper contract, without an injected partial filesystem failure.

Debug and optimized tool suites pass **58 runtime tests**: 34 unit, 5 file-hash,
5 key-encoding, 6 operational-input, 7 plaintext-profile-input and 1 public-key
input. Both full runs use approved loopback access for existing controls. Strict
all-target Clippy, workspace formatting and whitespace checks pass. No corrected
build/test failure was observed.

The AArch64 build and freshness/signature check pass with **41 signed ELFs**,
all identical to preceding control-input evidence. Existing staged x86 hashes
also match; no fresh x86 build or guest execution is claimed. Kernel/userspace
sources and dependencies are unchanged. All **180 formal inputs** match prior
evidence (34 modules, 36 safe and 109 negative configurations plus the runner).
**0 fresh TLC cases** ran; these host operations lie outside current projections.

## Limits

Regular-file admission checks the final component. Parent directories, hard
links, permissions/ACLs and concurrent content mutation remain operator policy.
A descriptor supplies no immutable content snapshot. Streaming has fixed local
read storage, without a total byte/work or slow-filesystem deadline; a file
that keeps growing can prolong the operation. Partial filesystem faults,
file-growth races and non-Unix no-follow behavior remain unqualified.

In-place ELF signing still reopens the output path for writing. Replacement
between read and write, partial writes, power loss and atomicity remain separate
work. Bounded vectors may grow; host OOM and aggregate allocation admission are
not qualified. Original files, OS buffers, compiler/register/cache copies and
physical memory retain their own policies. Ordinary erasing Drop requires
execution; forced termination and power loss do not promise it.

These changes do not authenticate chosen keys, provide production enrollment,
rollback-resistant state or recipient custody. Production trust stays disabled.
