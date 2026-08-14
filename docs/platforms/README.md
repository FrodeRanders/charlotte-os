# Platform status

These are living bring-up notes for concrete machine targets. A statement in
one platform note does not imply equivalent support on another architecture.

- [AArch64 QEMU `virt`](aarch64.md) — mature development target, current
  capabilities, limitations, and boot evidence.
- [QEMU `sbsa-ref`](sbsa-ref.md) — server-shaped UEFI/ACPI/GIC/ITS/SMMU/NVMe
  bring-up and firmware findings.
- [x86-64](x86_64.md) — current boot path, implemented pieces, and remaining
  parity gaps.

The cross-platform path to physical ARM servers is maintained separately in
the [real-hardware roadmap](../architecture/real-hardware-roadmap.md).
