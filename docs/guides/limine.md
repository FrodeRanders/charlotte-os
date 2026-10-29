# Limine dependency and boot policy

CharlotteOS uses two independently versioned Limine components:

- the `limine` Rust crate, which defines the kernel-side Limine Boot Protocol
  request and response types; and
- the Limine EFI binaries under `limine-binary/`, which load the kernel and
  implement that protocol.

Both are exact pins. The kernel uses `limine` crate **0.6.5**, whose base
revision 6 request layout matches the checked-in Limine loader **12.9.6**.
Do not turn either pin back into a lower-bound dependency: Limine crate 0.x
releases may change their Rust API, while loader releases can change the
architecture-specific entry state.

## Binary provenance and verification

`limine-binary/VERSION` records the official release URL, release archive
SHA-256, and upstream signing-key fingerprint. `limine-binary/SHA256SUMS`
records every file copied from that archive.

The normal boot-image builders call the offline verifier automatically. It can
also be run directly:

```sh
scripts/verify-limine.sh
```

For a provenance check against a fresh download of the official release:

```sh
scripts/verify-limine.sh --release
```

To restore missing or modified vendored files from that digest-verified
archive:

```sh
scripts/verify-limine.sh --restore
```

The archive SHA-256 is taken from the immutable GitHub release metadata. Limine
also publishes a detached signature made with the fingerprint recorded in
`VERSION`; signature verification can be added to a release ceremony when a
trusted copy of that public key is provisioned locally. The checksum remains
mandatory even when a signature is checked.

## Updating Limine

A loader update can affect the whole boot path. Check it as a platform change:

1. Read every intervening entry in Limine's `ChangeLog`, paying particular
   attention to the Limine protocol, ELF loading, UEFI, paging, MP startup,
   x86-64, and AArch64.
2. Update `limine-binary/VERSION` to an immutable release URL and the archive
   digest reported by that release.
3. Extract the official binary archive and replace only its payload files.
   Regenerate `limine-binary/SHA256SUMS` from those bytes.
4. Run `scripts/verify-limine.sh --release`.
5. Build both target kernels and boot the complete self-test suite on x86-64,
   AArch64 `virt`, and AArch64 `--sbsa-ref --no-network`. The last target is
   required because it exercises Limine's server-shaped AArch64/UEFI path; its
   current QEMU machine does not expose the default virt networking path.
6. Update the known-good version recorded in the platform documentation only
   after those boots pass.

The Rust crate should be updated separately when a new protocol binding is
needed. Review its ABI/API changes and keep it on an exact version.

### 12.9.6 update (2026-10-29)

