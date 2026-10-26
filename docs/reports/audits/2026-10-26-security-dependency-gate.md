# SEC-14: dependency advisory gate and ctutils correction

Date: 2026-10-26. Reviewed baseline: `0a79a800`.

A fresh dependency scan found two current cryptographic advisories absent from
the older published snapshot. `ctutils` is now patched from 0.4.2 to 0.4.4.
A new pinned, fail-closed advisory CI job checks the complete lockfile and rejects
warnings, including notices the scanner can otherwise report with success.
**The new job intentionally remains failing on `crypto-bigint` 0.5.5 in the TLS
curve stack.** No ignore, target filter, local arithmetic backport or exception
conceals that remaining dependency finding.

The [tracked evidence](evidence/2026-10-26-security-dependency-gate.json) preserves
before/after scan reports, exact lockfile/database/scanner/source/artifact hashes,
regression controls, preliminary runs and failure diagnostics. SEC-14 remains
partial; numbered finding dispositions stay **29 corrected / 6 partial /
3 open** and production trust stays disabled.

## Fresh findings and bounded correction

RustSec commit `115e418ee9a2ff542fee41db641128fa84483383`, dated
2026-10-26, contains 1355 advisories. The baseline complete 180-dependency lockfile
scan reports:

| Advisory | Selected version/path | Result in this batch |
| --- | --- | --- |
| [RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html) | `ctutils` 0.4.2 through digest/HMAC/HKDF and Poly1305 in HPKE | Updated to patched 0.4.4. |
| [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html) | `crypto-bigint` 0.5.5 through `elliptic-curve` 0.13.8, ECDSA/P-256/P-384 and embedded-tls | Remains reported; new CI job fails. |

The `ctutils` advisory concerns secret-dependent branches in mask generation and
`u128` comparisons. Its patched release changes mask conversion and verifies
upstream code generation; the dependency's changelog identifies both fixes.
Only that package's version and checksum change in `Cargo.lock`. No dependency
source, cryptographic arithmetic, application ABI, resource limit or ownership/
physical-completion rule is locally rewritten. Local functional tests and target
builds do not independently prove constant-time code generation on our platforms.

The `crypto-bigint` advisory lists patched versions from 0.7.6, while this TLS
stack requires the older API series. That is not a compatible lockfile-only
update. Migrating the stack, its key/cipher ownership contracts and TLS/mTLS
interop needs separate qualification. This batch does not decide that its
operations are unreachable or accept a timing-risk exception. The job's current
failure is an outstanding security dependency, not an unexplained test incident.
The historical clean scan remains a point-in-time record, without a claim that
its older database contained these October 23 advisories.

## Enforcement and configuration

The CI job reuses the repository's pinned checkout/toolchain actions. It
installs cargo-audit **0.22.2** with `cargo install --locked`, from outside the
repository's freestanding Cargo configuration. The version is centralized in
[`.cargo/audit-version`](../../../.cargo/audit-version); both CI and the runner
read it. Project-local [policy](../../../.cargo/audit.toml) takes precedence over
a developer's global scanner policy.

`check-dependency-advisories.py` accepts no narrowing arguments. It runs from the
repository root, selects the complete root lockfile and explicit official RustSec
URL, requires fresh database/registry access, and keeps all architecture/OS,
severity and informational categories in scope. Yanked checks stay enabled.
No stale/offline success or ignore option is exposed by this runner. Tool version,
process, network and configuration failures reject rather than produce a pass.

A zero scanner status still requires a complete JSON report with exact unfiltered
settings, no vulnerabilities, no warnings of any category, and database identity.
Malformed/empty output rejects. This extra check has a real regression control:
cargo-audit 0.22.2 returns **0** for the `plutonium` RUSTSEC-2020-0011 notice even
with `--deny warnings`; the gate's validator rejects that report. An actual
`ansi_term` unmaintained warning and vulnerable `crypto-bigint` fixture reject too;
a clean local fixture passes.

The [reference](../../reference/dependency-advisories.md), Configuration guide
and manual describe the inputs and current blocker. Fresh database contents
are intentionally variable: later advisories can reject the same lockfile.
Evidence records the exact database commit and lockfile hash for each scan.
This workflow source does not deploy CI, modify branch protection or mark the
job required on a remote repository. Periodic scanning, license policy and
protected release provenance remain separate SEC-14 work. CI's install step and
remote job were not executed here; the installed matching scanner and runner
were exercised locally.

