# QEMU `sbsa-ref` Bring-Up Notes

Status of getting Catten to boot on QEMU's `sbsa-ref` machine, and the
boot-firmware findings discovered along the way.

## Goal

`sbsa-ref` is a "Server Base System Architecture" reference machine that models
a real ARM server (UEFI + ACPI + GICv3/ITS, RAM starting at 1 TiB). Getting it
green exercises the discovery paths (GIC/PL011/ECAM from ACPI) that real ARM
server hardware needs, unlike QEMU `virt` where addresses are fixed.

## Boot chain achieved

```
sbsa-ref UEFI (TF-A BL1/BL2/BL31 + edk2, prebuilt SbsaQemu firmware)
  -> Limine BOOTAA64.EFI (via the UEFI Shell; ESP on NVMe)
  -> "limine: Loading executable boot():/catten"
  -> kernel load
```

The full chain to the kernel load is verified. The UEFI front page is driven
interactively (front page -> Boot Manager -> UEFI Shell -> `\EFI\BOOT\BOOTAA64.EFI`);
the firmware does not auto-boot a disk because the NV vars are fresh and the
machine has no stored boot entry.

## Kernel-side changes (committed on `bringup/sbsa-ref`)

- `bcbee45` — **ACPI discovery**: GICD/GICR bases from the MADT (GIC Distributor /
  Redistributor entries), console UART base from SPCR, with fallback to the QEMU
  `virt` geometry. `environment/acpi/sdt/discovery.rs` walks the XSDT without
  allocating (the heap is not ready during the earliest boot). GIC base
  resolution is gated behind `LazyLock` in `gic/mod.rs`.
- `e504011` — **EL2 -> EL1 descend at the kernel entry**: Limine base revision 6
  enters a kernel at EL2 with VHE when the boot firmware hands off at EL2. The
  kernel targets EL1/EL0, so `_start` now detects EL2, captures the kernel page
  tables from the VHE-redirected EL2 register bank, clears `HCR_EL2.E2H`,
  reprograms the EL1 bank, and `eret`s down to EL1h before `bsp_main`.
  Also corrected a stale "base revision 0" comment in the PL011 driver (the
  kernel requests revision 6, which Limine 12 requires on AArch64).

Virt stays fully green (18/18 self-tests) with these changes.

## The blocking finding: Limine's EL2+VHE handoff vs. QEMU TCG

### Protocol context

The kernel requests Limine **base revision 6** (`BaseRevision::new()` in
`limine-0.6.5` = `MAX_SUPPORTED`). Limine 12.x refuses any lower base revision
on AArch64 (verified: requesting 0 panics with "minimum: 6"). The protocol says
that at base revision >= 6, if the bootloader runs at EL2, the kernel is
**entered at EL2 with VHE**. `sbsa-ref`'s UEFI hands Limine off at EL2, so the
kernel is entered at EL2 with VHE — a path this EL1-oriented kernel must handle.

### The Limine bug

In `limine`'s `common/lib/spinup.asm_aarch64`, `enter_in_el2` enables `HCR_EL2.E2H`
(VHE) **before** the VHE-redirected EL1 register bank (`TTBR0_EL1`/`TTBR1_EL1`/
`TCR_EL1`/`MAIR_EL1`/`SCTLR_EL1`) holds the kernel's translation. On platforms
whose firmware leaves a small EL1-bank VA size (sbsa-ref: 39-bit) while Limine
and the kernel are loaded high in memory (sbsa-ref RAM starts at 1 TiB, 41-bit
addresses), the first instruction after enabling E2H faults with an
**address-size fault**. The VBAR recovery path also lands in unmapped kernel
region addresses, producing a fault loop before the kernel is ever entered.

This is a **latent** bug: no open Limine issue covers it, and the code is
unchanged in current Limine trunk. It only manifests when (a) the firmware hands
off the bootloader at EL2, and (b) the platform loads the bootloader/kernel at
addresses beyond the firmware's EL1-bank VA range. `virt` (firmware at EL1) never
takes the path.

