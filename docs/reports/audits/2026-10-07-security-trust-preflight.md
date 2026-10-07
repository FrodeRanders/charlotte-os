# Public trust-policy preflight and remediation criteria

Date: 2026-10-07. Baseline: `4e12b19e`, following
[whole-domain thread abort ownership](2026-10-07-security-domain-thread-abort.md).
This begins the next production-trust workstream and defines remaining SEC-07/18
acceptance criteria. **No finding is newly closed by this batch.**

## Changes

`ProductionTrustCandidate` is an immutable public-policy preflight result in
`charlotte-launch`. It requires a caller-supplied expected cluster and nonzero
minimum policy revision. The four public role keys must be distinct; every shipped
development root is rejected in every role. Curve validation rejects invalid,
noncanonical, small-order or mixed-order Ed25519 points, and noncanonical or
non-contributory X25519 recipients. Montgomery identities additionally reject
sign-negated fixture/role keys and signing keys converted into recipients.

Validation uses `curve25519-dalek` 5.0.0, already locked and used by HPKE. The
manifest adds an explicit dependency with default features disabled; the lockfile
adds that dependency edge without changing a package version. Validation uses
fixed-size stack storage and does not require heap allocation or random input.
No bespoke signing, encryption or point-arithmetic implementation is introduced.

`cluster-sign trust-policy-create` reads four public-key files and writes the
unchanged, 184-byte `CTRUST1` representation using exclusive creation.
`trust-policy-check` checks exact format, cluster and revision floor. Public-key
inputs are bounded to 4096 bytes; policy input is bounded to its fixed encoded
length. Unix descriptor-based readers reject symlinks, non-regular inputs and
FIFO blocking. Errors publish no new policy and existing output is preserved.
The commands report a public digest and identify their output as unsigned.

The [living remediation ledger](../../reference/security-remediation.md)
reconciles all 30 numbered findings, including the five corrected SEC-26–30
follow-ups omitted from earlier SEC-01–25 counting. It defines an allocation
inventory, remaining heap/metadata/principal admission, progress and pressure
criteria for SEC-07; and locking, cooperative shutdown, retained-owner recovery,
device coverage and concurrent recovery criteria for SEC-18. QEMU completion
and physical-platform qualification have separate acceptance gates.

Source and fixture comments now accurately describe the compiled development
root: a signing-file override does not provision or replace bootstrap trust.
The guide documents the new commands and their limits. The report index links
the previously unlisted October 7 remediation batches.

## Validation

- Full `scripts/run-host-tests.sh`: passed, including signing-policy rejection,
  boot-result parsing, standalone kernel storage tests, all discovered host crate
  tests, signer tests and its cryptographic/format self-test.
- Trust tests: **8 passed**, including six new production-candidate regressions.
  Signer tests: **10 passed**, including three new policy/file-boundary tests.
  Negative cases cover every fixture in every role, sign/conversion aliases,
  shared roots, weak/mixed-order points, non-contributory/aliased recipients,
  cluster mismatch, revision floor/zero/overflow boundary, truncated/reserved
  wire fields, oversized or private-length inputs, existing output, symlinks,
  directories and FIFOs.
- Host Clippy for `charlotte-launch` and `cluster-sign`, all targets,
  `--locked`, `-D warnings`: passed.
- Direct `CATTEN_TRUST_MODE=production` kernel check: rejected by the expected
  build-script trust gate, confirmed by its specific diagnostic. The new
  candidate API does not bypass production refusal.
- Bundled AArch64 services: `scripts/build-catten-services.sh --embed`: passed.
- x86 kernel Clippy, `--locked`, `-D warnings`, with its required service-bundle
  environment: passed. An initial invocation omitted that environment and failed
  at the existing `include_bytes!` paths; the configured rerun passed.
- Arm kernel Clippy, `--locked`, `-D warnings`, with its required service-bundle
  environment: passed.
- `cargo fmt --all -- --check`, `git diff --check` and local documentation-link
  checks: passed.

Logs: `/private/tmp/charlotte-trust-preflight-host.log`,
`/private/tmp/charlotte-trust-preflight-clippy-{host,x86,arm}.log` and
`/private/tmp/charlotte-trust-preflight-services.log`. The final signer tests
are in `/private/tmp/charlotte-trust-preflight-signer-final.log`, and the direct
production rejection in `/private/tmp/charlotte-trust-preflight-production-rejection.log`.

## Scope limits and next gate

The candidate is **not authenticated trust**, proof of private-key secrecy,
protected provisioning or rollback-resistant state. The caller can supply a
lower revision floor; it cannot use that check to establish freshness. Same-
revision substitution requires binding authenticated exact policy bytes/digest.
The public-key checks cannot prove how an independently supplied private key was
generated or stored. No runtime launch gate treats a candidate as authorization.

Production builds remain disabled. The development policy and wire format remain
usable on their existing paths. QEMU guests were not rerun for this offline
preflight batch; it adds no runtime reset, teardown or production-boot behavior.
The existing guest evidence stays associated with its own recorded revisions.

Next SEC-04 gate: a protected bootstrap root must authenticate the executable
boot chain and policy, and a privileged custody boundary must provide the
recipient key with authenticated, rollback-resistant enrollment/rotation state.
Only then can node/operator authentication and encrypted transport (SEC-08/10)
be integrated under a provisioned root. SEC-09's authenticated time remains a
separate prerequisite for secure expiry/freshness enforcement. SEC-04/07/18
remain partial; SEC-08/09/10/12 remain open.
