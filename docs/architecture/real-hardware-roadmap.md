# Real-Hardware Roadmap

> Goal context: take CharlotteOS / Catten from its emulated bring-up targets
> (QEMU `virt` and QEMU `sbsa-ref`) to booting on real ARM server hardware.
> `sbsa-ref` models ARM **SystemReady SR** (server-class) platforms — UEFI +
> TF-A firmware, ACPI, GICv3 with an ITS, an SMMU, PCIe with NVMe — the same
> architecture as real servers (Ampere Altra, AWS Graviton, Fujitsu A64FX).

## TL;DR

- The emulated, server-shaped path is fully green: **18/18** self-tests on both
  QEMU `virt` and QEMU `sbsa-ref` (`docs/platforms/sbsa-ref.md`).
- The architecture that transfers directly to real hardware is validated: UEFI
  + ACPI boot, GICv3/ITS **LPI/MSI delivery**, **SMMU protected DMA**, PCIe
  enumeration, an **EL0 NVMe driver**, an object store, and durable Raft.
- The remaining work to real silicon is dominated by three things:
  1. the kernel runs at **EL1** and has no **EL2** layer (real server firmware
     boots an OS at EL2),
  2. two interrupt/timer behaviors were tuned to **QEMU's model** (the firmware
     `GICD_CTLR.DS` patch and the timer-priority tuning), and
  3. nothing has booted on **real silicon** — controller and ACPI-table quirks
     are unvalidated.

## What the sbsa-ref work validated (portable)

These subsystems are implemented against the architecture (ACPI, GICv3, ITS,
SMMUv3, PCIe, NVMe), not against QEMU's quirks, so they are the transferable
core:

- **Boot chain**: TF-A → edk2 UEFI → Limine → kernel at EL1, with ACPI table
  parsing (XSDT, FADT, MADT, SPCR, GTDT).
- **GICv3**: distributor + redistributors, Group 1 NS configuration, and the
  **ITS** command queue (MAPD/MAPC/MAPTI/INVALL) delivering **LPIs** as MSI-X
  completions to an EL0 driver.
- **SMMU**: a protected-DMA domain mapping an NVMe requester stream, with a
  verified zero-fault transfer.
- **PCIe / ECAM**: device topology discovery and MSI-X programming.
- **EL0 driver model**: capability-granted MMIO + interrupt + DMA to userspace
  drivers (`device`, `el0_uart`, `el0_nvme`).
- **Storage**: a block driver, an object store, and NVMe-backed Raft recovery.
- **Correctness fixes that are general, not QEMU-specific**:
  - ACPI MADT signature is `"APIC"`, not `"MADT"`.
  - The IRQ dispatcher treated every INTID >= 1020 as spurious, dropping valid
    LPIs; the check is now `1020..=1023`.

## Current status

- `scripts/run-aarch64.sh` boots `virt` (default) and `--sbsa-ref`; both
  complete `SELFTEST COMPLETE: passed=18 failed=0 pending=0`.
- The kernel is entered at EL1 and contains an **EL2 → EL1 descent** in `_start`
  (`crates/catten/src/main.rs`: reads `CurrentEL`, clears `HCR_EL2.E2H`, erets
  to EL1h), so it tolerates a bootloader that enters at EL2 — it just does not
  *use* EL2.
