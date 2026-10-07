# Contributor guides

These documents contain repeatable workflows rather than architectural claims.

- [Testing](testing.md) — host suites, QEMU target tests, and validation scope.
- [Kernel panic backtraces](kernel-panic-backtraces.md) — fail-stop behavior,
  build-matched symbol maps, and offline function/source lookup.
- [Limine dependency and boot policy](limine.md) — exact version pins, binary
  provenance, update validation, and measured/Secure Boot boundaries.
- [Disposable QEMU Secure Boot tests](qemu-secure-boot.md) — signed EFI/config/
  kernel/policy chain, negative boot evidence and test-only provisioning scope.
- [Userspace development](userspace-development.md) — `catten-rt`, entry
  points, launch manifests, capabilities, and service packaging.
- [Signing and development trust](signing-and-trust.md) — restricted key-file
  inputs, fixture compatibility, and the fail-closed production-image gate.
- [Userspace resource ownership](resource-ownership.md) — RAII, typed mappings,
  IPC transfer/borrowing, server replies, cancellation, and raw boundaries.
- [Cooperative shutdown](shutdown.md) — signed grace periods, lifecycle-aware
  event loops, resource teardown, and forced deployment retirement.
- [AArch64 network development](aarch64-network-development.md) — macOS TCG,
  default network services, two-guest stream LANs, and optional verifier
  commands.
