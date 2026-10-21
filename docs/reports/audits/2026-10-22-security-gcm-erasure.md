# SEC-15: GCM scratch and software-state erasure

Date: 2026-10-22. Reviewed baseline: `cb205cd9`.

The shared TLS dependency now enables authentication-key scratch wiping in
AES-GCM and GHASH, and state erasure in POLYVAL's software backend. AArch64
builds require destructor presence for the selected GHASH/POLYVAL owners.
**SEC-15 remains partial**; the ledger stays **29 corrected / 6 partial /
3 open**, and production trust stays disabled. The
[tracked evidence](evidence/2026-10-22-security-gcm-erasure.json) records the
feature graph, source hashes, controls, diagnostics and artifacts.

## Finding and correction

The preceding AES fix covered expanded round keys. The locked AES-GCM 0.10.3,
GHASH 0.5.1 and POLYVAL 0.6.2 packages each gate separate wiping statements
behind their own `zeroize` feature. Those features were absent in the baseline.
AES-GCM leaves its derived GHASH key scratch without wiping; GHASH does the
same for its reversed and transformed key copies. POLYVAL's software state
destructor is compiled out. These are source-confirmed gaps, without a new
private-field memory probe or demonstrated cross-domain read.

`catten-services` now enables all three features directly. The lockfile adds
six dependency edges; package versions and checksums are unchanged. The
resolved graph supplies the same packages to embedded-tls. AES-GCM and GHASH
clear their named construction temporaries after copying them into their
existing state owners. POLYVAL's software destructor clears its key and
accumulator fields on ordinary Drop. There is no new buffer or allocation.

A production type-equality check ties the dependency to the TLS-selected
AES-128-GCM cipher. AArch64 constant assertions require `needs_drop` for both
selected authentication-state types. This checks destructor presence; source
inspection establishes what that destructor does. It is not a memory-erasure
measurement or a general proof that every destructor erases secrets.

The current AArch64 software backend satisfies those assertions. The optional
PMULL backend has no erasing destructor and now rejects this build guard,
even with the feature enabled. No PMULL or additional CPU feature is enabled
for CharlotteOS. The pinned x86 dispatch owner also lacks Drop delegation;
its live state remains an explicit gap. Consuming finalization can drop an
extracted backend, while dropping the dispatcher itself does not.
This batch does not claim complete x86 authentication-state erasure.

Record/cipher bytes, TLS suite, authentication, capability ownership and error
or reconnect policy stay unchanged. The existing
[C18 row](../../reference/cleanup-recovery.md) records the same local record
cipher and byte fields. Their Drop adds no Charlotte capability/root cleanup,
allocator, guard, wait, remote teardown, registry, custody or physical/retry
authority. No dependency source is edited or vendored.

## Controls and validation

An isolated package builds the exact production AArch64 assertions. With
erasure disabled, it rejects the POLYVAL destructor check with `E0080`.
Enabling the features compiles. Enabling optional `polyval_armv8` while keeping
erasure enabled rejects the same assertion. Those two expected failures are
separate from passing tests. Final controls pin zeroize 1.9.0 to match the
repository; an earlier exploratory enabled control used 1.9.1 and also passed.

The new test uses the actual TLS-selected cipher for a public all-zero GCM
vector, matching ciphertext and tag, rejecting a changed tag without modifying
the ciphertext, and decrypting the authentic result. Node v22.16.0 with
OpenSSL 3.0.16 independently produces the same ciphertext
`0388dace60b6a392f328c2b971b2fe78` and tag
`ab6e47d42cec13bdf53a67b21257bddf`.

Service-library, Kafka, S3 and transport suites pass **260 runtime tests and
nine compile-fail doctests in each build profile**. Strict production-library
Clippy, workspace formatting and whitespace checks pass. No incidental source
build/test failure was observed. The source/feature checks do not inspect
private scratch, cipher padding or post-Drop storage.

Both service bundles build and sign: **41 AArch64 ELFs** and **37 x86_64
ELFs**. The AArch64 freshness/signature check passes. The evidence records
all final artifact hashes and changed-image names. There is no new x86 guest
run or direct target-memory erasure measurement.

The fresh-storage AArch64 S3 TLS fixture passes **20 / 0 / 0**, bitmap
`0x303ffff`, including a verified 33-byte PUT/HEAD/GET/DELETE round trip.
Kernel SHA-256:
`8b3537a165bead5f254c5fa1dc670e249cc852e05bd6d4653e42fd588f3a47a4`.
Immutable RustFS server and CLI image IDs were inspected before launch.
The attempted running-container inspection came after fixture cleanup and
returned `no such object`; no running-container inspection is claimed.
Post-run project container and volume inventories are empty. Relevant guest
results and the inspection limitation are retained inline in the evidence.

All **180 formal inputs** remain unchanged. No TLC cases were rerun: these
local cryptographic field lifetimes are outside the existing projections.
Living references and the manual source describe the qualified scope; the
manual PDF is untouched.

## Remaining scope

TLS traffic-key/IV arrays and HKDF state still lack an erasing owner. The
pinned x86 POLYVAL dispatcher and optional ARM PMULL backend need separate
corrections and validation. Explicit copies, scalar/compiler intermediates,
registers/cache, caller/launch/kernel/socket backing and physical memory have
separate lifetimes. Ordinary Drop requires execution; forced termination,
panic-abort and power loss are outside these checks. Protected enrollment,
rollback-resistant policy state and recipient-key custody remain missing
production boundaries.
