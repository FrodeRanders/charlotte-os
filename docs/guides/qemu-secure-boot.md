# Disposable QEMU Secure Boot tests

This x86 QEMU fixture authenticates Limine, its configuration, the kernel and a
signed public policy module against freshly enrolled **test** firmware authority.
It exercises the kernel's consuming policy handoff. It supplies no production
enrollment, rollback-resistant storage, recipient custody or physical-platform
qualification. `CATTEN_TRUST_MODE=production` still rejects.

## Prepare and run

Prerequisites: Python 3.11+, C compiler, CMake, OpenSSL development libraries,
QEMU x86 with Secure Boot edk2 code and a matching empty vars template, mtools
and the repository Rust toolchain. Stage the usual x86 service bundle first:

```sh
scripts/build-catten-services-x86_64.sh --embed
scripts/prepare-secure-boot-tools.sh

python3 scripts/run-secure-boot-test.py \
  --firmware-code /opt/homebrew/share/qemu/edk2-x86_64-secure-code.fd \
  --vars-template /opt/homebrew/share/qemu/edk2-i386-vars.fd
```

Firmware paths are platform-specific; pass the matching files from the host's
QEMU/edk2 package. `--tools-dir` selects an alternative prepared test tool tree;
`--cases` selects comma-separated cases and `--timeout` bounds each VM. The
default runs the full matrix. `--output` must name a new directory.

Preparation installs only under `target/secure-boot-tools`. The Python tools
and all transitive packages use pinned versions and wheel SHA-256 allowlists.
osslsigncode 2.14 source has a fixed SHA-256, checked before extraction/build.
The host Limine utility is compiled from the verified vendored source. Tests
perform no downloads and install nothing system-wide. On non-Homebrew hosts,
`CATTEN_TEST_OPENSSL_ROOT` can select an OpenSSL development prefix.

## Chain and fixture boundaries

The harness generates separate disposable RSA certificates for firmware PK,
KEK and the EFI signer in `db`, plus an untrusted signer. It enrolls only those
explicit authorities into a copy of the supplied vars template, enables Secure
Boot, and uses Q35 SMM-protected pflash. Every case starts with a fresh copy of
that enrolled state. The host owner and supplied firmware/template/tooling are
trusted inputs to this VM test; they are not a protected production installer.
The fixture deliberately adds foreign KEK/db authority to its starting template,
then clears PK/KEK/db before enrollment. The foreign-loader rejection proves that
inherited trust was removed; existing template revocations are preserved.

The generated configuration hashes the exact kernel and policy-module bytes
with BLAKE2B-512, enables hash-mismatch panic and disables editing. Its own hash
is enrolled into a copy of Limine **before** EFI signing. Signing the stock
unenrolled loader would leave its configuration/kernel policy unprotected.
This follows [Limine 12.6.0's Secure Boot contract](https://github.com/Limine-Bootloader/Limine/blob/v12.6.0/USAGE.md#secure-boot)
and [hashed-path format](https://github.com/Limine-Bootloader/Limine/blob/v12.6.0/CONFIG.md#paths).

The test builds `catten` with `acpi,boot_trust_test`, snapshots its exact bytes
for the whole matrix, and checks its executable/read-only trampoline sections.
This feature supports only the x86 fixture. It requires exactly one Limine
module named `charlotte.boot-trust-test`, exactly 328 bytes long, and verifies
its `CBTRUST1` signature/cluster against compiled public test authority. Expected
revision 7 and its exact digest are pinned in the test kernel. A valid successor
is rejected as **uninstalled**, since boot consumes installed state rather than
committing a new policy revision. There is no enrollment or development fallback
when a module/state check fails.

The selected recipient is the publicly known test value `[77; 32]`. The public
policy fixtures are reproducible from seeds documented by the shared-protocol
host test; these are not production credentials. Initial platform-service loading
still uses development artifact roots. The compiled test expectation is not
persistent anti-rollback state: replacing the whole signed old boot chain/store
remains outside this fixture's protection.

Private certificate keys and mutable VM images live in one temporary owner
directory with mode 0700; keys have mode 0600. Every VM is terminated and reaped
on success, rejection, timeout or interruption. Temporary keys/images are removed
when the owner exits. Retained evidence contains public certificates, enrollment
metadata, configuration, digests and logs, plus `results.json` only when every
selected case passes. Existing evidence directories are never overwritten.

## Required execution evidence

| Case | Required outcome |
| --- | --- |
| Valid Intel VT-d / AMD-Vi | Signed module selected, installed public policy/digest agree, boot-trust assertions run and the full 15-check kernel suite passes. |
| Unsigned / foreign-signed / altered EFI | Firmware rejects the USB boot image with `Access Denied` or `Security Violation`; kernel entry never occurs. |
| Altered configuration | Enrolled config checksum mismatch; kernel entry never occurs. |
| Altered kernel / policy module | Limine rejects the appropriate URI's BLAKE2B digest before kernel entry. |
| Bad policy signature inside newly signed outer packaging | Kernel reports `InvalidSignature` before trust/service publication. |
| Correctly signed old / conflicting current policy | Kernel reports `RevisionRollback` / `RevisionConflict` before publication. |
| Correctly signed direct successor | Kernel reports an uninstalled revision before publication. |
| Missing / duplicate module in authorized configuration | Kernel rejects absent or ambiguous module identity before publication. |

The inner-policy cases deliberately authorize the changed outer configuration
under the test EFI signer, retaining the same pinned kernel, to test kernel
signature/state rejection independently of loader byte-integrity checks. Silence,
timeout, QEMU startup failure, an unrelated panic or a rejection marker followed
by kernel/publication evidence cannot pass. Host parser tests cover these rules.

The [SEC-04 roadmap](../reference/security-remediation.md) and
[bootstrap policy contract](../reference/bootstrap-trust-policy.md) retain the
missing protected installation, production roots, custody and recovery work.
