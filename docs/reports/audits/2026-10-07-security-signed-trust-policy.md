# Signed bootstrap policy and revision lineage

Date: 2026-10-07. Baseline: `6b3298e4`, following the
[public trust preflight](2026-10-07-security-trust-preflight.md).
This implements public-policy authentication under an explicitly supplied
bootstrap anchor. **SEC-04 remains partial; no finding is closed by this batch.**

## Implementation

The shared `CBTRUST1` representation is exactly 328 bytes: a canonical header,
full bootstrap-key digest, predecessor digest, the existing 184-byte `CTRUST1`
candidate and a 64-byte Ed25519 signature. The signature covers a domain-separated
SHA-256 digest of every non-signature byte. There is no unprotected extension
field or self-selected verification key. Its signed-fields digest identifies
the accepted policy independently of signing noise.

`BootstrapKey` rejects invalid/noncanonical/non-prime-order points and shipped
development fixtures, including known sign/conversion aliases. Its identity
must be separate from all four admission roles. Encoding and verification both
enforce that qualification, so a correctly signed policy cannot bypass it.
Existing production-candidate validation retains its behavior; its signing-point
helper is shared rather than duplicated. No dependency/version changes or
new cryptographic primitive implementations are introduced.

`PolicyExpectation` requires explicit enrollment or installed state. Enrollment
needs a nonzero minimum revision and zero predecessor. Installed state needs a
nonzero accepted revision/digest: older revisions reject, the current revision
must match that digest, and a successor must advance exactly once and name it
as predecessor. There is no wraparound, inferred floor or automatic enrollment
fallback. A verified receipt remains public data, not boot/installation authority.

`trust-policy-sign` accepts the bootstrap private key by restricted file path
only, validates its private/public halves, signs and self-verifies before
exclusive output creation. Its temporary decoded private buffer is zeroized
immediately after constructing the library-owned secret key. The existing
library secret-key owner wipes on Drop. It has no legacy argv-secret exception.
`trust-policy-verify` requires the public anchor, cluster and explicit state;
records are bounded regular-file inputs with the existing Unix no-follow/nonblocking
reader. Neither command installs policy or advances protected state.

Contract: [signed bootstrap policy](../../reference/bootstrap-trust-policy.md).
Commands: [signing guide](../../guides/signing-and-trust.md#signed-trust-policy).

## Evidence

- **Eight new shared-protocol tests** pass. They verify exact wire layout,
  successful initial/current/successor acceptance, key-role separation,
  development/weak/mixed-order root rejection, domain-separated signing and
  explicit revision-state validation.
- Every one of the **328 record bytes** is independently altered and rejected.
  Every truncated prefix and trailing input reject through verification and
  signing helpers without panic. Unsigned records, wrong signers/anchors,
  substituted key selectors and foreign clusters reject.
- Valid operator signatures cannot admit an old revision, same-revision policy
  substitution, a skipped revision or a wrong predecessor against installed
  state. A valid alternate successor passes against the old state but conflicts
  after the first successor is accepted: this tests why a future installer must
  serialize state commit, rather than reuse stale expectations concurrently.
- Fresh signing noise produces a different valid signature with the same
  accepted policy digest. `u64::MAX` can be checked as the current revision;
  signing cannot wrap it into a new revision.
- **Three new signer/file tests** pass (13 signer tests total). They cover
  enrollment/rotation/current checks, failed rollback/conflict/context,
  no-output rejection, existing-output preservation, explicit-state syntax,
  invalid/uninitialized/overflow state, unsafe private-file mode, mismatched
  key halves, fixture/role reuse, raw argv-secret rejection, bounded records,
  symlinks, FIFOs, directories and signature tampering.
- A separate command-line process smoke test generates temporary test keys,
  signs and verifies initial/successor/current policies, and confirms rollback,
  signature tamper and digest-conflict diagnostics. Its owned temporary files
  are removed afterward; no private key contents are printed.

## Validation

- `scripts/run-host-tests.sh`: passed, including all discovered host crate tests,
  prior candidate tests, signing-policy/boot-result checks and signer self-test.
- Host Clippy (`charlotte-launch`, `cluster-sign`, all targets, `--locked`,
  `-D warnings`): passed.
- x86 kernel Clippy with its required bundle environment, `--locked`,
  `-D warnings`: passed.
- `scripts/build-catten-services.sh --embed`: passed.
- Arm kernel Clippy with its required bundle environment, `--locked`,
  `-D warnings`: passed.
- Direct `CATTEN_TRUST_MODE=production` kernel check: rejected with the expected
  build-script diagnostic; policy signing does not bypass that gate.
- `cargo fmt --all -- --check`, `git diff --check` and local documentation-link
  checks: passed.

Logs: `/private/tmp/charlotte-signed-policy-host.log`,
`/private/tmp/charlotte-signed-policy-tests.log`,
`/private/tmp/charlotte-signed-policy-clippy-{host,x86,arm}.log`,
`/private/tmp/charlotte-signed-policy-services.log` and
`/private/tmp/charlotte-signed-policy-cli.log`. Direct production rejection is in
`/private/tmp/charlotte-signed-policy-production-rejection.log`.

## Remaining boundaries

This verifies public policy relative to caller-supplied anchor/state. Those
inputs are not protected by a CLI argument or host file. There is no firmware
or kernel installer, authenticated executable boot chain, atomic persistent
rollback-state update, recipient custody provider or bootstrap-root rotation
protocol in this batch. Missing or failed protected state must never become a
fresh enrollment request. A future installer must commit state and publish
authority atomically, including recovery after power loss.

The format contains no kernel/bootloader digest, credentials or UTC expiry.
Full crypto-state zeroization assurance remains SEC-15 work. Production builds
stay disabled and development launch policy remains unchanged. QEMU guests were
not rerun for this offline protocol/tooling change; no runtime boot, retirement
or device-recovery path is added. The current ledger remains 20 scoped
corrections, six partial findings and four open findings.
