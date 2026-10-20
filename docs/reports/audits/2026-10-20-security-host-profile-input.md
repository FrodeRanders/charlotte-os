# SEC-15: host plaintext profile input

Date: 2026-10-20. Reviewed baseline: `06ae008`.

`cluster-sign operations-seal` now owns its plaintext destination before the
first read. The [tracked evidence](evidence/2026-10-20-security-host-profile-input.json)
records controls, source/artifact hashes and execution results. SEC-15 remains
**partial**, with **29 corrected / 6 partial / 3 open** overall.

## Finding and correction

The former `Zeroizing::new(fs::read(profile_path)?)` installed its erasing owner
only after the entire read succeeded. The installed host standard library's
`std::fs::read` creates a plain vector, reserves from file metadata and performs
fallible reads before returning it. A partial error could release copied
plaintext before the tool received an owner. Empty/oversized input already
failed later envelope validation, after reading; this was no encryption-policy
bypass. The file read also followed symlinks and could block opening a FIFO.

The private `profile_input` module opens one descriptor, checks regular-file
metadata and the existing 1..=65536-byte envelope bound, then reads into one
fully initialized 65537-byte erasing destination. Unix opens use `O_NOFOLLOW`
and `O_NONBLOCK`, rejecting symlinks and permitting non-regular admission to
reject without waiting for a FIFO writer. Checks apply to the opened descriptor.
An extra byte detects growth beyond the bound during reading. There is no
vector growth or alternate plaintext input path. Interrupted reads retain the
same destination; other errors drop it.

`ProfileInput` lends only the accepted prefix, retaining the complete backing
until Drop. It also erases bytes a faulty reader wrote beyond its reported
count, including a dirty error/EOF. Such synthetic cases qualify the helper's
stronger destination contract; they are not observations of `std::File`.
The caller borrows accepted bytes through envelope sizing and encryption.

CLI arguments, COPSENC1 bytes, cryptography, exclusive output creation and
plaintext-output mode 0600 are unchanged. No dependency, runtime API, wire or
persistent-format change is introduced. The existing [C18 row](../../reference/cleanup-recovery.md)
records this host byte owner. Ordinary byte Drop adds no Charlotte capability
release, guards, waits, remote teardown, recovery registry/custody, physical
completion or replay authority. Manual LaTeX and tooling/signing references
are updated; the PDF is untouched.

## Controls and host validation

The baseline read-stage adapter preserves `Zeroizing::new(fs::read(path)?)`.
Its generic fault adapter preserves a plain `Vec::read_to_end` followed by the
outer wrapper, enabling a controlled partial-read error. It lacks `File`'s
metadata capacity hint; its allocation/growth counts do not reproduce exact
`std::fs::read` behavior. No actual partial filesystem failure is claimed.
Sources, manifest, lockfile and setup are embedded in the evidence.

Baseline release controls produce **four expected failures and one pass**.
After eight public `Z` bytes then a read error, the observer reports
`allocations: 1, growths: 1, copy_releases: 1, copy_erased: false`.
Empty, oversized and symlink read-stage rejection controls also fail. The
original full CLI already rejected empty/oversized input later; these controls
cover its earlier read stage only. The existing successful-owner erasure
control passes. A separate exact-byte parity control passes. The parity
adapter emits one unused-function warning. Expected failures are retained
separately from passing corrected runs.

Seven new tests cover successful release, partial error, dirty errors/EOF and
impossible counts, dirty tails beyond a short accepted prefix, exact-limit and
sentinel bounds, interrupted reads, and Unix file/CLI admission. The allocator
reads eight initialized public fixture bytes, with volatile accesses immediately
before actual deallocation. Observation ends when that allocation is released;
no freed bytes, padding or uninitialized spare capacity are read. Selected
prefix/tail erasure and no-growth assertions pass; these probes do not prove
whole physical-memory erasure.

The real CLI test rejects empty/oversized regular files and symlinks without
creating an envelope. The file helper also rejects a directory and FIFO. An
exact-64-KiB public payload seals and opens through the actual built tool, with
exact plaintext recovery, expected ciphertext length, mode-0600 output and
existing-envelope rejection preserving its bytes. This opaque `Z` payload
qualifies the envelope roundtrip, not S3 profile-codec admission. Public
development keys are used. The fixture owns its mode-0700 directory only after
exclusive creation succeeds.

Debug tool tests pass **31 runtime tests**. Optimized tool/launch/Kafka/protocol/
S3/transport suites pass **229 runtime tests and 13 compile-fail doctests**.
Strict tool Clippy, workspace formatting and whitespace checks pass. Both
service builds pass: **41 signed AArch64 ELFs** and **37 signed x86_64 ELFs**;
the AArch64 freshness/signature check passes. All 78 staged ELF hashes match
the preceding key-file evidence. No incidental corrected build/test failure
was observed. No fresh guest run is claimed for this host-only change.

## Limits

Input permissions and ACLs remain the operator's responsibility; this adds no
private-key-style mode/ownership policy for profiles. An opened descriptor
prevents path replacement from substituting the checked file, but concurrent
content mutation can still change the bytes read. A byte bound supplies no I/O
deadline on slow regular or remote filesystems. Unix admission is tested on
the host; non-Unix no-follow handling, filesystem fault injection and concurrent
mutation are unqualified. Encrypted-envelope readers remain separate review
work.

Other host/library copies, source files, OS buffers/storage, compiler spills,
registers/cache and physical memory remain separate boundaries. Ordinary Drop
requires execution; abort, forced termination and power loss remain outside
these observations, as do OOM recovery and aggregate heap admission. Protected
enrollment, rollback-resistant state and recipient custody remain missing;
production trust stays disabled.

All **180 formal inputs** match preceding evidence: 34 modules, 36 safe and
109 negative configurations plus the runner. No fresh TLC cases were run;
these host byte owners are outside existing projections. Kernel/userspace
sources and the lockfile are unchanged.
