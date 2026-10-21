# Security audit: bounded public-key file inputs

Date: 2026-10-21. Reviewed baseline: `1c1a593`.

Public-key file arguments in `cluster-sign` now share the trust-policy commands'
bounded regular-file reader and hex parser. The [tracked evidence](evidence/2026-10-21-security-public-key-input.json)
contains baseline controls, diagnostics and source/artifact hashes. This closes
one host-tool input gap. Numbered findings remain **29 corrected / 6 partial /
3 open**; SEC-06 management flood fairness and SEC-15 secret-state work retain
their existing acceptance criteria.

## Finding and correction

Ordinary `read_hex_key` used `fs::read_to_string`, read the complete file before
checking decoded width and followed symlinks. A file containing a valid 32-byte
key plus arbitrarily large comments was accepted. A FIFO could block opening.
The trust-policy module already had bounded, no-follow regular-file admission
and separately normalized/decoded public keys. The tooling guide's broad claim
of bounded key-file reads therefore exceeded ordinary command behavior.

The existing `read_bounded_regular` moves unchanged from the trust-policy module
to its parent tool module. Public key files, public policy candidates and signed
policy inputs use that one helper. It opens once, checks the opened descriptor's
regular-file type and byte limit, then reads at most the limit plus one sentinel
byte. Growth beyond the limit rejects. Public key files use a 4096-byte bound,
including comments/whitespace. Policy byte limits remain unchanged. On Unix,
`O_NOFOLLOW | O_NONBLOCK` rejects final-component symlinks and permits FIFO
admission to reject without waiting for a writer.

The trust module now delegates public key parsing to `read_hex_key`; there is
one UTF-8/trim/blank/comment/hex path. Exact decoded width checks remain. The
unbounded `read_to_string` path and separate trust-policy normalizer invocation
are removed. Main callers cover recipient keys for sealing and bundle signing,
operational verification keys for open/verify and bundle verification, recipient
keys for bundle verification, and operational keys for ingress-policy verify.
Trust-policy creation and signed-policy verification use the same decoder.

Existing commands that define public hex arguments keep their interface.
Private-key reading, its development-content permission exception, signing-key
owners, encrypted-envelope input readers, output creation, cryptography and
signed formats are unchanged. No dependency, kernel/runtime API or persistent
format changes. The existing [C18 row](../../reference/cleanup-recovery.md)
records the shared local byte-release scope. Ordinary byte Drop adds no
capability/root cleanup, guards, waits, remote teardown, registry, custody or
physical/replay authority. Living Markdown and manual LaTeX are updated; the
PDF is untouched.

## Baseline and corrected controls

The tests ran directly against `1c1a593` production functions with only the
new unit tests added. No byte adapter replaces file operations or decoding.
The evidence embeds the exact test-only patch, its hashes, original functions
and relevant test diagnostics.

Two optimized baseline controls fail separately: the reader accepts a valid
key followed by a 4096-byte whole-line comment (**4162 total bytes**), and it
accepts a symlink to a valid public key. Both fail the assertion that
`read_hex_key(...).is_err()`. A separate baseline grammar control passes:
exactly 4096 bytes of uppercase hex, whitespace, blank lines and comments decode
to the expected key, while malformed UTF-8, odd/invalid hex and wrong decoded
width reject. No baseline FIFO is opened and no large-file/OOM failure is
inferred from these small controls.

Four new unit tests cover those cases plus missing files, directories and FIFO
rejection. One integration test invokes the actual built `operations-seal` and
`operations-verify` commands. It uses commented, exact-4096-byte public fixture
keys to seal and verify an opaque public payload. Each command then rejects
symlink, 4097-byte comment-padded key, directory and FIFO inputs. Rejected seals
create no output; every rejected operation preserves the valid envelope's exact
bytes. The test creates its uniquely owned mode-0700 fixture directory before
arming cleanup. Development keys and payload are public. The payload qualifies
the signed envelope path, not the S3 wire-profile grammar.

Both debug and optimized tool suites pass **36 runtime tests**: 23 unit tests,
5 key-encoding tests, 7 profile-input tests and the new CLI test. Existing trust
policy sign/verify, private-file policy, key generation and plaintext envelope
roundtrip controls pass. Strict all-target tool Clippy, workspace formatting
and whitespace checks pass. No incidental corrected build/test failure was
observed. Baseline failures are retained separately from passing repeats.

The AArch64 service build and freshness/signature check pass, with **41 signed
ELFs** identical to the preceding profile-input evidence. Staged x86 ELFs also
retain their previous hashes; no fresh x86 build or guest execution is claimed.
This host-only change leaves kernel/userspace sources unchanged. All **180 formal
inputs** match preceding evidence (34 modules, 36 safe and 109 negative configs
plus the runner); **0 fresh TLC cases** ran. These host file readers are outside
existing formal projections.

## Limits and remaining work

No-follow applies to the final path component; parent directories, hard links,
file permissions/ACLs and concurrent content mutation remain operator policy.
Descriptor checks prevent path replacement from substituting the opened file,
without promising a content snapshot. A byte limit provides no deadline for
slow regular or remote filesystems. File-growth races and partial I/O faults
were not injected. Non-Unix no-follow enforcement is unqualified. Public inputs
still use the existing growable vector within the read bound; this is no fixed
allocation or erasure proof.

Public-file admission does not authenticate a chosen key, establish private-key
custody or enroll trust. Other tool inputs, including encrypted envelopes and
public hex arguments, retain their existing bounds and policies and need their
own review. Management flood resistance, aggregate admission, broader secret
copies and physical memory remain separate. Protected enrollment,
rollback-resistant state and recipient custody are still missing; production
trust stays disabled.
