# Current-only deployment descriptor protocol

First batch of the [API/protocol simplification sweep](../../reference/api-simplification.md).
Only `CDEPLOY5`, version 5, with its exact 182-byte header is accepted. Retired
v1–v4 inputs reject rather than inheriting stack/thread, shutdown or singleton
placement defaults. Current encoder output and signing grammar stay unchanged;
no magic is renamed or reused in this batch.

## Implementation

The shared deployment decoder has one straight validation path for required
execution limits, shutdown grace, placement, artifact/object names and grants.
Four decoder branches, legacy magic/header/version constants, the redundant
`format_version` result field and the legacy minimum-header alias are removed.
Placement offsets now follow the actual current fields rather than a v4 name.
Kernel/runtime descriptor submission, operations pickup and HTTP framing use
`HEADER_LEN`; no old short-header admission remains. Release encoding/decoding
and signature helpers compose that same current decoder, so a valid outer
signature cannot revive a retired nested descriptor.

`DEFAULT_SHUTDOWN_GRACE_MS` still supplies explicit built-in launch policy.
It no longer describes a compatibility parser default. No resource owner,
authority transfer, teardown receipt or cleanup context changes. The native
spawn rejection contract is documented consistently: quota rejection returns
`ThreadError::SpawnFailed` without forcibly aborting the caller. This corrects
stale explanatory text; it changes no spawn implementation.

Living architecture/reference/guides and both affected LaTeX chapters now state
one accepted descriptor. Historical audit reports remain intact. Contributor
instructions record removal-before-rename ordering, signed retired-input rejection
and coordinated callers/documentation. Existing retired deployment files and
containing release/operations envelopes require regeneration; this batch does
not erase stored data or migrate other storage formats. Current v5 bytes remain
usable.

## Regression evidence

Current signed round trips retain execution, shutdown and replica placement
checks, plus resource-bound and signature-tamper rejection. Rejection fixtures
construct all four retired layouts and sign their raw digests independently
of current signing helpers. Their Ed25519 signatures verify cryptographically,
but decode/verify/signing preparation reject each obsolete grammar. Release
encoding refuses them. Separately signed, canonically framed release envelopes
containing those same obsolete descriptors also reject.

Header probes reject mismatched magic/version/length and every truncated header.
HTTP probes reject retired header-size bodies before waiting for more input,
and preserve current-floor framing; body authenticity remains later validation.
Existing signed-policy, release, operations-bundle, catalog, runtime ownership
and ingress tests remain enabled.

## Validation

Validated on 2026-10-13 from base commit `ef071236` plus this batch:

- `scripts/run-host-tests.sh`: 392 passed, zero failed, four ignored
  documentation examples; signing-policy, boot-result and syscall-emitter
  script probes and the signing-tool self-test also passed. Four old-format
  acceptance tests were replaced with two current/rejection tests and one HTTP
  test, accounting for the net change from 393 to 392.
- Strict Clippy passed for bundled AArch64 services and both native kernels.
  Signed service bundles rebuilt through `scripts/build-catten-services.sh`
  for both architectures. `cargo fmt --all -- --check` and `git diff --check`
  passed; all 146 local links in changed Markdown resolved.
- The tracked manual rebuilt successfully with `pdflatex`; its final two
  passes follow a resource-field wrapping correction. The PDF has 208 pages.
  Rendered pages 135 and 151–152 were visually inspected; affected protocol
  and resource text is readable without clipping. Existing unrelated manual
  overfull-box diagnostics remain outside this batch's layout scope.

Every guest run used fresh storage, a 180-second limit and the rebuilt signed
service bundle. The AMD run reused the Intel ELF only after its SHA-256 matched
the captured Intel boot artifact. Final results:

| Run / relevant flags | Deferred self-tests | Additional execution result |
| --- | --- | --- |
| ARM `--security-test --http-test --dhcp-test` | 21 passed, 0 failed, 0 pending | Both security generations passed `checks=0xffff`; HTTP/keyhole and malformed/oversized/invalid-body, EOF, abortive-peer, idle/trickle deadline recovery passed. |
| x86 Intel `--http-test --dhcp-test --iommu intel` | 18 passed, 0 failed, 0 pending | Deployment ingress framing and recovery probes passed. |
| x86 AMD `--http-test --dhcp-test --iommu amd` | 18 passed, 0 failed, 0 pending | Deployment ingress framing and recovery probes passed. |
| ARM `--shutdown-ingress-test` | 19 passed, 0 failed, 0 pending | Signed node shutdown reached PSCI power-off. |
| ARM `--deployment-ingress-test` | 20 passed, 0 failed, 0 pending | RustFS TLS object round trip and signed upload/atomic-release/pull/launch/readiness passed. |

The signed-release integration accepted release `greet-ingress-e2e` at generation
1. The guest fetched and verified 96,440 bytes, launched `greet` at generation 1
and published service generation 1; host status reported `state=ready`,
`desired_replicas=1` and `ready_replicas=1`. Its first host attempt observed
`Connection refused (os error 61)` before listener readiness; the fixture's
bounded startup retry then succeeded. Recovery probes also emitted bounded
`ConnectionResetError` retries. These are recorded transient observations,
not evidence of accepted-request replay or established transport reliability.
No guest run failed in this batch. Earlier intermittent scheduled-close,
ingress and DMA observations remain unresolved as recorded in
[completion-wait evidence](2026-10-12-completion-wait-outcomes.md); these passes
do not establish their causes or erase their failed samples.

SHA-256 identities captured for the artifacts actually executed/published:

| Artifact | SHA-256 |
| --- | --- |
| ARM security/HTTP/DHCP kernel | `d6822b7270ec64999173953450640c4dbb610c7bfd9d69a3a4e0ae7bcf6dee46` |
| Intel/AMD HTTP/DHCP kernel | `ab1dd7252dca304725107cf450d8cd420ad366852671bb414c2496b4702c06e2` |
| ARM shutdown kernel | `3dc7f0b6c7d787be91b2cfa6a8c2bca94e3efb5173cec0eea9f35422ba9ee798` |
| ARM signed deployment-ingress kernel | `022e000cd855b45c866510adb0d200f138405e1515a0b1008b1dc5a0796ec924` |
| ARM signed bundle, 41 ELFs | `a70bbbea5dea586367e498d500e6fdd8fd9a06a46e13248079d91156739171d0` |
| x86 signed bundle, 37 ELFs | `6a0c895bbceea7e47300002059fce53506bd549fb1cff083ef3e54bf1aab567c` |
| Integration `greet.cdep` | `5fc2bb845855ee5ae93ad926bdf3edc88a0ae449ac9653b0d9bde1c46d9e2cab` |
| Integration `greet.crelease` | `217f29a293c43ce63d24b56526d5009e0c091a2263fc8a314107530526053b5c` |
| Rebuilt manual PDF | `6dea1d9eed5f381ae68af88d91ecd5a548bc2779a095f6b5cf1871c146bee0d6` |

Bundle identity hashes the sorted, newline-terminated
`basename:ELF-SHA256` lines. Evidence is recorded inline above; temporary run
logs, symbol captures, regenerated fixtures and build outputs are not published
evidence dependencies. These QEMU fixtures use public development trust, not
production enrollment.

## Remaining sweep

The next candidates are discovery payload compatibility, Raft persistence
migration and duplicate deployment/IPC APIs. Their active callers and fixtures
still need coordinated migration; this first batch is not the whole sweep.
Hardware support, external protocol negotiation and serialized kernel owner
contexts stay distinct from internal compatibility. Security remediation
finding counts and the remaining production trust/transport boundaries are
unchanged.
