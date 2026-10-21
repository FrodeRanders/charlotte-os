# SEC-15: TLS AES round-key erasure

Date: 2026-10-21. Reviewed baseline: `74f41801`.

The service dependency now enables the existing AES round-key erasure feature.
Production trait checks reject removal of that support. This corrects one
dependency feature gap, while **SEC-15 stays partial** and the ledger remains
**29 corrected / 6 partial / 3 open**. Production trust stays disabled.
The [tracked evidence](evidence/2026-10-21-security-aes-erasure.json) records
the dependency graph, source hashes, controls, diagnostics and artifacts.

## Finding and correction

The shared TLS adapter selects embedded-tls 0.19.0's AES-128-GCM-SHA-256
suite. That uses AES-GCM 0.10.3 and AES 0.8.4. The baseline graph does not
enable AES's `zeroize` feature: the selected software and hardware destructors
compile out their round-key wiping statements. A destructor's presence alone
does not establish erasure. This is a source-confirmed missing defense, without
a demonstrated cross-domain secret read or a new retained-storage measurement.

`catten-services` now directly requires AES with `zeroize` enabled. The locked
package versions and checksums stay unchanged; the lockfile adds two dependency
edges. Feature unification supplies that same AES package to embedded-tls.
Production constant trait checks require `ZeroizeOnDrop` for `Aes128`,
`Aes128Enc` and `Aes128Dec`. They add no runtime call or allocation. The selected
software or hardware owner erases its named round-key fields on ordinary Drop.

embedded-tls constructs its AEAD cipher locally during record encryption and
decryption. Ordinary return and rejection drop that cipher, including its AES
owner. The connection still retains its original traffic keys and other state.
This feature change preserves the selected TLS suite, record bytes, certificate
checks, authentication, error/reconnect policy and capability ownership. It
does not require a new buffer, registry, wire grammar or runtime ABI.

The existing [C18 row](../../reference/cleanup-recovery.md) records local
cipher-field erasure in the existing transport scope. AES Drop introduces no
Charlotte capability/root cleanup, allocator, guard, wait, remote teardown,
custody or physical completion/retry authority. No cached dependency source is
edited or vendored.

## Validation

An isolated package compiles the exact production trait-check block with only
AES 0.8.4 and zeroize 1.9.0. With AES erasure disabled, it rejects all three
owner bounds with `E0277`; enabling the feature compiles. These expected
negative controls are separate from passing tests. They establish the feature
contract, not a memory observation of the private fields.

A new host test checks combined, encrypt-only and decrypt-only AES-128 owners
against a public fixed block vector. An independent OpenSSL invocation produces
the same `69c4e0d86a7b0430d8cdb78070b4c55a` ciphertext. The service-library,
Kafka, S3 and transport suites pass **259 distinct runtime tests and nine
compile-fail doctests in each build profile**. Repeated Kafka/S3 unit tests
from the initial library selection are excluded from that total. Strict
production-library Clippy, workspace formatting and whitespace checks pass.

Both bundled-service builds pass: **41 signed AArch64 ELFs** and **37 signed
x86_64 ELFs**. The AArch64 freshness/signature check passes. The evidence records
all final hashes and identifies the changed images. There is no new native
x86 run or direct target-memory erasure measurement.

The fresh-storage AArch64 Kafka fixture passes **20 / 0 / 0**, bitmap
`0x503ffff`, with TLS, required P-256 mTLS and SCRAM. Kernel SHA-256:
`c913536af7c00a121a7dff806f561ccbb1b85b19f6df84932b35e1056a237852`.
The runner kills route leader kafka2 in the guest's declared fault window and
restarts it afterward. Two TLS transport-failure diagnostics occur during the
outage. The later low-level transaction and generic step succeed; the step
reports four commits, four produced records, two retries, one DLQ result and
one timeout. Those fixture outcomes do not establish causes for unrelated
historical cleanup failures.

All running broker inspections identify image
`sha256:aee4e663253e674c1aa42ddc0485ff8c98ee6885997f303924085a59f8b46c7a`.
The immutable requested image reference and post-run project cleanup checks are
recorded in the evidence. The initial sandboxed image inspection was denied
before testing; its permitted retry passed. No incidental source build/test
failure was observed.

All **180 formal inputs** are unchanged. No TLC cases were rerun: dependency
field erasure is outside the existing projections. Living references and the
manual source describe the scope; the manual PDF is untouched.

## Remaining scope

The pinned embedded-tls key schedule retains plain traffic-key/IV arrays and
HKDF state without an erasing destructor. AES-GCM's authentication-key
temporary and GHASH/POLYVAL state have separate erasure gaps; enabling AES
does not enable or qualify those paths. In particular, dependency erasure
features must be checked against the selected backend rather than assumed to
cover every target.

Caller keys, TLS/library scratch, scalar/compiler copies, registers/cache,
kernel/socket backing and physical memory retain their own lifetimes. Ordinary
Drop requires execution; forced termination, panic-abort and power loss are
outside this correction. Protected enrollment, rollback-resistant policy state
and recipient-key custody remain missing production boundaries.