## Controls, failures and host validation

The new harness passes **12 tests**: eight gate/policy controls and four actual
scanner/database controls. It checks fixed root/lockfile selection, version and
argument rejection, propagation of findings/tool failures, malformed successful
reports, notices, narrowed settings and the pinned CI install. The real fixtures
use the freshly captured database without a second fetch and omit live yanked
lookup for deliberately old fixture versions. Production scans keep both
fetching and yanked lookup enabled. The default host-driver hook passes eight
controls and skips the four explicitly enabled scanner/database cases; it needs
no installed scanner. The full cross-workspace wrapper is not rerun.

Failures are retained separately:

- The first sandbox scan could not fetch GitHub's advisory database; its output
  is not a clean audit. The permitted scan succeeds as a scan and returns failure
  for the two actual advisories.
- The first draft policy omitted cargo-audit's required `output.quiet` field.
  The scanner stopped before scanning, and three real controls failed to parse
  its empty output. The explicit corrected field and improved diagnostic capture
  precede all passing controls. This is a corrected construction error, not an
  advisory or passing execution.
- The first signer debug run passed 55 unit tests and failed two local-listener
  fixtures with `PermissionDenied: Operation not permitted`. The permitted rerun
  passes its complete suite; no signer code or expectation was changed.

Both debug and release then pass **384 distinct runtime tests plus one doctest
per mode**: 227 service-library tests, 64 complete shared-launch tests and
93 signer tests. Shared-launch coverage includes encrypted operational envelope
round trips, bad-key/tamper rejection, signed policy/receipt checks and HPKE
owner erasure contracts. Service/shared-launch unit tests also ran together
before complete launch tests; those repeated ten launch unit tests are not
counted twice. Strict service Clippy passes for both freestanding targets.

## Native artifacts and manual

AArch64's forty services, the separate Sitas image and all 37 x86 services are
rebuilt and signed against the updated lockfile. The first AArch64 guest passed
with the old Sitas ELF still staged. That is preliminary regression evidence,
not the final complete-bundle qualification. Rebuilding Sitas changed its digest
from `fbf1b47a721d84498a64e43bc626a7f962a52d3740e3fa7e14adaeea7fa5f311`
to `a97f28fc30e98206e8693262752f674f4ef6a965519541ac58c41c32c95233ee`.
A separate AArch64 repeat qualifies the complete rebuilt bundle. The earlier
passing artifact/result and its retry diagnostics remain separately recorded.

Final fresh-storage, four-LP security/HTTP runs finish with `reason: complete`,
both security publication generations (1 and 2) passing and no failed or pending
deferred tests:

| Complete rebuilt bundle | Deferred passed / failed / pending | HTTP checks passed |
| --- | --- | --- |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 |
| AArch64 / SMMUv3, after Sitas rebuild | 20 / 0 / 0 | 14 |
| x86_64 / AMD-Vi | 18 / 0 / 0 | 14 |

That table follows final execution order and excludes the preliminary AArch64
pass. Each final host run records four `ConnectionResetError` retries: one after
EOF, one after abortive peer closes and two during the idle-client probe. Final
AArch64 also records one initial-metrics `TimeoutError`. Every affected probe
subsequently passes. Evidence retains all **thirteen final retry diagnostics**,
and the preliminary AArch64 run's separate five diagnostics (four resets and one
initial-metrics timeout). No timeout, attempt bound or quota is widened. No final
guest failed. Passing results do not establish the cause of historical scheduler,
IPC, root-close, metrics or NIC failures. These are native functional/regression
checks, not a new side-channel timing or hardware qualification.

The manual's final two-pass build has 251 pages. The table of contents and
affected/adjacent pages 14, 197–198 and 204–205 are rendered and visually checked.
There are 189 overfull-box warnings across the full build and no undefined
references; the rest of the manual is not re-audited. Workspace formatting,
whitespace, frozen source inputs, exact lockfile change and local Markdown
targets pass. Formal inputs and policy transitions are unchanged; no model run,
new timing/assembly proof or physical-platform qualification is claimed.

The new advisory check is deliberately not green while the TLS finding remains.
Functional regressions passing does not discharge that finding. Production trust,
licenses, release provenance and the other audit acceptance gates retain their
existing status.
