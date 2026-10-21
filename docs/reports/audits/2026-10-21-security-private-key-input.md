# SEC-15: private-key input review

Date: 2026-10-21. Reviewed baseline: `758a90ac`.

The private-key reader already erases its input buffer after read errors.
This review adds regression coverage and makes its normalization helper return
an erasing owner directly. It does **not** establish an earlier reader leak.
The [tracked evidence](evidence/2026-10-21-security-private-key-input.json)
records baseline controls, diagnostics, source hashes and validation.
SEC-15 remains **partial**; the ledger stays **29 corrected / 6 partial /
3 open**, and production trust stays disabled.

## Review and contract change

The initial suspicion was that a failed read could modify bytes beyond a
vector's reported length, leaving those bytes outside erasure. Inspection of
the pinned zeroize 1.9.0 implementation rules that out: its vector erasure
clears the complete capacity. The existing private-key reader installs that
owner before `read_to_end`, reserves 4097 bytes and bounds reading to that
capacity. Partial/error and dirty-EOF controls pass against the original
algorithm. An initialized-then-truncated fixture independently checks erasure
beyond logical length. No larger initialized destination or replacement reader
is needed for this result.

The old `development_fixture_hex` helper returned a plain String. Its
private-key caller immediately wrapped the result in `Zeroizing`, so ordinary
release at that call site already erased it. The renamed `normalize_key_text`
now installs the owner before copying and returns `Zeroizing<String>` directly.
It reserves the complete input length once, preserving blank-line, whole-line
comment, whitespace and hexadecimal-case handling. There is no plain-result
compatibility helper. Public fixture tests make their deliberate copies explicit.

The bounded text read moves unchanged into the same `key_input` module so
fault-reader tests exercise production code. The caller still admits the opened
regular-file descriptor, checks UTF-8, applies real-key ownership/mode policy,
validates exact encoded length and decodes into its existing erasing byte owner.
Unix no-follow/nonblocking admission, the known public fixture exemption,
file-only argv policy, errors and all wire formats remain unchanged. Normalizing
text does not validate a key or grant that exemption on its own.

This is a helper ownership improvement and verification of existing behavior,
not a reproduced secret exposure in the command path. The existing
[C18 row](../../reference/cleanup-recovery.md) records the same host file and
byte-owner scope. Ordinary byte Drop adds no Charlotte capability/root cleanup,
guards, waits, remote teardown, custody registry, physical completion or retry
authority.

## Controls and validation

An isolated package builds the baseline normalization helper and original read
algorithm with zeroize pinned to 1.9.0. In both debug and optimized profiles,
**seven controls pass and two fail as expected**. The failed controls release
the bare normalization result, once ordinarily and once through consuming
rejection. Each observes one allocation, no growth and an un-erased initialized
fixture prefix. A separate original-caller-wrapper control passes. These helper
controls do not represent a failure of the original wrapped command path.
The evidence preserves the extracted source and harness adaptation instructions.

Eight new production-helper tests cover successful bounded reads, partial
errors, dirty errors/EOF, interruption, exact-limit and sentinel rejection,
normalization grammar, consuming same-allocation rejection, and full-capacity
erasure after truncation. Only public fixture bytes are used. The allocator
observer reads eight initialized bytes immediately before System deallocation;
it reads no freed or uninitialized storage. Passing observations show the
selected ranges erased with no tracked growth. They do not measure every byte
or prove physical erasure.

The full signer suites pass **92 tests in each build profile**: 56 unit tests
and 36 integration tests. Existing real-file/CLI tests retain permission,
symlink/FIFO/directory, malformed encoding, exclusive output, signing,
operational-envelope roundtrip and key-generation coverage. Strict Clippy,
workspace formatting and whitespace checks pass. No incidental development
build/test failure was observed; expected baseline helper failures remain
separate from passing validation.

The AArch64 bundled-service build and freshness/signature check pass. All
**41 staged AArch64 and 37 staged x86_64 ELF hashes** match preceding tracked
evidence. The signing build uses the repository-pinned compiler; host tests
use the compiler recorded in the evidence. Tested host binary hashes are
captured before that signing rebuild, separately from the final signer hashes.
No new guest run or x86 build is claimed.

All **180 formal inputs** remain unchanged. No TLC cases were rerun: these
host byte owners are outside the existing model projections. The living
references and manual source describe the ownership contract; the manual PDF
is untouched.

## Remaining limits

Read fault injection qualifies the byte helper, not an exhaustive filesystem
failure matrix. File contents and permissions may change concurrently; the byte
bound is neither an I/O deadline nor a filesystem snapshot. Source files, OS
buffers, decoded cryptographic/library intermediates, compiler spills/registers,
physical memory and explicit copies retain separate lifetimes. Ordinary Drop
requires execution and does not cover forced termination, panic-abort or power
loss. Protected enrollment, rollback-resistant state and recipient-key custody
remain missing production boundaries.
