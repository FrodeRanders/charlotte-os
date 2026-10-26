# Dependency advisory gate

CI runs `python3 scripts/check-dependency-advisories.py` on the complete root
`Cargo.lock`. Install the scanner version in [`.cargo/audit-version`](../../.cargo/audit-version)
with `cargo install --locked --version <version> cargo-audit` from outside the
repository, then run the same command locally. Installing outside the repository
avoids its freestanding build configuration; scanning runs from the repository
root to select [`.cargo/audit.toml`](../../.cargo/audit.toml).

The project policy takes precedence over a developer's global policy. It enables
all architectures/OSes, every severity, unmaintained/unsound/notice categories,
yanked-package checks and fresh database/registry fetches. There are no ignores.
The wrapper accepts no arguments that could narrow this policy, verifies the
scanner version and propagates scanner/network/configuration failures. The
RustSec database cache is `target/security-audit/advisory-db`.

A zero scanner status also requires a complete JSON report: no vulnerabilities,
no warnings (including notices), exact unfiltered settings and database identity.
This matters because cargo-audit 0.22.2 can report notices without failing its
own `--deny warnings` check. Malformed/empty success output rejects too. These
checks supply no production trust or authority to ignore a finding.

The scanner pin and policy are version-controlled build-validation inputs,
separate from kernel resource capacity or boot configuration. Tool changes must
requalify report semantics and regression controls. Database contents deliberately
refresh at each gate invocation, so the same lockfile can fail after a new
advisory is published. Preserve the exact lockfile hash, scanner version and
report's database commit with an audit result. There is no stale/offline success
mode in the production gate.

## Current findings: 26 October 2026

The [follow-up and evidence](../reports/audits/2026-10-26-security-dependency-gate.md)
record a fresh scan at RustSec commit
`115e418ee9a2ff542fee41db641128fa84483383` (1355 advisories).
`ctutils` moves from 0.4.2 to patched 0.4.4 for
[RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html).
This is the only package/version/checksum change.

**The new CI check currently fails** on
[RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html):
`crypto-bigint` 0.5.5 remains through `elliptic-curve` 0.13.8 and the TLS P-256/
P-384 stack. The advisory lists patched versions from 0.7.6; this is not a
compatible lockfile-only upgrade of that dependency chain. No ignore, custom
arithmetic backport or unverified exception is installed. Migrate and qualify
that stack before expecting a clean advisory job. Production trust remains
disabled; functional tests do not prove absence of side channels.

The selected migration target is the pinned **Embassy-based embedded-tls main
branch**. The [assessment and reproducible probe](../reports/audits/2026-10-26-crypto-bigint-embassy-migration.md)
compile its crypto stack on both service targets, exercise six host controls and
scan a candidate graph containing crypto-bigint 0.7.10 without findings.
Cutover still requires a domain-owned entropy driver, the new verifier/mTLS API,
an erasing AES-GCM context and complete TLS/native qualification. The isolated
candidate is not the active root lockfile; the production gate remains red.

CI executes the gate on its existing push/PR triggers. The job must be deployed
and made required through repository branch-protection settings to enforce merge
policy; this source change does not configure those external settings. Periodic
scans, license enforcement, protected release provenance and third-party source/
compiler/platform qualification remain separate SEC-14 work.
