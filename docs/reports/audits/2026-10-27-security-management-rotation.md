# Management credential rotation across signed-policy restart

Date: 2026-10-27. Parent: `77e614a7`. Follow-up to
[management startup qualification](2026-10-27-security-management-startup.md).
SEC-04 and SEC-10 remain partial. The ledger remains **29 corrected in scope,
seven partial and two open**; production remains disabled.

The x86 development fixture now exercises credential replacement across an exact
installed-policy revision change. Revision 8 rejects both old operator leaves
while continuing to trust their CA. It rejects a resource bound to revision 7
and a retired policy module. Replaying the still-authorized revision 7 kernel
restores its old policy and operator access: **the fixture does not provide
platform rollback resistance**. This observed limitation is separate from the
five passing rotation/rejection cases.

## Implementation and trust boundary

`CATTEN_BOOT_TEST_INSTALLED_POLICY` accepts an absolute regular-file path only
inside the x86 development `boot_trust_test` feature. The build bounds and
structurally validates a complete current signed-policy record, staging a
separate compiled snapshot. The default remains the public revision 7 fixture.
At boot, the kernel authenticates that compiled snapshot against the pinned
fixture bootstrap key before creating its exact installed expectation. The
candidate Limine module must then pass shared `verify_for_boot` against it.
The candidate cannot choose installed state; installation-verifier acceptance
of a successor cannot publish that successor at boot against revision 7.

The host fixture first checks the signed 7-to-8 lineage with the existing
installation verifier, writes/fsyncs/renames/fsyncs its **public test snapshot**,
and reads it back before building the revision 8 kernel. These ordinary host
file operations supply no protected state, production installer, custody or
power-loss/anti-rollback guarantee. All fixture authorities and the recipient
remain public test material. Native admission still publishes one policy from
`PreparedBootTrust`; no management-specific trust slot is introduced.

The shared native TLS adapter logs a bounded `client policy rejected` event
with its existing connection ID. It neither constructs an authenticated identity
for rejection nor prints raw certificate/parser/crypto contents. The category
also includes verifier time/credential/capacity gates; it does not identify every
rejection as revocation. Host TLS handshakes test that classification separately
from missing certificates, foreign roots and wrong purposes.

## Native execution

`scripts/run-management-rotation-test.py` reuses the existing firmware-authority
preparation and credential generator. Both signed service bundles were rebuilt
through their architecture-specific builders. The x86 campaign copies/re-signs
its platform bundle with the selected public fixture artifact authority; it does
not alter the normal staged bundle.

The campaign retains one enrolled firmware-variable image and one NVMe data
image across all six boots. Only the signed loader/config/kernel/module inputs
change. The three successful boots report NVMe-backed Raft restart terms **2,
4 and 6**, respectively, and each completes **16 passed / 0 failed / 0 pending**
with bitmap `0x3bbff`. Negative boots make no self-test-success claim.

| Boot | Observed result |
| --- | --- |
| Installed 7, module/resource 7 | Both old roles read status on both interfaces; reader mutation denied |
| Installed 7, module 8 | `UninstalledRevision`, terminal panic before artifact-authority selection or operational policy publication |
| Installed 8, module/resource 8 | Both new roles read both interfaces; four old role/interface combinations rejected, with native policy events and a healthy new reader after each |
| Installed 8, module 8, resource 7 | Exact `WrongPolicy`, terminal `boot admission trust rejected`, no operational policy or steady-state publication |
| Installed 8, module 7 | Exact `RevisionRollback`, terminal panic before artifact-authority selection or operational policy publication |
| Replay old signed kernel/module/resource 7 | **Observed limitation:** old roles regain access on both interfaces despite reuse of the firmware/data images |

Revision 8 retains both client CA roots and enrolls only the replacement reader
and administrator leaves. OpenSSL independently validates all four client
chains against the combined trust store. The replacement server has a new
key/certificate/root; clients trusting both roots verify its exact DER digest,
while the old-server-root-only client reports certificate verification failure.
TLS negotiates 1.3/AES-128-GCM, and successful connections report no tickets or
session reuse. Returning the old reader's non-ticket session object cannot
preserve its enrollment. This is not a test of an issued resumable ticket.

