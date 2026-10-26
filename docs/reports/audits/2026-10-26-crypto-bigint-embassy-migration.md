# SEC-14/15: crypto-bigint migration through Embassy TLS

Date: 2026-10-26. Reviewed baseline: `88694e77`.

**Use the Embassy-based embedded-tls main branch as the migration target, pinned
to reviewed commits.** The isolated candidate removes the vulnerable 0.5 curve
family, passes a fresh advisory scan, and compiles on both freestanding service
targets. Six host controls pass in debug and release. This is a migration
assessment and prototype: the active workspace still uses registry embedded-tls
0.19.0 and its advisory gate still rejects crypto-bigint 0.5.5. No live TLS
cutover, native TLS execution or audit finding closure is claimed here.

The [evidence](evidence/2026-10-26-crypto-bigint-embassy-migration.json) records
exact input hashes, revisions, reports, commands and limitations. The archived
[manifest template](evidence/2026-10-26-embassy-tls-probe/Cargo.toml.in),
[lockfile](evidence/2026-10-26-embassy-tls-probe/Cargo.lock.txt) and
[Rust fixture](evidence/2026-10-26-embassy-tls-probe/lib.rs) reproduce the candidate
without adding an experimental package to the root workspace. SEC-14 and SEC-15
remain partial, the ledger stays **29 corrected / 6 partial / 3 open**, and
production trust remains disabled.

## Selected dependency path

The current path is crypto-bigint 0.5.5 → elliptic-curve 0.13.8 → P-256/P-384 →
embedded-tls 0.19.0 → catten-services. There is also a direct P-256/signature
dependency in the Charlotte TLS adapter. Updating a transitive lockfile entry
alone cannot cross these API families. RustSec's
[RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html) lists
patched crypto-bigint versions starting at 0.7.6.

The selected source inputs are:

