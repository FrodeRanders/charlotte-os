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

### Progress

**TF-A EL1 handoff works.** The BL33 entry EL is chosen in
`plat/qemu/common/qemu_bl2_setup.c` (`qemu_get_spsr_for_bl33_entry`): it was
`el_implemented(2) ? MODE_EL2 : MODE_EL1`, changed to unconditionally
`MODE_EL1`. TF-A for `qemu_sbsa` is built with `aarch64-elf-gcc`
(`CROSS_COMPILE=aarch64-elf- aarch64-oc=aarch64-elf-objcopy`), the FIP is
packed with `tools/fiptool/fiptool create --tb-fw bl2.bin --soc-fw bl31.bin`
(host-fixtool needs `-D_UUID_T -D_DARWIN_C_SOURCE` and
`OPENSSL_DIR=/opt/homebrew/opt/openssl@3` on macOS), and `SBSA_FLASH0.fd` is
assembled as the original BL1 (0x0) followed by the new FIP at `0x12000`
(`dd bs=1 seek=73728`).

With the rebuilt BL2/BL31 (original BL1 kept — the freshly built BL1 panics),
the chain reaches: BL1 -> BL2 -> BL31 -> **UEFI firmware** (the r1mikey
`SBSA_FLASH1.fd`). The EL1 handoff is confirmed.

**Remaining: the prebuilt edk2 UEFI hangs at EL1.** The r1mikey
`SBSA_FLASH1.fd` was built for EL2; under the EL1 handoff it prints the UEFI
banner then stalls before the front page. edk2 QemuSbsa must be built for an
EL1 handoff (or the UEFI must tolerate EL1) to proceed to Limine and the
kernel.

### edk2 build status

The edk2 QemuSbsa build is in progress but currently blocked on the edk2 build
tool itself on macOS:

- edk2 + edk2-platforms + edk2-non-osi cloned (latest), BaseTools compiled,
  submodules updated, `GCC5_AARCH64_PREFIX=aarch64-elf-`.
- The `build` tool fails silently (`- Failed -`, 0 s) during `Build.__init__`;
  invoking `Build(...)` directly reveals:
  `WorkspaceDatabase.CreateBuildObject: 'str' object has no attribute 'Type'`
  in `GetToolChainAndFamilyFromDsc` — a version mismatch between edk2 master
  and edk2-platforms master. Pinning edk2 to a matching stable release and/or
  resolving the workspace-path handling is the next step.
- The resulting `SBSA_FLASH0.fd`/`SBSA_FLASH1.fd` would then contain the EL1
  TF-A (already built) plus an EL1-capable UEFI.

### Milestone: the full boot chain works

**The entire chain now boots under QEMU TCG:**

```
TF-A (BL1 -> BL2 -> BL31, EL1 handoff)
  -> edk2 QemuSbsa UEFI (built for EL1)
  -> Limine BOOTAA64.EFI
  -> Catten kernel
  -> heap, name service, observability, ACPI parsing (XSDT, 8 tables)
```

Key fixes that unblocked the UEFI at EL1:

- **`SIP_SVC_GET_CPU_TOPOLOGY` (SMC 202)**: the current edk2 QemuSbsa
  `HardwareInfoLib` requires it and TF-A v2.11 does not implement it, so the
  UEFI looped in `ResetShutdown()`. Implemented in
  `plat/qemu/qemu_sbsa/sbsa_sip_svc.c` (read `/cpus/topology` from the DTB,
  return sockets/clusters/cores/threads).
- **edk2 build tool**: `-p` leaves `PlatformFile` as a plain string which the
  workspace database cannot inspect (`'str' object has no attribute 'Type'`).
  Patched `LoadConfiguration` to convert it to a `PathClass`. Recent edk2 also
  renamed the toolchain `GCC5` -> `GCC`, and the QemuSbsa build needs `iasl`
  (`brew install acpica`), the BaseTools wrappers on `PATH`, and a workspace
  where `edk2-platforms`/`edk2-non-osi` are real subdirectories.

