# QEMU Secure Boot chain and signed-policy consumption

Date: 2026-10-08. Baseline: `9d185168`, following the
[one-shot kernel handoff](2026-10-07-security-boot-trust-handoff.md).
**SEC-04 remains partial; no finding is closed by this batch.**

## Implementation

The new disposable x86 fixture runs QEMU's Secure Boot edk2 firmware with
Q35 SMM-protected pflash and separate temporary PK, KEK and EFI/db certificates.
It deliberately seeds foreign KEK/db authority into its starting vars template,
then clears PK/KEK/db before enrolling its own authority, preserving existing
revocations. Each VM receives a fresh enrolled-store copy. The host owner,
firmware/template and prepared tools remain trusted test inputs.

The generated Limine configuration hashes the kernel and policy module using
BLAKE2B-512, requires hash-mismatch panic and disables editing. Its exact hash
is enrolled into a loader copy before Authenticode signing. The host verifies
that signature before boot. The kernel is snapshotted once for the matrix so
later Cargo feature builds cannot change the bytes used by its cases.

The `boot_trust_test` kernel feature is explicitly restricted to x86 QEMU and
development mode. It requires exactly one named Limine module, checks its exact
328-byte length, verifies `CBTRUST1` against a pinned public test bootstrap root,
cluster and expected installed revision/digest, and hands its prepared owner into
the common one-shot launch path. It has no failure fallback. The kernel's boot
assertions check that the live public policy/digest came from this signed fixture.

The shared verifier permits a direct successor for installation. Boot additionally
requires that the verified receipt exactly match its installed expectation;
an otherwise valid successor cannot publish without corresponding installed
state. That distinction is exercised with an authentic signed successor, not a
corrupted record. Correctly signed rollback and current-revision conflict fixtures
are also regenerated/verified by the shared-protocol host test.

Module backing stays borrowed from the Limine ABI while verifying/copying public
fields; it is never adopted as runtime capability ownership. Physical admission
uses only `MEMMAP_USABLE`, so inherited boot/module backing is not recycled.
The recipient is a known public test value in a zeroizing owner, and initial
platform services retain development artifact roots. No production recipient
is compiled into the fixture or read from a caller's private key file.

The host harness owns each VM through termination/reaping, including timeout or
interruption. One restricted temporary directory owns all generated private keys
and mutable disk/vars artifacts. These are removed on exit; public certificates,
enrollment/configuration/digests and logs remain. Evidence output must be new and
the summary is written only after every selected case passes.

Host tooling is prepared inside `target/secure-boot-tools`: osslsigncode 2.14
source has a fixed verified SHA-256; virt-firmware 26.9 and all transitive Python
packages have pinned versions and published wheel-hash allowlists. The host
Limine utility uses verified vendored source. Tests download/install nothing.
This pins test tooling; it does not close SEC-14's advisory/license/provenance work.

Workflow and primary-source contracts:
[QEMU Secure Boot guide](../../guides/qemu-secure-boot.md).

## Execution evidence

The **14-case** matrix requires specific failure evidence. Silence, timeout,
startup errors and unrelated panics cannot pass; neither can a rejection followed
by forbidden kernel-entry or trust-publication evidence. Four new host parser
tests cover complete-result parsing, missing signed-handoff evidence, these false
success conditions and firmware/loader/kernel failure separation.

| Cases | Required evidence |
| --- | --- |
| Valid Intel VT-d and AMD-Vi | Signed module selected; live policy/digest assertion and boot-trust tests pass; each kernel suite completes 15/15. |
| Unsigned, foreign-signed and modified EFI | Firmware reports `Access Denied` for the USB boot image; no kernel entry. Foreign rejection also proves inherited db trust was removed. |
| Modified configuration | Enrolled checksum mismatch before kernel entry. |
| Modified kernel or policy module | Limine reports the corresponding URI's BLAKE2B mismatch before kernel entry. |
| Invalid policy signature | Correctly authorized outer packaging reaches the pinned kernel, which reports `InvalidSignature` before publication. |
| Authentic older/current-conflicting policy | Kernel reports `RevisionRollback` / `RevisionConflict` before publication. |
| Authentic direct successor | Kernel reports an uninstalled revision before publication. |
| Missing or duplicate policy module | Kernel rejects absent/ambiguous module selection before publication. |

The first diagnostic parser expected firmware `Security Violation`; this edk2
build emits `Access Denied`. That run failed on timeout rather than passing from
silence. The parser now accepts either explicit USB-image rejection, covered by
host tests; the final matrix passed all cases with the corrected parser.

## Validation

- Pinned tool preparation and hash-locked offline package installation: passed.
- Full host suite: passed, including nine signed-policy tests with reproducible
  current/rollback/conflict/successor fixtures, 13 signer tests and four new
  Secure Boot result tests.
- Host Clippy and both default custom kernel targets: passed with `--locked`
  and `-D warnings`; x86 `acpi,boot_trust_test` Clippy also passed.
- Final Secure Boot matrix: **14/14 passed**, including both 15/15 kernel
  suites and all explicit firmware/loader/kernel policy rejections. Enrollment
  evidence confirms PK/KEK/db, `SecureBootEnable=1` and `CustomMode=0`.
- Final test review moved expected-policy verification outside the masking
  policy guard, capturing only the installed digest under it. Fixture Clippy
  and both positive Intel/AMD boots were rerun afterward: each **15/15 passed**.
  The change affects the positive boot assertion, not any negative policy path.
- Ordinary Intel/no-network boot: **15/15 passed**. Ordinary Arm SMMUv3 security
  boot: **19/19 passed**, with `0xffff` probe checks across generations 1/2 and
  cancellation traffic retired after 4,776 requests. Both confirm the fixture
  selector is inactive in default images.
- Final success/failure artifact checks confirm no fixture private key or
  temporary owner directory remains after those runs.
- Direct production build with `boot_trust_test`: rejected with the required
  build-script diagnostic.
- `cargo fmt --all -- --check`, Python syntax/parser tests, shell syntax,
  `git diff --check` and local documentation-link checks: passed.

Guest service source did not change; existing staged bundles are reused.

## Remaining production boundaries

This authenticates a disposable QEMU boot chain under host-enrolled test roots.
It is not protected production enrollment, persistent anti-rollback state or
recipient custody. Replaying an entire older signed boot chain/vars store remains
unaddressed; compiled expected state cannot prevent it. Initial platform-service
roots, authenticated recovery/rotation and physical qualification are unfinished.
Production remains disabled. The current ledger remains **20 corrected, six
partial, four open** across 30 findings.