### The QEMU TCG limitation

A fix was built and applied (prime the EL1 bank from the EL2 bank before enabling
E2H). It is architecturally correct, but it **cannot be validated under QEMU TCG**:
with the EL1 bank correctly primed (verified via gdb: `TTBR0_EL1` = a real table,
`TCR_EL1` = 48-bit VA), enabling E2H still makes the very next instruction fetch
fail with an address-size fault at a 41-bit address — architecturally impossible
with valid 48-bit state. The same happens with `-cpu max`. This indicates QEMU
TCG's E2H/VHE emulation does not faithfully implement the architecture (it
advertises `FEAT_VHE`/`FEAT_E2H0`, but the encoding is not honoured). The Limine
fix would need validation on real hardware or KVM.

## Firmware adaptation (chosen path)

Because the QEMU TCG E2H/VHE emulation is unusable, the EL2+VHE kernel-entry path
cannot be exercised under TCG. The chosen route is to **adapt the boot software
to enter the kernel at EL1**, avoiding E2H/VHE entirely.

Two attempts were made; both hit the same QEMU TCG wall:

- **Adapt Limine's `enter_in_el2` to descend to EL1.** Limine is built from
  source (`aarch64-elf-gcc` + `aarch64-elf-binutils` via Homebrew;
  `./bootstrap`, `./configure --enable-uefi-aarch64`, `make`). `enter_in_el2`
  was rewritten to program the EL1 bank with the kernel's page tables and
  `eret` to EL1 instead of enabling E2H/VHE. The kernel is then entered (its
  `_start` VMA is reached — a milestone), but the transition is unstable: QEMU
  TCG aliases EL1 system-register writes performed at EL2 into the EL2 MMU
  (SCTLR_EL1/TTBR0_EL1 writes perturb the EL2 translation), so programming the
  EL1 bank from EL2 breaks the running EL2 code and/or leaves the real EL1 bank
  unprogrammed. The VBAR-continuation structure (mirroring `enter_in_el1`)
  recovers the transition fault but the descend still cannot be made reliable
  under TCG.

Conclusion: under QEMU TCG, **any EL2->EL1 transition in firmware is
unreliable** because EL1 system-register writes from EL2 are not faithfully
handled. The firmware must therefore hand the bootloader off at **EL1** from
the start, so Limine runs at EL1, uses the proven `enter_in_el1`, and enters the
kernel at EL1.

### Firmware work

Build the SbsaQemu firmware (TF-A + edk2) configured to hand BL33 (UEFI/Limine)
off at EL1 rather than EL2:

- TF-A for the sbsa platform, with the non-secure BL33 entry point at EL1.
- edk2 `QemuSbsa` platform.
- Combined into the sbsa-ref flash images (`SBSA_FLASH0.fd`/`SBSA_FLASH1.fd`).

On real hardware or KVM the upstream Limine fix remains the correct long-term
answer; the EL1-handoff firmware is the TCG-compatible path.

## Tooling notes

- Firmware: prebuilt SbsaQemu UEFI (`SBSA_FLASH0.fd`/`SBSA_FLASH1.fd`, truncated
  to 256 MiB) from the `r1mikey/edk2-qemu-sbsa-bins` repo.
- Boot image: FAT32 ESP with `EFI/BOOT/BOOTAA64.EFI` (Limine), `/catten`
  (kernel), `/limine.conf`, attached as an NVMe (`-device nvme,drive=nvme0`).
- UEFI driven via a serial FIFO (`-serial pipe:`); keystrokes navigate
  Front Page -> Boot Manager -> UEFI Shell -> `fs0:` -> `\EFI\BOOT\BOOTAA64.EFI`.
- `sbsa-ref` uses `-pflash` for the two flash images; `-cpu neoverse-n1`
  (or `max`); RAM at 1 TiB (`0x10000000000`).
- Limine source: `git clone --depth 1 https://github.com/limine-bootloader/limine`
  then `./bootstrap`.