**Remaining: a kernel-side ACPI read fault.** After ACPI parsing begins, the
kernel takes a data abort at `FAR = 0xffff0000_08000c10` (physical `0x8000c10`,
a low address not covered by the higher-half direct map). The kernel's ACPI
discovery reads tables through the HHDM, but at least one ACPI structure lives
below the HHDM-mapped region on sbsa-ref. Fixing the kernel's handling of
low-memory ACPI tables is the next step (see the discovery code in
`environment/acpi/sdt/discovery.rs` and the ACPI table map).

A reproducible build script for the firmware is at
`scripts/build-sbsa-firmware.sh`.

### Milestone: sbsa-ref boots and runs the self-test suite

**The kernel boots on sbsa-ref and passes most of the self-test suite.** The
ACPI GIC discovery bug was that the Multiple APIC Description Table's signature
is **`"APIC"`**, not `"MADT"`; searching for `b"APIC"` resolves the real GIC
bases (`0x40060000`/`0x40080000`) and the kernel proceeds through GIC init into
the tests.

Passing on sbsa-ref: EL0, EL0 IPC (endpoint/blocking/cross-AS/memory
copy/move/cancel), device (MMIO + SPI), cq-wait, async, sitas, service, uart
(console driver via discovered PL011 base/IRQ from the SPCR), and raft (elects
a leader). The boot completes (boot-done marker).

**Status: the full self-test suite passes on sbsa-ref (18/18), matching
virt/TCG.** The last gap was the object store: its mount allocates a bitmap
plus a directory-entry vector (~96 KiB for a 128 MiB disk) while formatting,
which exceeded the 52 KiB per-domain EL0 heap — the store OOM'd after the two
superblock reads and aborted. The heap is now 256 KiB (`HEAP_SIZE = 0x40000`)
and relocated to `0x300000` (above the services' ELF load segments at `0x20000`,
below the status page at `0x7f0000`) so the larger arena doesn't collide with
the image; the sitas self-test's own heap mapping uses the shared constants.
With that, the NVMe storage stack and the persistent Raft recovery pass on
sbsa-ref alongside every other test.

### GIC security: the SPI/LPI delivery blocker (root-caused Aug 2026)

The sbsa-ref GIC has `has-security-extensions=true`, and this boot chain never
runs any secure world (everything is Non-secure EL1 because QEMU TCG's EL2 is
unreliable). With `GICD_CTLR.DS=0` QEMU enforces the security model against the
Non-secure kernel in three ways that silently break device interrupt delivery:

1. **Non-secure writes to `GICD_IGROUPR` are dropped.** The distributor's
   `GICD_IGROUPR` write handler ignores NS accesses while `DS=0`, so the
   kernel's `enable_spi` can never move an SPI into Group 1 NS. Every SPI
   stays Group 0.
2. **Group 0 SPIs are masked from the pending scan.** `gicd_int_pending()`
   only includes an SPI whose group is enabled in `GICD_CTLR`; Group 0 SPIs
   require `EnableGrp0` (bit 0), which the NS kernel is also forbidden from
   setting (the NS `GICD_CTLR` write mask is just bit 1). So a Group 0 SPI is
   never eligible for signalling.
3. **LPIs are reported as Group 0.** `update_for_one_lpi()` sets
   `hpp->grp = ds ? G1NS : G0`. With `DS=0` a pending LPI is signalled as FIQ
   (the kernel's FIQ vector does not ack it), so it stays pending forever and,
   via the hpplpi cache, outranks/masks every other interrupt on that CPU.

 **Firmware fix (TF-A v2.11 + `plat_qemu_gic_init` setting `GICD_CTLR.DS=1` at
EL3):** with `DS=1` the NS kernel's `GICD_IGROUPR` writes take effect (its SPIs
become Group 1 NS and are deliverable as IRQ), and QEMU tags LPIs Group 1 NS so
they are acked/EOI'd normally instead of wedging the CPU. The EL3 write is the
only way to set `DS` (the NS write mask forbids it).

**Reproducing the firmware (version control):** the patched third-party
deltas are *not* vendored (the sources stay upstream); each is tracked as a
patch file under `patches/` and applied by `scripts/build-sbsa-firmware.sh`
(via a single `apply_patch` helper that runs `git apply` on the checkout,
guarded by a marker grep so re-runs are idempotent):