The loader was updated from 12.6.0 to the latest immutable upstream release,
[12.9.6](https://github.com/Limine-Bootloader/Limine/releases/tag/v12.9.6).
The complete intervening [changelog](https://github.com/Limine-Bootloader/Limine/blob/v12.9.6/ChangeLog)
was reviewed. Relevant changes include x86 AP/APIC startup fixes, AArch64
non-global and execute-protected mappings, a fix for the highest memory-map
entry being omitted from page tables, UEFI graphics/config parsing fixes, and
disabling the UEFI Shell shortcut under enrolled Secure Boot. Existing configs
already place global options before their first entry, as the updated parser
requires. The revision 6 Rust binding remains pinned to 0.6.5.

The official `limine-binary.tar.xz` SHA-256 is
`939f670e0831cc940453fd3a0ddfb2b324c0b4e3c5d0e741c190aa0d13df0103`,
checked against the immutable GitHub release asset metadata. All 16 upstream
payload files, including the new third-party license notices, are covered by
`SHA256SUMS` and were compared byte-for-byte with a second fresh download using
`scripts/verify-limine.sh --release`. This verifies the published digest;
the detached GPG signature was not independently verified.

The host C99 utility builds and reports 12.9.6, both boot-image packaging
integration tests pass, and the offline verifier rejects a one-byte EFI
alteration. Both target kernels were rebuilt along with their signed service
bundles. Four-LP boots with fresh NVMe storage passed:

| Runner (`--timeout 180 --fresh-storage`) | Authoritative result | Kernel SHA-256 |
| --- | --- | --- |
| `scripts/run-aarch64.sh --instance limine1296-virt` | 17 passed, 0 failed/pending; bitmap `0x1ffff` | `e17c0207b545a0bbc1a49adeea659cf6a94405e6e024eabd84250df74b61da7c` |
| `scripts/run-x86_64.sh --instance limine1296-x86` | 15 passed, 0 failed/pending; bitmap `0x1bbff` | `55983e2bdcfea1b615df345d47ce9de910e337f7e37e71e00ba7f9dfff63d89e` |

The first launch attempt was blocked by the host sandbox's QMP socket policy
before guest entry; the successful run used normal host socket access.

All 16 disposable Secure Boot cases passed across two runs of
`scripts/run-secure-boot-test.py --timeout 180` with Homebrew's
`edk2-x86_64-secure-code.fd` and `edk2-i386-vars.fd`. The initial run passed
valid Intel/AMD boots, unsigned/foreign/tampered-loader rejection,
config/kernel/policy tamper rejection, and policy signature/rollback rejection.
It then stopped at `policy-conflict` with `kernel panicked for an unrelated
reason`; the captured diagnostic ended at
`[boot trust fixture] verification rejected: RevisionC` (serial SHA-256
`46156f12909f3547196e9a9fe965d628ed4400af548091da599c47715f8694b7`).
That incomplete capture was not counted as a pass; its cause is unconfirmed.
A separate run selecting `policy-conflict,policy-next,missing-module,duplicate-module,platform-ns-root,platform-driver-root`
passed all six cases with their exact expected rejection messages. No complete
single-run matrix pass is claimed.

Both Secure Boot runs used the same selected-root kernel BLAKE2B-512:
`7e64aa520b7aee9f645ae0c6dc2602e31bbe59390a84e325d96eca33e5c4c80ec082155739a21152b976d927d9cddd1788c760d0b1ba0969298664e862689b67`.
The firmware code SHA-256 was
`32807682a9e5c0e2d192ecb6077941d4be8bcfa6b9cd6e60830c3a9f48c0555a`;
the second run's enrolled/signed loader SHA-256 was
`b56202032187c6aa64d35dc9cb6fb9ff45010c0366ea262dd974bc40ca09ab5a`.
All certificates were disposable test authority, and both runs removed their
private keys and mutable VM artifacts on exit.

SBSA qualification was completed in the follow-up investigation on the same
date. The initial firmware build failed in `VfrCompile` (`redefinition of
'act2'`, expected semicolon and extraneous closing brace): macOS Make 3.81
did not enforce edk2's grouped generator targets. Rebuilding with GNU Make
4.4.1 passed all 318 BaseTools tests and produced the firmware. The build
script now requires grouped-target support, pins all firmware source commits,
prepares standard edk2 configs and records artifact provenance.

The first guest boot exposed the runner's FAT boot disk to the object store,
which correctly rejected it. Separating the USB boot volume from NVMe data
then exposed a kernel LPI geometry regression: IDBits 11 excluded all LPIs,
whose first INTID is 8192. IDBits 13 with two contiguous property-table pages
preserves bounds and permits delivery. With these fixes,
`scripts/run-aarch64.sh --sbsa-ref --no-network --fresh-storage --timeout 240`
passed all 17 registered tests (bitmap `0x1ffff`, zero failed/pending), including
76 MSI-X completions, fault-free DMA and Raft restart recovery. A `virt`
regression boot also passed all 17 tests with the same corrected kernel SHA-256
`0965f06694a34c2e820f8f0f99601097da894e43659bf86e3bacdaab269cb4b3`.
This qualifies the existing EL1/TCG boot path. See the
[SBSA investigation](../platforms/sbsa-ref.md#2026-10-29-build-and-current-suite-requalification)
and its [version-controlled evidence](../reports/audits/evidence/2026-10-29-sbsa-limine.json)
for distinct failed attempts, exact source revisions and artifact hashes.

## AArch64 EL2 status

Limine 12.6.0 substantially reordered and hardened its AArch64 page-table
handoff, including the EL2/VHE path, relative to the previously vendored
12.2.0. It also synchronises the instruction stream after loading executables.
Those are relevant improvements to the area identified during `sbsa-ref`
bring-up.

They do not make QEMU TCG a trustworthy VHE validation environment. The
CharlotteOS `sbsa-ref` recipe therefore continues to have TF-A enter the UEFI
boot chain at EL1. Removing that firmware adaptation requires a separate
EL2-entry test on KVM or real AArch64 hardware; a successful EL1 boot is not
evidence for that path.

## Development, measured boot, and Secure Boot

`limine.conf` is the normal development configuration. It deliberately does
not claim Secure Boot: its kernel path is unhashed and the checked-in EFI
binary has no CharlotteOS config hash or signing identity enrolled.

`limine-measured.conf` opts into Limine measured boot. Use it without modifying
the repository default:

```sh
CATTEN_LIMINE_CONFIG=limine-measured.conf scripts/run-aarch64.sh
CATTEN_LIMINE_CONFIG=limine-measured.conf scripts/run-x86_64.sh
```

Measurements occur only when firmware exposes a TPM2 TCG2 or confidential-
computing measurement protocol. They change PCRs 8 and 9, so this is opt-in
rather than a silent default.

Secure Boot packaging is deployment-specific because it requires a trusted
private key and certificate. A correct image must:

1. append the kernel's BLAKE2B-512 digest to `KERNEL_PATH`;
2. enroll the resulting config's BLAKE2B-512 digest into a copy of the
   architecture's Limine EFI executable using `limine enroll-config`;
3. sign that modified executable with the platform-enrolled key; and
4. keep the private key outside the repository and build artifacts.

Signing the stock EFI file without enrolling and hashing the configuration
does not give Limine's kernel/config integrity policy. Conversely, enrolling a
config hash without signing the resulting EFI binary does not establish a
firmware trust chain.

The [disposable QEMU Secure Boot matrix](qemu-secure-boot.md) now generates that
chain under test-only firmware certificates and checks loader/config/kernel/
policy substitution and kernel policy-state rejection. Ordinary boot defaults
remain development configuration; the test does not enable production.