- Firmware is reproducible from `scripts/build-sbsa-firmware.sh` with the
  tracked patches under `patches/` (`docs/platforms/sbsa-ref.md` "Reproducing the
  firmware").

## What is still emulation-shaped (the gaps)

### 1. No EL2 / hypervisor layer
The whole boot chain is forced to EL1 because QEMU TCG's EL2/VHE is
unreliable; TF-A is patched to hand BL33 off at EL1
(`patches/tf-a/0002-sbsa-bl33-entry-el1.patch`). Real SystemReady firmware
enters the OS at EL2. CharlotteOS has no VHE or hypervisor mode — it descends
to EL1 and ignores EL2.

**Impact**: cannot boot on real server firmware as-is; no virtualization or
EL2-resident features.

### 2. QEMU-tuned interrupt/timer behavior
- `patches/tf-a/0001-sbsa-gic-disable-security-ds.patch` sets `GICD_CTLR.DS=1`
  in BL31 because QEMU drops Non-secure `GICD_IGROUPR` writes and tags LPIs
  Group 0 with `DS=0`. Real SystemReady firmware configures the GIC correctly,
  so this patch should be unnecessary there (and the kernel must be robust
  either way).
- The SPI/LPI/timer priorities (`SPI_PRIORITY`, `TIMER_PRIORITY`,
  `LPI_PRIORITY` in `crates/catten/src/cpu/isa/aarch64/interrupts/gic/`) were
  tuned because QEMU re-pends the timer PPI almost continuously, letting the
  timer win the cached-hppi tie. Real timer PPIs do not behave that way, so
  these need re-validation on silicon (they are harmless defaults if kept).

### 3. No real-silicon validation
- PCIe/NVMe/SMMUv3/ITS have vendor quirks and different BARs, MSI-X table
  layouts, interrupt routes, and ACPI table content than QEMU's models.
- The UART is PL011 (sbsa-ref) or the virt PL011; other platforms use different
  consoles.
- The virtio-net path is validated against QEMU's device, not a real NIC.
- Port-wide caveats from `docs/platforms/aarch64.md`: framebuffer/display is
  unreliable under emulation; KASLR and similar are untested.

## Roadmap

### Phase 0 — keep the emulated baselines green (ongoing)
- Maintain `SELFTEST COMPLETE: passed=18 failed=0 pending=0` on `virt` and
  `--sbsa-ref` after every change (this is the merge gate used during the
  bring-up).

### Phase 1 — EL2 readiness (prerequisite for real server firmware)
- Decide the EL2 story: **VHE** (reuse the EL2 register bank at EL1 semantics)
  or a minimal hypervisor.
- Make the boot path accept an EL2 entry without the firmware EL1-handoff
  patch (i.e., the kernel, not the firmware, performs the EL2 → EL1 transition,
  as it already can via the `_start` descent).
- Add a TCG/HVF EL2 test mode so this is exercised before real hardware.

#### Why EL2 — what it constructively buys

EL2 is not merely "where real firmware enters us"; it is an *isolation
authority* that CharlotteOS's capability model is unusually well-placed to
use. Two models:

- **Separate hypervisor at EL2** (kernel stays at EL1, a thin hypervisor
  isolates it), or
- **VHE — the kernel itself at EL2** with stage-2 isolating every EL0 domain.
  This is the cheaper option and the likely default for a single kernel.

Concrete uses, strongest first:

1. **Capability-root at EL2 (headline).** If the authoritative capability
   table / minting authority lives at EL2, even a compromised EL1 kernel
   cannot forge or mint capabilities — it can only use what it was granted.
   This gives a *compromise-resistant microkernel*: the isolation already
   granted to EL0 drivers is extended to the kernel itself. CharlotteOS's
   capability abstraction already provides the right primitive; EL2 is where
   its root belongs. Design sketch: `docs/architecture/el2-capability-root.md`.
2. **Stage-2 defense-in-depth for EL0 driver domains.** A second, unforgeable
   "you may only touch what you were granted" layer even if a domain's stage-1
   page table is corrupted. This is the CPU-side twin of the SMMU (which
   polices DMA): the two compose into complete capability enforcement.
3. **Virtual interrupt mediation (vGIC).** Deliver *virtual* interrupts per
   domain, keeping the real GIC and interrupt-grant routing under EL2.
4. **Per-domain time (virtual counter).** Separate virtual timers/counters for
   sandboxing, accounting, and cross-domain timing-side-channel protection.
5. **Trapping for per-domain policy.** Trap-and-emulate CPU features (SIMD/SVE,
   debug/PMU, system registers) so "this domain may use the PMU" becomes an
   EL2-enforced grant, matching the device-grant model.
6. **Trusted boot / attestation root.** EL2 holds the measurement root and
   verifies EL1 before releasing kernel secrets or bootstrapping the
   capability root.
7. **Functional:** guest-OS hosting (a Linux VM for compatibility), domain
   checkpoint/migration via stage-2, virtual devices, and an EL1 "kernel
   firewall" for audit.

Cost/considerations: a separate hypervisor adds a second privileged codebase;
**VHE keeps it to one kernel at EL2** and is the pragmatic first step.
Capability-rooting at EL2 is a genuine redesign of *where* authority lives —
high value, high effort. Whatever is chosen must keep the EL1 path working on
the emulated targets, which remain the dev baseline.

### Phase 2 — remove QEMU-shape assumptions
- Re-validate the GIC SPI/LPI/timer priorities against a platform whose timer
  PPI is not continuously re-pended; only keep tuning that is architecturally
  correct.
- Make the GIC security configuration driven by the platform's actual
  `GICD_CTLR.DS` / group configuration rather than a firmware patch: the kernel
  should detect and adapt (e.g., set Group 1 NS if NS `GICD_IGROUPR` writes are
  honored; otherwise use the platform's default).
- Generalize the console to non-PL011 UARTs (or keep PL011 as the primary).

### Phase 3 — first boot on real SystemReady SR hardware
- Target an accessible server-class board or a cloud AArch64 instance under KVM
  (KVM provides EL2 without real-metal provisioning).
- Validate: ACPI table content, MADT/ITS location, SPCR console, GIC routing,
  the scheduler/timer under real counter behavior.
- Boot the full suite and capture the first `SELFTEST COMPLETE` on the target.

### Phase 4 — real devices
- NVMe on a real drive (end-to-end write/flush/read with MSI-X and SMMU).
- A real NIC (replace or complement the virtio-net test) and the network
  stack (discovery/DNS/TCP/IP).
- Exercise SMMUv3 features beyond the single-stream QEMU configuration.
- Stress: KASLR, long-running scheduler soak, power/CPU hotplug if the target
  exposes it.

## Validation criteria

- **Gate (all phases)**: `SELFTEST COMPLETE: passed=18 failed=0 pending=0` on
  the target, matching the emulated baseline.
- **Phase 1**: the kernel boots when the bootloader enters at EL2 with no
  firmware EL1-handoff patch.
- **Phase 2**: the GIC security and priority code paths pass on a
  non-QEMU-tuned platform, ideally with the `patches/tf-a/0001...` DS patch
  not applied.
- **Phase 3/4**: ACPI discovery, ITS LPI delivery, SMMU DMA and a real-NVMe
  round trip verified on silicon; the object store survives a power-loss/
  restart (durable Raft recovery).

## Non-server targets (Raspberry Pi)

A complementary, much cheaper real-silicon track. Not a SystemReady SR
rehearsal: both the Pi 4 (BCM2711, Cortex-A72) and the Pi 5 (BCM2712,
Cortex-A76) use a **GIC-400 in GICv2 mode** — verified from the BCM2712
device tree (`interrupt-controller@7fff9000` = `arm,gic-400`, node named
`gicv2`; GICD/GICC/GICH/GICV only, no redistributor, no ITS). There are
**no LPIs, no ITS, and no SMMU** anywhere on the Pi. The server-class crown
jewels (GICv3+ITS LPI delivery, SMMU-protected DMA, MSI-X-via-ITS) therefore
have **no target** on a Pi. What a Pi does provide, cheaply:

- **Real EL2/VHE silicon** — exactly the validation QEMU TCG cannot give.
  Pi 5 (A76, ARMv8.2) has VHE; Pi 4 (A72, ARMv8.0) has EL2 but no VHE. This
  directly de-risks Phase 1 (EL2 readiness) and the EL2 capability-root
  design (`docs/architecture/el2-capability-root.md`).
- **Real timers/GIC/MMIO/timing** — to re-validate the QEMU-shape
  assumptions (Phase 2) on real silicon.
- **A second interrupt-controller family** — GICv2 exercises the
  `ExternalInterruptControllerIfce` abstraction for the first time; one
  GICv2 backend covers both Pi generations.
- **Real storage/NVMe** — the Pi 5 M.2 HAT+ slot is PCIe with working
  MSI-X.
- **Real USB / GPIO / Ethernet / SD** — driver coverage beyond virtio.

### What transfers

- Boot via the official Pi UEFI + Limine (the existing chain). The Pi 4
  UEFI provides ACPI tables (reuse the MADT/SPCR/GTDT discovery); the Pi 5
  UEFI's ACPI support needs verifying — the kernel's DT parsing path is
  still `todo!()`.
- Console: PL011 (Pi 5 SoC PL011 at `0x107d001000`, SPI 121; Pi 4 at
  `0xfe201000`).
- Generic timer, scheduler, IPC, EL0 model, object store / Raft (given a
  block device).
- PSCI (`method = "smc"`) for secondary cores on Pi 5.

### Required work

1. **GICv2 backend (GIC-400)** — distributor init (GICD only, no GICR),
   SPI targeting via `GICD_ITARGETSR`/`ICFGR` instead of v3 `IROUTER`, no
   LPI/PEND tables, and the MMIO GICC CPU interface if the system-register
   (SRE) path is unavailable. The `ICC_*_EL1` CPU-interface code largely
   carries over.
2. **MSI path** — the Pi 4 PCIe has no MSI at all (INTx/polling only; NVMe
   not viable). The Pi 5 PCIe MSI-X works via the vendor **MIP controller**
   (`brcm,bcm2712-mip-intc`, `msi-base-spi = <128>`), which translates MSI
   writes into plain SPIs — a new transport, distinct from the sbsa-ref ITS
   path.
3. **No SMMU** — add a "no SMMU, identity DMA" fallback. Watch the Pi 5
   `dma-ranges`, which restrict low-speed-peripheral DMA to the lower 1 GB
   ("emulate a contiguous 30-bit range").
4. **New drivers** — SDHCI (`sdio1`, SPI 273) for SD; a USB host stack
   (missing today); a real NIC (the Pi 4 ethernet is USB-attached, so it
   needs USB first; the Pi 5 ethernet lives on the RP1 southbridge).

### Placement

A Pi 5 (ideally) is a **Phase 1/2 enabler**: the cheapest real-silicon EL2/
VHE test bed and a way to re-validate the emulation-shaped assumptions — not
a substitute for the server track, which still needs GICv3/ITS/LPI/SMMU
validated on a real SystemReady SR platform or under KVM.

## References

- `docs/architecture/el2-capability-root.md` — the EL2 capability-root design sketch (the
  headline security use for the EL2 layer).
- `docs/platforms/sbsa-ref.md` — the emulated bring-up, the GIC/LPI/heap fixes,
  and the reproducible firmware.
- `docs/platforms/aarch64.md` — the earlier `virt` port status and port-wide
  caveats.
- `patches/` — the tracked third-party firmware deltas
  (`tf-a/0001` GIC DS, `tf-a/0002` EL1 handoff, `tf-a/0003` SMC 202,
  `edk2/0001` build-tool).
- `scripts/run-aarch64.sh` (`--sbsa-ref`) and `scripts/build-sbsa-firmware.sh`.
- Kernel entry / EL descent: `crates/catten/src/main.rs`.
- GIC/ITS/LPI: `crates/catten/src/cpu/isa/aarch64/interrupts/gic/`.
- Pi facts (GICv2, MIP MSI, SDHCI, PL011, DMA ranges): the BCM2712 device
  tree `arch/arm64/boot/dts/broadcom/bcm2712.dtsi` in the Raspberry Pi Linux
  tree.