- `patches/tf-a/0001-sbsa-gic-disable-security-ds.patch` — the `DS=1` change
  to `plat/qemu/qemu_sbsa/sbsa_gic.c`;
- `patches/tf-a/0002-sbsa-bl33-entry-el1.patch` — hand BL33 (UEFI/Limine) off
  at EL1 (`plat/qemu/common/qemu_bl2_setup.c`);
- `patches/tf-a/0003-sbsa-sip-smc-cpu-topology.patch` —
  `SIP_SVC_GET_CPU_TOPOLOGY` (SMC 202), missing from TF-A v2.11 and required by
  the edk2 QemuSbsa HardwareInfoLib (`sbsa_sip_svc.c`);
- `patches/edk2/0001-build-py-pathclass-p.patch` — convert a `-p`-given
  `PlatformFile` string to a `PathClass` in edk2's `build.py`.

The build script pins the TF-A clone to the **v2.11** tag (the images are
built from v2.11; the prebuilt upstream BL1 is v2.11.0-774, and later TF-A
moved the SRAM layout so a v2.11 BL1 cannot load a master BL2); edk2 /
edk2-platforms / edk2-non-osi are cloned from their default branches, matching
the original build. The resulting `SBSA_FLASH0.fd` places the original BL1 at
`0x0` and the new FIP (BL2 + BL31, `--tb-fw`/`--soc-fw`) at `0x12000`.

Build tooling notes (macOS): TF-A's v2.11 makefiles need GNU make 4.x
(`gmake`) and GNU sed (`gsed`) on `PATH` — the system make 3.81 / BSD sed
fail on `make_helpers/toolchain.mk` and the version-detection helpers — and
`aarch64-elf-gcc`. If `libc.a`/`libfdt.a` come out empty after a failed build,
`aarch64-elf-ar cr build/qemu_sbsa/release/lib/libc.a build/.../libc/*.o` (and
likewise for `libfdt`) then re-run. The current image is saved at
`target/firmware/SBSA_FLASH0-ds1.fd` (gitignored under `target/`).

**Kernel fix (committed):** QEMU re-pends the per-LP timer PPI (INTID 27)
almost continuously, and the distributor caches one "highest priority pending
interrupt" per CPU. With both the timer PPI and a device SPI at the same
priority (`0xa0`), the cached hppi keeps the lower INTID (the timer), so
`SPI 33` (the PL011's IRQ) was permanently outranked. `enable_spi` now gives
device SPIs priority `0x50`, strictly above the timer PPI, so a pending SPI
always wins the cached-hppi tie. This makes the device and uart self-tests pass
reliably on sbsa-ref.

### NVMe LPI (MSI) delivery: the `intid >= 1020` spurious bug

The irq_dispatcher's spurious check `if intid >= 1020 { return; }` treated
every INTID 1020+ as a spurious interrupt — including the valid LPI `8192`
delivered by the ITS. So the NVMe driver's MSI-X completion was acknowledged
and then silently dropped; the driver polled instead of waking on its CQ.
Restricted the check to `1020..=1023`.

Supporting fixes that make the LPI delivery reliable on QEMU:
- clear the Group 0/1 active-priority registers at GIC init (the UEFI
  firmware can leave an acknowledged-but-never-EOI'd NVMe MSI active, wedging
  the running priority);
- QEMU reports an enabled LPI's priority as `byte & 0xfc` (always >= 0x80,
  because the config byte's bit 7 is both the enable and the top priority
  bit), so the timer PPI must sit below that: the per-LP timer now uses
  `TIMER_PRIORITY` `0xf0` (below `SPI_PRIORITY` 0x50 and every enabled LPI)
  so a busy timer can't win the cached-hppi preemption tie against a device
  interrupt;
- `LPI_PRIORITY` raised so the property-table byte stays enabled (bit 7) while
  keeping an effective priority (0x90) above the timer.

With these, the NVMe self-test runs its full 12 KiB PRP-list round trip, the
ITS delivers MSI-X completion interrupts to the driver's CQ, and the SMMU
domain completes the transfer without translation faults.

These are the same "hardcoded platform geometry" class of issue as the original
GIC/PL011 constants, and are the natural next bring-up items.

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
