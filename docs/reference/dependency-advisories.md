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

## Latest recorded scan: 30 October 2026

The complete root graph passes the production gate with fresh database and
registry access. Scanner, policy and lockfile are unchanged by the scheduled
workflow addition.

| Input or result | Recorded value |
| --- | --- |
| cargo-audit | `0.22.2` |
| Cargo.lock SHA-256 | `1c320daef7b13df180c65c7c318a76746cec9f6226890c194a883e4ac4e2f1aa` |
| Dependencies | 205 |
| RustSec commit | `2295466c4e9df26647fd67d8fa1c705ac2fc7f81` |
| Database update | `2026-10-30T12:12:39+01:00` |
| Advisories | 1395 |
| Vulnerabilities / warnings | 0 / 0 |
| Gate exit status | 0 |

This is a point-in-time result, not a future scan result or a constant-time
code-generation guarantee. The [original gate](../reports/audits/2026-10-26-security-dependency-gate.md)
and [native TLS migration](../reports/audits/2026-10-27-security-management-tls-native.md)
retain earlier findings, corrections and database identities. The
[TLS reference](tls-client.md) records current source pins and ownership rules.

## Periodic operation and remaining boundaries

[Dependency advisory watch](../../.github/workflows/dependency-advisories.yml)
requests a scan every Monday at 05:23 UTC and supports manual dispatch. It runs
only the advisory job, using the same tool pin, locked installation outside the
workspace and unfiltered gate as the push/PR job. Both jobs have a 20-minute
limit and record scanner version plus hashes of the lockfile, policy, version
pin and gate before scanning. The gate prints the complete JSON report, including
database identity, into the job log. Findings or scanner/network failures fail
the job; neither workflow suppresses them or restores a cached clean result.

Scheduled execution requires this workflow on the default branch with Actions
enabled. GitHub may delay or drop scheduled runs; its schedule is not a freshness
guarantee. See [GitHub's schedule contract](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule).
Local validation covers YAML and the gate's regression controls, not a hosted
workflow run or repository settings. No hosted activation is claimed here.

The existing push/PR check still needs deployment and required-check enforcement
through repository settings. License enforcement, protected release provenance
and third-party source/compiler/platform qualification remain separate SEC-14
work. SEC-14 remains partial and production trust stays disabled.
