# Limine dependency and boot policy

CharlotteOS uses two independently versioned Limine components:

- the `limine` Rust crate, which defines the kernel-side Limine Boot Protocol
  request and response types; and
- the Limine EFI binaries under `limine-binary/`, which load the kernel and
  implement that protocol.

Both are exact pins. The kernel uses `limine` crate **0.6.5**, whose base
revision 6 request layout matches the checked-in Limine loader **12.6.0**.
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

Treat a loader update as a platform change, not routine dependency churn:

1. Read every intervening entry in Limine's `ChangeLog`, paying particular
   attention to the Limine protocol, ELF loading, UEFI, paging, MP startup,
   x86-64, and AArch64.
2. Update `limine-binary/VERSION` to an immutable release URL and the archive
   digest reported by that release.
3. Extract the official binary archive and replace only its payload files.
   Regenerate `limine-binary/SHA256SUMS` from those bytes.
4. Run `scripts/verify-limine.sh --release`.
5. Build both target kernels and boot the complete self-test suite on x86-64,
   AArch64 `virt`, and AArch64 `--sbsa-ref`. The last target is required because
   it exercises Limine's server-shaped AArch64/UEFI path.
6. Update the known-good version recorded in the platform documentation only
   after those boots pass.

The Rust crate should be updated separately when a new protocol binding is
needed. Review its ABI/API changes and keep it on an exact version.

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