Kernel SHA-256 values:

| Compiled snapshot / resource | SHA-256 |
| --- | --- |
| 7 / 7 | `dba3341ec2dc5ede13d49c0e94f47793427fa0f06b1790a428640e64c2b0514d` |
| 8 / 8 | `7b9be439c9db1eaa2666e718f2f4f2ac8830dd3d998c0e452ec27dc40e574860` |
| 8 / 7 | `a3c820246e09c1da7f6b9dbc688e164fd85f10453f9a04797dcd81ec298b4284` |

The [tracked evidence](2026-10-27-security-management-rotation-evidence.json)
contains concise native observations, signed-policy/resource/credential/loader/
service hashes, host validation, source hashes and failed preliminaries. No
temporary capture is needed to interpret these results.

## Failed preliminaries

These failed runs remain separate from the passing campaign:

1. Two disposable CA roots initially used the same issuer name. OpenSSL rejected
   the replacement reader with `error 7 at 0 depth lookup: certificate signature
   failure`. The campaign stopped before any kernel build. Distinct fixture CA
   names remove that ambiguous issuer selection; all four chain controls pass.
2. The image helper preserved a case-specific EFI basename, leaving no
   `BOOTX64.EFI` fallback. Firmware reported `Not Found` and `No bootable option
   or device was found`; the observation deadline failed before kernel entry.
   The corrected image uses the required UEFI fallback filename.
3. A transitional virtio NIC lacked the existing driver's required features:
   `startup rejected stage=3 error=RequiredFeatures detail=0x101`. The guest
   later panicked at `self_test/el0_net.rs:105:22`, waiting for frame-router
   startup, and recorded **15 passed / 1 failed / 0 pending**. Kernel SHA-256:
   `64b6e974c81ad7c02e375f5997b4c9a7ebedee2b5ec16b3b34ae6f860435929d`.
   The final campaign uses the existing runner's non-transitional virtio NIC
   with `iommu_platform=on`; the network test and TLS probes pass.

These are fixture preparation/configuration failures, not passing rejection
tests or evidence of a TLS authorization defect.

## Validation and remaining qualification

- **23 Rust host tests:** 11 TLS handshake/session controls and 12 shared signed-
  policy controls, including exact boot versus successor installation acceptance.
- **24 Python host tests:** eight actual build-input controls, three rotation
  observation/snapshot controls, six existing Secure Boot observation controls
  and seven real-CLI deployment-status controls using the shared TLS fixture.
- **Five existing Secure Boot native regression cases pass:** default revision
  7 Intel boot (**15/0/0**), foreign loader, uninstalled successor, invalid policy
  signature and kernel tamper. This also checks the shared enrollment refactor.
- Both bundled-service builds, strict Clippy for both actual service/kernel
  targets and the host TLS engine, native assembly-section checks, workspace
  rustfmt, Python syntax and whitespace checks pass.
- Markdown and the manual Configuration chapter document the adjustable fixture
  input and its limits. Two LaTeX passes produce 253 pages; changed/adjacent pages
  200-202 were rendered and inspected. The 190 existing global overfull warnings
  remain outside the Configuration chapter; no undefined references are reported.

The native artifacts use the final Rust sources. Subsequent host-only changes
tighten failed-suite/full-line and terminal boot-gate observation, add SIGTERM cleanup handling and
label rollback replay as an observed limitation on the console. Host observation
tests cover the stricter gates, and the final observer accepts all six captured
native outcomes against their exact expectations; other harness edits pass syntax
checks. No second full native campaign or separate SIGTERM fault injection is
claimed. Dependency versions are unchanged;
no fresh advisory scan is claimed.

Remaining work includes protected installation/custody and platform anti-rollback
(SEC-04), native expiry, ARM revision rotation, live factory/session retirement,
multi-node leader movement and broader fault/pressure qualification (SEC-10).
Authenticated time, cluster-peer authentication and full secret-state erasure
retain their separate SEC-09, SEC-08 and SEC-15 boundaries. This campaign does
not close any numbered finding.