| Component | Exact revision | Role |
| --- | --- | --- |
| [embedded-tls](https://github.com/embassy-rs/embedded-tls/tree/6487c62bb084c33361822022e21836ddfd48388f) | `6487c62bb084c33361822022e21836ddfd48388f` | Embassy-based TLS main, committed October 23. |
| [embassy-crypto / RustCrypto backend](https://github.com/embassy-rs/embassy/tree/822a76ea91f2c0885c6eb0c65fe91181e31133e1) | `822a76ea91f2c0885c6eb0c65fe91181e31133e1` | The revision already required by that TLS commit. |
| [unitrait](https://github.com/embassy-rs/unitrait/tree/bb96601b3d293fa27d1ca2891f6ea148e534e433) | `bb96601b3d293fa27d1ca2891f6ea148e534e433` | Transitive link-time driver implementation. |

Upstream main still declares package version **0.19.0** despite its different
API. Record the source revision, not just that version. Use `git` plus full
`rev`, retain the lockfile, and qualify later revision changes deliberately.
Do not track a floating branch. The backend selects P-256/P-384 0.14.0,
elliptic-curve 0.14.3, ECDSA 0.17.0 and crypto-bigint **0.7.10** in this probe.
There is one crypto-bigint package and no 0.5 copy in its complete lockfile.

Enable TLS `rustpki,p256,p384,ed25519`; the old TLS `alloc` feature is removed.
Select only the required SHA-256/384/512, HMAC-SHA-256/384, P-256 ECDH/ECDSA,
P-384 ECDSA and Ed25519 backend features. Do not enable the backend's `all`
feature. Embassy's crypto driver interface does not require migrating Charlotte
to an Embassy executor; the existing synchronous socket adapter can remain.

## Ownership and API work before cutover

| Boundary | Required migration |
| --- | --- |
| Entropy | Replace per-handshake `CryptoProvider::rng` with the link-time `embassy_crypto::driver::Rng` implementation. Prepare a domain-owned entropy context before publishing TLS users. |
| Verification | Construct the existing CA/hostname verifier directly and pass it to `TlsContext::new`. Preserve the nonzero synchronized-time precondition and fail-closed configuration checks. |
| mTLS | Replace the provider's signer with `TlsConfig::with_client_cert` and a typed `PrivateKey::EcdsaP256`. Retain P-256 SEC1 validation and zeroizing intermediate scalar storage. |
| Record cipher | Register exactly one AES-128-GCM driver with an erasing context, and assert the actual selected TLS cipher contract. |
| Transport owner | Keep `OwnedSocket`, stable record-buffer boxes, explicit consuming close and connection-before-buffer destruction. Recheck failure/drop paths against the new connection type. |
| Callers and errors | Update S3, Kafka, curl and probes together; remove obsolete direct signature/rand dependencies only after their remaining callers compile. Preserve authentication versus transport failure and accepted-request replay rules. |

The new [RNG interface](https://github.com/embassy-rs/embassy/blob/822a76ea91f2c0885c6eb0c65fe91181e31133e1/embassy-crypto/src/driver.rs)
is global and infallible. It cannot borrow a short-lived handshake connection.
Installation should consume a dedicated owned `Connection` and retain it for the
domain lifetime, using the normal owned RPC/mapping path for entropy. Launch-owned
connections remain borrowed through `Context`; installation must not adopt them.
Resolve how each TLS consumer obtains that owned connection during startup before
changing its public API. Architectural random remains usable under the existing
policy. Missing installation, unavailable entropy and reentrant use must fail
closed; neither deterministic bytes nor host `getrandom` belongs in service code.
The probe's host RNG is a **dev dependency only**, not that production adapter.

One-time installation and concurrent access need an explicit admission/lifetime
contract, without holding a spin lock across an entropy RPC. The current TLS
clock is also domain-global under a synchronous single-handshake assumption.
Preserve that assumption with checked admission, or replace the clock contract
before allowing concurrent opens. The new backend draws randomness for ECDSA
signing as well as ECDHE; the installed source must cover both. This migration
does not authorize new timer polling for resource readiness.

The [new SEC1 parser](https://github.com/embassy-rs/embedded-tls/blob/6487c62bb084c33361822022e21836ddfd48388f/src/config.rs)
selects the curve by scalar length. A public scalar-one fixture with a changed
curve OID is accepted by that convenience parser, while the typed P-256 parser
rejects it. The probe's `load_client_key` preserves rejection by using P-256
0.14 with its `pkcs8` feature, then constructs the Embassy key from a zeroizing
scalar temporary. This is a configuration-validation difference, not evidence
that a mismatched certificate can authenticate. The external signer variant
requires a static signer; a borrowed per-handshake signer is not a drop-in path.

Upstream's verifier skips certificate date checks if its clock returns `None`.
Do not use that as a fallback. Requalify hostname, validity and trust-root
rejection. Its directly root-signed leaf restriction remains a limitation;
the migration does not add general intermediate-chain support.

## Preserve record-key erasure

The pinned [stock GCM driver](https://github.com/embassy-rs/embassy/blob/822a76ea91f2c0885c6eb0c65fe91181e31133e1/embassy-crypto-rustcrypto/src/aes.rs)
uses a plain `[u8; 16]` context and constructs a temporary AES-GCM owner for each
operation. Its retained array has no erasing destructor. The generated
[unitrait destructor](https://github.com/embassy-rs/unitrait/blob/bb96601b3d293fa27d1ca2891f6ea148e534e433/unitrait-macros/src/lib.rs)
forwards the registered context's destructor; it does not independently wipe
the opaque storage. `needs_drop::<embassy_crypto::Aes128Gcm>()` alone therefore
does not establish key erasure.

The candidate disables the stock GCM feature and registers a Charlotte driver
whose context is the existing AES-GCM 0.10.3 owner, retaining AES 0.8.4,
GHASH 0.5.1 and POLYVAL 0.6.2 with their zeroize features. It uses Embassy's
safe in/out-buffer conversion; no local arithmetic, TLS fork or unsafe pointer
adapter is needed. The selected cipher passes the existing public GCM vector,
bad-tag rejection without ciphertext mutation, and successful decryption.
Separate input/output buffers round-trip with nonempty associated data too.

Both freestanding checks retain the AES `ZeroizeOnDrop` bound and GHASH/POLYVAL
destructor guards. x86 retains the repository's `polyval_force_soft` selection.
The registered context fits Embassy's opaque layout on both 64-bit targets.
These are compile/source and functional checks, not a post-destruction memory
or constant-time proof. The curve API has explicit erasing secret-key,
signing-key and shared-secret destructors. HKDF PRK, traffic-secret/IV fields,
temporary copies and forced termination still require the existing SEC-15
qualification; selecting Embassy does not close that finding.

## Executed checks and remaining acceptance

The final candidate has **84 locked packages, including the probe itself**.
A fresh unfiltered cargo-audit 0.22.2 scan reports zero vulnerabilities and zero warnings, with no
ignores, at database commit `b655eaacb2b622c94865e29bc4c3d2e11d206323`
(1355 advisories). The same policy's fresh active-root scan at database commit
`ea60a296f48f91775a152ff3a0fea89e11001b2e` still reports the crypto-bigint 0.5.5
advisory. The scans captured different database revisions; both are retained.
A candidate pass is not a root-workspace pass.

Both custom service targets pass `cargo check --locked --offline` with the
repository toolchain and target configuration. The final six host tests pass
in debug and release with `nightly-2026-07-27`: selected TLS GCM vector/tamper
contract, separate buffers/AAD, typed SEC1 OID rejection, P-256 ECDH agreement
and invalid peer rejection, and P-256/P-384 signature verification with changed
digest rejection. These host tests link the registered drivers. Freestanding
checks compile libraries; they do not link a native service or exercise a
Charlotte entropy driver.

Preliminary stock-backend checks passed on both targets and its earlier
78-package graph scanned clean. They were superseded by the erasing-driver
candidate; their input lock hash was not captured, so they are not final artifact
evidence. The first extended SEC1 fixture failed to compile because it omitted
the P-256 OID feature and hybrid-array conversion. An offline attempt then
could not download the newly required PKCS8/SPKI packages. Enabling `pkcs8`,
correcting the conversion and fetching those packages preceded all final
passes. Earlier host passes used the ambient newer nightly; the recorded final
host qualification explicitly selects the repository's pinned toolchain.

Implement the migration in this order:

1. Add the domain-owned entropy installation/admission contract and cover
   installation failure, unavailable entropy, duplicate installation and
   reentrant/concurrent access. Keep SCRAM's fallible nonce API and deadlines.
2. Port `tls_client` and all consumers to the pinned graph, typed key parser,
   verifier and erasing cipher driver. Remove the old provider and dependency
   family; check the complete root graph for duplicates and advisories.
3. Run the TLS/mTLS interop and rejection matrix: valid server and client,
   wrong host/CA, expired/not-yet-valid dates, missing time, malformed/wrong-curve
   key, rejected client identity, tampered/truncated records, transport failure,
   close and drop after handshake/read/write rejection. Exercise S3 and Kafka
   through their existing profiles and uncertain-timeout policy.
4. Rebuild/sign both service bundles and the separate Sitas image; execute
   native Intel/AMD/AArch64 probes, including the selected cipher and real
   entropy path. Update the C18 cleanup row, living TLS references, Markdown
   configuration documentation and manual to describe the final behavior.

No native boot, TLS server interop, service bundle, full workspace test, manual
PDF rebuild, license/provenance review or timing analysis is performed in this
assessment. Runtime source, root manifest/lockfile, limits and capability policy
are unchanged. The exact probe inputs and commands below keep the next
implementation step concrete and reviewable.

## Reproduction

Run from the repository root. The template materializes an isolated disposable
workspace under ignored `target`; it is not another root-workspace package.

```sh
migration_root="$PWD"
mkdir -p target/crypto-migration/reproduce/src
cp docs/reports/audits/evidence/2026-10-26-embassy-tls-probe/Cargo.toml.in target/crypto-migration/reproduce/Cargo.toml
cp docs/reports/audits/evidence/2026-10-26-embassy-tls-probe/Cargo.lock.txt target/crypto-migration/reproduce/Cargo.lock
cp docs/reports/audits/evidence/2026-10-26-embassy-tls-probe/lib.rs target/crypto-migration/reproduce/src/lib.rs
cargo fetch --locked --manifest-path target/crypto-migration/reproduce/Cargo.toml
cargo check --locked --offline --manifest-path target/crypto-migration/reproduce/Cargo.toml --target crates/catten-services/aarch64-unknown-none.json --target-dir target/crypto-migration/build
cargo check --locked --offline --manifest-path target/crypto-migration/reproduce/Cargo.toml --target crates/catten-services/x86_64-unknown-none.json --target-dir target/crypto-migration/build
cargo audit --db target/security-audit/advisory-db --url https://github.com/RustSec/advisory-db.git --file target/crypto-migration/reproduce/Cargo.lock --json --deny warnings
cd /private/tmp
cargo +nightly-2026-07-27 test --locked --offline --manifest-path "$migration_root/target/crypto-migration/reproduce/Cargo.toml" --target-dir "$migration_root/target/crypto-migration/host-build"
cargo +nightly-2026-07-27 test --locked --offline --release --manifest-path "$migration_root/target/crypto-migration/reproduce/Cargo.toml" --target-dir "$migration_root/target/crypto-migration/host-build"
cd "$migration_root"
```

Host commands deliberately run outside the root's freestanding build-std
configuration. The advisory command refreshes external data and can change
result over time. Apply the complete-report validator in
`scripts/check-dependency-advisories.py` to its JSON; cargo-audit's zero exit
alone does not reject every informational notice. The production wrapper
continues to scan only the root lockfile and is not redirected to this probe.
