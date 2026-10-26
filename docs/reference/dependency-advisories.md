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

The [earlier follow-up](../reports/audits/2026-10-26-security-dependency-gate.md)
introduced the gate and patched ctutils 0.4.2 to 0.4.4. Its remaining
[RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html)
finding required replacing the TLS curve API family, rather than changing a
single transitive lockfile version.

The [Embassy migration](../reports/audits/2026-10-26-security-embassy-tls.md)
now installs the pinned main-branch TLS/backend, domain-owned entropy driver,
typed P-256 mTLS parser and erasing GCM context in the **active workspace**.
The root lockfile contains one crypto-bigint, **0.7.10**, and ctutils **0.4.6**.
No advisory ignore, local arithmetic backport or target exception is installed.
The [TLS reference](tls-client.md) records exact source pins and ownership rules.

**The migrated complete root graph passes the advisory gate.** Its fresh
182-package scan at RustSec commit `a78282a1414d6c28cc228c4eae3570451affb0dc`
(1378 advisories) has zero vulnerabilities and warnings. This is a point-in-time
result, not a guarantee about future database contents or constant-time code
generation. SEC-14 remains partial and production trust stays disabled.

CI executes the gate on its existing push/PR triggers. The job must be deployed
and made required through repository branch-protection settings to enforce merge
policy; this source change does not configure those external settings. Periodic
scans, license enforcement, protected release provenance and third-party source/
compiler/platform qualification remain separate SEC-14 work.
